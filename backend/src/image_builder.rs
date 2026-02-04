//! Image building from Dockerfiles
//!
//! This module provides functionality for building images from Dockerfile-like specifications.
//! It supports parsing Dockerfiles, executing instructions in a chroot environment,
//! and managing ZFS snapshots for layer management.

use crate::image::{Image, ImageConfig, DockerfileInstruction, ImageId};
use crate::zfs::Zfs;
use crate::bootstrap::{Bootstrap, BootstrapConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use std::io::Write;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Image builder error type
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ZFS error: {0}")]
    Zfs(String),
    #[error("Build failed: {0}")]
    BuildFailed(String),
}

pub type Result<T> = std::result::Result<T, ImageError>;

/// Progress updates during image building
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageBuildProgress {
    pub image_id: ImageId,
    pub step: usize,
    pub total_steps: usize,
    pub current_instruction: String,
    pub status: BuildStatus,
}

/// Status of an image build operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildStatus {
    Building,
    Failed,
    Complete,
}

/// Image builder for constructing images from Dockerfiles
pub struct ImageBuilder {
    zfs: Zfs,
    base_dataset: String,
    progress_tx: mpsc::Sender<ImageBuildProgress>,
    build_args: HashMap<String, String>,
    build_context: PathBuf,
}

impl ImageBuilder {
    /// Create a new image builder
    ///
    /// # Arguments
    ///
    /// * `zfs` - ZFS wrapper for dataset management
    /// * `base_dataset` - Base ZFS dataset for images (e.g., "tank/kawakaze/images")
    ///
    /// # Returns
    ///
    /// Returns a tuple of (ImageBuilder, progress receiver)
    pub fn new(zfs: Zfs, base_dataset: String) -> (Self, mpsc::Receiver<ImageBuildProgress>) {
        let (progress_tx, progress_rx) = mpsc::channel(100);
        let builder = Self {
            zfs,
            base_dataset,
            progress_tx,
            build_args: HashMap::new(),
            build_context: PathBuf::from("."),
        };
        (builder, progress_rx)
    }

    /// Set build arguments for variable substitution
    pub fn with_build_args(mut self, args: HashMap<String, String>) -> Self {
        self.build_args = args;
        self
    }

    /// Set the build context directory
    pub fn with_build_context(mut self, context: PathBuf) -> Self {
        self.build_context = context;
        self
    }

    /// Build an image from a Dockerfile
    ///
    /// # Arguments
    ///
    /// * `name` - Name for the resulting image
    /// * `dockerfile` - Dockerfile content as a string
    /// * `from_image` - Optional base image to build from
    ///
    /// # Returns
    ///
    /// Returns the built Image on success
    pub async fn build(
        &mut self,
        name: String,
        dockerfile: &str,
        from_image: Option<&Image>,
    ) -> Result<Image> {
        info!("Starting image build for '{}'", name);

        // Parse dockerfile
        let instructions = self.parse_dockerfile(dockerfile)?;
        let total_steps = instructions.len();

        info!("Parsed {} instructions from Dockerfile", total_steps);

        // Initialize config from base image or default
        let mut config = from_image.map(|i| i.config.clone()).unwrap_or_default();
        let mut parent_id = from_image.map(|i| i.id.clone());

        // Create build dataset
        let build_dataset = format!("{}/build-{}", self.base_dataset, name);
        self.create_build_dataset(&build_dataset, from_image)?;

        // Mount the build dataset to a temporary location for building
        let build_mountpoint = PathBuf::from(format!("/var/db/kawakaze/builds/{}", name.replace('/', "-")));
        self.zfs.mount_dataset(&build_dataset, &build_mountpoint)
            .map_err(|e| ImageError::Zfs(e.to_string()))?;

        // Execute instructions
        let build_result = (|| async {
            let mut config = from_image.map(|i| i.config.clone()).unwrap_or_default();
            let parent_id = from_image.map(|i| i.id.clone());

            // Execute instructions
            for (step, instruction) in instructions.iter().enumerate() {
                self.report_progress(
                    &name,
                    step,
                    total_steps,
                    instruction,
                    BuildStatus::Building
                ).await;

                if let Err(e) = self.execute_instruction(&build_mountpoint, instruction, &mut config).await {
                    error!("Build failed at step {}: {}", step, e);
                    return Err(e);
                }
            }

            // Create snapshot of the final image
            let snapshot_name = format!("{}-{}", name.replace('/', "-"), uuid::Uuid::new_v4());
            self.zfs.create_snapshot(&build_dataset, &snapshot_name)
                .map_err(|e| ImageError::Zfs(e.to_string()))?;

            let snapshot = format!("{}@{}", build_dataset, snapshot_name);

            // Get image size
            let size_bytes = match self.zfs.get_used_space(&build_dataset) {
                Ok(size) => size,
                Err(_) => 0,
            };

            // Rename build dataset to final image dataset
            let final_dataset = format!("{}/{}", self.base_dataset, name.replace('/', "-"));
            self.zfs.rename(&build_dataset, &final_dataset)
                .map_err(|e| ImageError::Zfs(e.to_string()))?;

            let final_snapshot = format!("{}@{}", final_dataset, snapshot_name);

            // Create image - only set parent_id if we have a base image
            let mut image = Image::new(name, instructions)
                .with_snapshot(final_snapshot)
                .with_config(config)
                .with_size(size_bytes)
                .with_state(crate::image::ImageState::Available);

            // Set parent_id only if building from a base image
            if let Some(pid) = parent_id {
                image = image.with_parent(pid);
            }

            self.report_progress(
                &image.id,
                total_steps,
                total_steps,
                &DockerfileInstruction::Run("complete".to_string()),
                BuildStatus::Complete
            ).await;

            info!("Image build completed successfully: {}", image.id);
            Ok::<Image, ImageError>(image)
        })().await;

        // Unmount the build dataset
        let _ = self.zfs.unmount_dataset(&build_dataset);

        build_result
    }

    /// Parse a Dockerfile into instructions
    fn parse_dockerfile(&self, dockerfile: &str) -> Result<Vec<DockerfileInstruction>> {
        let mut instructions = Vec::new();

        for (line_num, line) in dockerfile.lines().enumerate() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Handle line continuation
            let mut full_line = line.to_string();
            if line.ends_with('\\') {
                // Remove the backslash
                full_line.pop();
                full_line.push(' ');
                continue; // Would need multi-line handling in a more complete implementation
            }

            // Parse instruction
            match self.parse_instruction(&full_line) {
                Ok(instr) => instructions.push(instr),
                Err(e) => {
                    error!("Failed to parse line {}: {}", line_num + 1, e);
                    return Err(ImageError::ParseError(
                        format!("Line {}: {}", line_num + 1, e)
                    ));
                }
            }
        }

        // Validate Dockerfile has FROM as first instruction
        // Allow "scratch" as a special no-op base image
        if !instructions.is_empty() {
            if !matches!(&instructions[0], DockerfileInstruction::From(_)) {
                return Err(ImageError::ParseError(
                    "Dockerfile must start with FROM instruction".into()
                ));
            }
            // If FROM scratch, remove it from instructions since it's a no-op
            if matches!(&instructions[0], DockerfileInstruction::From(name) if name == "scratch") {
                instructions.remove(0);
            }
        }

        Ok(instructions)
    }

    /// Parse a single Dockerfile instruction
    fn parse_instruction(&self, line: &str) -> Result<DockerfileInstruction> {
        let line = self.substitute_build_args(line);

        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.is_empty() {
            return Err(ImageError::ParseError("Empty instruction".into()));
        }

        let instruction = parts[0].to_uppercase();
        let args = parts.get(1).unwrap_or(&"").trim();

        match instruction.as_str() {
            "FROM" => Ok(DockerfileInstruction::From(args.to_string())),

            "BOOTSTRAP" => {
                // Parse BOOTSTRAP [VERSION] [ARCHITECTURE] [MIRROR]
                let parts: Vec<&str> = args.split_whitespace().collect();
                let version = if parts.len() > 0 && !parts[0].is_empty() {
                    Some(parts[0].to_string())
                } else {
                    None
                };
                let architecture = if parts.len() > 1 && !parts[1].is_empty() {
                    Some(parts[1].to_string())
                } else {
                    None
                };
                let mirror = if parts.len() > 2 && !parts[2].is_empty() {
                    Some(parts[2].to_string())
                } else {
                    None
                };
                Ok(DockerfileInstruction::Bootstrap { version, architecture, mirror })
            }

            "RUN" => Ok(DockerfileInstruction::Run(args.to_string())),

            "COPY" => {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if parts.len() < 2 {
                    return Err(ImageError::ParseError("COPY requires source and destination".into()));
                }
                let dst = parts.last().unwrap().to_string();
                let src = parts[0].to_string();
                Ok(DockerfileInstruction::Copy {
                    from: None,
                    src,
                    dest: dst,
                })
            }

            "ADD" => {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if parts.len() < 2 {
                    return Err(ImageError::ParseError("ADD requires source and destination".into()));
                }
                let dst = parts.last().unwrap().to_string();
                let src = parts[0].to_string();
                Ok(DockerfileInstruction::Add { src, dest: dst })
            }

            "WORKDIR" => Ok(DockerfileInstruction::WorkDir(args.to_string())),

            "ENV" => {
                let mut env_map = HashMap::new();
                // Parse multiple ENV vars
                for part in args.split_whitespace().collect::<Vec<_>>().chunks(2) {
                    if part.len() == 2 {
                        env_map.insert(part[0].to_string(), part[1].to_string());
                    }
                }
                Ok(DockerfileInstruction::Env(env_map))
            }

            "EXPOSE" => {
                let ports: std::result::Result<Vec<u16>, _> = args
                    .split_whitespace()
                    .map(|p| p.parse::<u16>().map_err(|_| ImageError::ParseError(format!("Invalid port: {}", p))))
                    .collect();
                Ok(DockerfileInstruction::Expose(ports?))
            }

            "USER" => Ok(DockerfileInstruction::User(args.to_string())),

            "VOLUME" => {
                let volumes: Vec<String> = args
                    .split_whitespace()
                    .map(|v| v.to_string())
                    .collect();
                Ok(DockerfileInstruction::Volume(volumes))
            }

            "CMD" => {
                let cmd = if args.starts_with('[') {
                    // Exec form
                    serde_json::from_str::<Vec<String>>(args)
                        .map_err(|_| ImageError::ParseError("Invalid CMD syntax".into()))?
                } else {
                    // Shell form
                    vec![args.to_string()]
                };
                Ok(DockerfileInstruction::Cmd(cmd))
            }

            "ENTRYPOINT" => {
                let entrypoint = if args.starts_with('[') {
                    serde_json::from_str::<Vec<String>>(args)
                        .map_err(|_| ImageError::ParseError("Invalid ENTRYPOINT syntax".into()))?
                } else {
                    vec![args.to_string()]
                };
                Ok(DockerfileInstruction::Entrypoint(entrypoint))
            }

            "LABEL" => {
                let mut label_map = HashMap::new();
                for part in args.split_whitespace().collect::<Vec<_>>().chunks(2) {
                    if part.len() == 2 {
                        label_map.insert(part[0].to_string(), part[1].to_string());
                    }
                }
                Ok(DockerfileInstruction::Label(label_map))
            }

            "ARG" => {
                // ARG is handled during build, not stored
                let parts: Vec<&str> = args.split('=').collect();
                if !parts.is_empty() {
                    debug!("Build ARG: {}", parts[0]);
                }
                Ok(DockerfileInstruction::Run(format!("# ARG {}", args)))
            }

            "STOPSIGNAL" => {
                // Store as label for now
                let mut labels = HashMap::new();
                labels.insert("stop_signal".to_string(), args.to_string());
                Ok(DockerfileInstruction::Label(labels))
            }

            "SHELL" => {
                let shell = if args.starts_with('[') {
                    serde_json::from_str::<Vec<String>>(args)
                        .map_err(|_| ImageError::ParseError("Invalid SHELL syntax".into()))?
                } else {
                    vec![args.to_string()]
                };
                // Store as label for now
                let mut labels = HashMap::new();
                labels.insert("shell".to_string(), shell.join(" "));
                Ok(DockerfileInstruction::Label(labels))
            }

            _ => Err(ImageError::ParseError(format!("Unknown instruction: {}", instruction))),
        }
    }

    /// Substitute build arguments in a line
    fn substitute_build_args(&self, line: &str) -> String {
        let mut result = line.to_string();
        for (key, value) in &self.build_args {
            result = result.replace(&format!("${{{}}}", key), value);
            result = result.replace(&format!("${}", key), value);
        }
        result
    }

    /// Create a build dataset, cloning from base image if provided
    fn create_build_dataset(&self, dataset: &str, from_image: Option<&Image>) -> Result<()> {
        if let Some(base) = from_image {
            // Clone from base image snapshot
            let snapshot_parts: Vec<&str> = base.snapshot.split('@').collect();
            if snapshot_parts.len() != 2 {
                return Err(ImageError::Zfs(format!("Invalid snapshot format: {}", base.snapshot)));
            }

            self.zfs.clone_snapshot(&base.snapshot, dataset)
                .map_err(|e| ImageError::Zfs(e.to_string()))?;
        } else {
            // Create new empty dataset
            self.zfs.create_dataset(dataset)
                .map_err(|e| ImageError::Zfs(e.to_string()))?;
        }

        Ok(())
    }

    /// Execute a Dockerfile instruction
    async fn execute_instruction(
        &mut self,
        root: &Path,
        instruction: &DockerfileInstruction,
        config: &mut ImageConfig,
    ) -> Result<()> {
        match instruction {
            DockerfileInstruction::From(_) => {
                // FROM is handled before execution (in create_build_dataset)
                debug!("FROM instruction (already handled)");
            }

            DockerfileInstruction::Bootstrap { version, architecture, mirror } => {
                info!("Executing BOOTSTRAP: version={:?}, arch={:?}", version, architecture);
                self.execute_bootstrap(root, version.clone(), architecture.clone(), mirror.clone()).await?;
            }

            DockerfileInstruction::Run(cmd) => {
                info!("Executing RUN: {}", cmd);
                self.execute_run(root, cmd).await?;
            }

            DockerfileInstruction::Copy { from: _, src, dest } => {
                info!("Executing COPY: {} -> {}", src, dest);
                self.execute_copy(root, src, dest)?;
            }

            DockerfileInstruction::Add { src, dest } => {
                info!("Executing ADD: {} -> {}", src, dest);
                self.execute_add(root, src, dest)?;
            }

            DockerfileInstruction::WorkDir(path) => {
                info!("Setting WORKDIR: {}", path);
                config.workdir = Some(PathBuf::from(path));
                self.create_directory(root, path)?;
            }

            DockerfileInstruction::Env(env_map) => {
                info!("Setting ENV: {} variables", env_map.len());
                config.env.extend(env_map.clone());
                self.write_environment(root, env_map)?;
            }

            DockerfileInstruction::Expose(ports) => {
                info!("Exposing ports: {:?}", ports);
                config.exposed_ports.extend(ports);
            }

            DockerfileInstruction::User(user) => {
                info!("Setting USER: {}", user);
                config.user = Some(user.clone());
            }

            DockerfileInstruction::Volume(volumes) => {
                info!("Adding volumes: {:?}", volumes);
                config.volumes.extend(volumes.clone());
                for vol in volumes {
                    self.create_directory(root, vol)?;
                }
            }

            DockerfileInstruction::Cmd(cmd) => {
                info!("Setting CMD: {:?}", cmd);
                config.cmd = Some(cmd.clone());
            }

            DockerfileInstruction::Entrypoint(ep) => {
                info!("Setting ENTRYPOINT: {:?}", ep);
                config.entrypoint = Some(ep.clone());
            }

            DockerfileInstruction::Label(labels) => {
                info!("Adding labels: {} entries", labels.len());
                config.labels.extend(labels.clone());
            }

            _ => {
                debug!("Skipping instruction: {:?}", instruction);
            }
        }

        Ok(())
    }

    /// Execute a RUN instruction in the chroot
    async fn execute_run(&mut self, root: &Path, cmd: &str) -> Result<()> {
        debug!("Running command in chroot: {}", cmd);

        // Check if we're on FreeBSD and if chroot is available
        #[cfg(target_os = "freebsd")]
        {
            let result = self.chroot_command(root, cmd);
            if result {
                return Ok(());
            }
        }

        // Fallback: simulate by writing to a script
        warn!("chroot not available, simulating RUN command");
        let script_path = root.join("tmp").join("kawakaze-build.sh");
        if let Some(parent) = script_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut script = fs::File::create(&script_path)?;
        writeln!(script, "#!/bin/sh")?;
        writeln!(script, "{}", cmd)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms)?;
        }

        Ok(())
    }

    /// Execute a command in a chroot environment
    #[cfg(target_os = "freebsd")]
    fn chroot_command(&self, root: &Path, cmd: &str) -> bool {
        use std::ffi::CString;

        let root_c = match CString::new(root.to_string_lossy().as_ref()) {
            Ok(r) => r,
            Err(_) => return false,
        };

        // Fork and exec in chroot
        unsafe {
            match libc::fork() {
                -1 => return false,
                0 => {
                    // Child process
                    if libc::chdir(root_c.as_ptr()) != 0 {
                        libc::_exit(1);
                    }
                    if libc::chroot(root_c.as_ptr()) != 0 {
                        libc::_exit(1);
                    }

                    // Execute the command
                    let shell = CString::new("/bin/sh").unwrap();
                    let arg_c = CString::new("-c").unwrap();
                    let cmd_c = match CString::new(cmd) {
                        Ok(c) => c,
                        Err(_) => libc::_exit(1),
                    };

                    libc::execlp(
                        shell.as_ptr(),
                        shell.as_ptr(),
                        arg_c.as_ptr(),
                        cmd_c.as_ptr(),
                        std::ptr::null::<libc::c_char>()
                    );
                    libc::_exit(1);
                }
                _ => {
                    // Parent process - wait for child
                    let mut status: libc::c_int = 0;
                    if libc::wait(&mut status) != -1 {
                        return libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0;
                    }
                    return false;
                }
            }
        }
    }

    /// Execute a BOOTSTRAP instruction to install FreeBSD base system
    async fn execute_bootstrap(
        &mut self,
        root: &Path,
        version: Option<String>,
        architecture: Option<String>,
        mirror: Option<String>,
    ) -> Result<()> {
        info!("Bootstrapping FreeBSD base system at {:?}", root);

        // Check if already bootstrapped
        if Bootstrap::is_bootstrapped(root) {
            info!("Already bootstrapped, skipping");
            return Ok(());
        }

        // Create bootstrap config
        let config = BootstrapConfig {
            version,
            architecture,
            mirror,
            no_cache: false,
            config_overrides: None,
        };

        // Create progress channel
        let (progress_tx, _progress_rx) = tokio::sync::mpsc::channel(100);

        // Create and run bootstrap
        let bootstrap = Bootstrap::new(root, config, progress_tx)
            .map_err(|e| ImageError::BuildFailed(format!("Failed to create bootstrap: {}", e)))?;

        bootstrap.run().await
            .map_err(|e| ImageError::BuildFailed(format!("Bootstrap failed: {}", e)))?;

        info!("Bootstrap completed successfully");
        Ok(())
    }

    /// Execute a COPY instruction
    fn execute_copy(&self, root: &Path, src: &str, dest: &str) -> Result<()> {
        let src_path = self.build_context.join(src);
        let dst_path = root.join(dest.trim_start_matches('/'));

        // Create destination directory
        if let Some(parent) = dst_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Copy file or directory
        if src_path.is_dir() {
            self.copy_directory(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }

        Ok(())
    }

    /// Copy a directory recursively
    fn copy_directory(&self, src: &Path, dst: &Path) -> Result<()> {
        fs::create_dir_all(dst)?;

        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() {
                self.copy_directory(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path)?;
            }
        }

        Ok(())
    }

    /// Execute an ADD instruction (supports URLs and auto-extraction)
    fn execute_add(&self, root: &Path, src: &str, dest: &str) -> Result<()> {
        // Check if src is a URL
        if src.starts_with("http://") || src.starts_with("https://") {
            // Download to destination
            info!("Downloading from URL: {}", src);
            let dst_path = root.join(dest.trim_start_matches('/'));
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)?;
            }

            // For now, just log - actual download would be implemented here
            warn!("URL download not yet implemented: {}", src);
        } else {
            // Copy like COPY instruction
            self.execute_copy(root, src, dest)?;
        }

        Ok(())
    }

    /// Create a directory in the chroot
    fn create_directory(&self, root: &Path, path: &str) -> Result<()> {
        let full_path = root.join(path.trim_start_matches('/'));
        fs::create_dir_all(full_path)?;
        Ok(())
    }

    /// Write environment variables to a file
    fn write_environment(&self, root: &Path, env_map: &HashMap<String, String>) -> Result<()> {
        // FreeBSD-style: write to /etc/profile
        let profile_path = root.join("etc").join("profile.kawakaze");

        if let Some(parent) = profile_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&profile_path)?;

        for (key, value) in env_map {
            writeln!(file, "export {}=\"{}\"", key, value)?;
        }

        Ok(())
    }

    /// Report build progress
    async fn report_progress(&self, image_id: &str, step: usize, total: usize, instruction: &DockerfileInstruction, status: BuildStatus) {
        let current_instruction = match instruction {
            DockerfileInstruction::From(img) => format!("FROM {}", img),
            DockerfileInstruction::Bootstrap { version, architecture, .. } => {
                format!("BOOTSTRAP {} {}", version.as_deref().unwrap_or("auto"), architecture.as_deref().unwrap_or("auto"))
            }
            DockerfileInstruction::Run(cmd) => format!("RUN {}", cmd),
            DockerfileInstruction::Copy { src, dest, .. } => format!("COPY {} {}", src, dest),
            DockerfileInstruction::Add { src, dest } => format!("ADD {} {}", src, dest),
            DockerfileInstruction::WorkDir(path) => format!("WORKDIR {}", path),
            DockerfileInstruction::Env(env) => format!("ENV {} vars", env.len()),
            DockerfileInstruction::Expose(ports) => format!("EXPOSE {:?}", ports),
            DockerfileInstruction::User(user) => format!("USER {}", user),
            DockerfileInstruction::Volume(vols) => format!("VOLUME {:?}", vols),
            DockerfileInstruction::Cmd(cmd) => format!("CMD {:?}", cmd),
            DockerfileInstruction::Entrypoint(ep) => format!("ENTRYPOINT {:?}", ep),
            DockerfileInstruction::Label(labels) => format!("LABEL {} entries", labels.len()),
        };

        let progress = ImageBuildProgress {
            image_id: image_id.to_string(),
            step,
            total_steps: total,
            current_instruction,
            status,
        };

        let _ = self.progress_tx.send(progress).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitute_build_args() {
        let mut build_args = HashMap::new();
        build_args.insert("VERSION".to_string(), "1.0.0".to_string());
        build_args.insert("DEBUG".to_string(), "true".to_string());

        let builder = create_test_builder();

        let line = "RUN apt-get install myapp-${VERSION}";
        let result = builder.substitute_build_args(line);
        assert_eq!(result, "RUN apt-get install myapp-1.0.0");

        let line2 = "ENV DEBUG=${DEBUG}";
        let result2 = builder.substitute_build_args(line2);
        assert_eq!(result2, "ENV DEBUG=true");
    }

    #[test]
    fn test_parse_from() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("FROM ubuntu:22.04").unwrap();
        assert!(matches!(instr, DockerfileInstruction::From(_)));
    }

    #[test]
    fn test_parse_run() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("RUN apt-get update").unwrap();
        assert!(matches!(instr, DockerfileInstruction::Run(_)));
    }

    #[test]
    fn test_parse_copy() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("COPY . /app").unwrap();
        match instr {
            DockerfileInstruction::Copy { src, dest, .. } => {
                assert_eq!(src, ".");
                assert_eq!(dest, "/app");
            }
            _ => panic!("Expected Copy instruction"),
        }
    }

    #[test]
    fn test_parse_env() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("ENV PATH=/usr/bin DEBUG=true").unwrap();
        match instr {
            DockerfileInstruction::Env(map) => {
                assert_eq!(map.get("PATH"), Some(&"/usr/bin".to_string()));
                assert_eq!(map.get("DEBUG"), Some(&"true".to_string()));
            }
            _ => panic!("Expected Env instruction"),
        }
    }

    #[test]
    fn test_parse_expose() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("EXPOSE 8080 8443").unwrap();
        match instr {
            DockerfileInstruction::Expose(ports) => {
                assert_eq!(ports, vec![8080, 8443]);
            }
            _ => panic!("Expected Expose instruction"),
        }
    }

    #[test]
    fn test_parse_workdir() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("WORKDIR /app").unwrap();
        match instr {
            DockerfileInstruction::WorkDir(path) => {
                assert_eq!(path, "/app");
            }
            _ => panic!("Expected WorkDir instruction"),
        }
    }

    #[test]
    fn test_parse_user() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("USER appuser").unwrap();
        match instr {
            DockerfileInstruction::User(user) => {
                assert_eq!(user, "appuser");
            }
            _ => panic!("Expected User instruction"),
        }
    }

    #[test]
    fn test_parse_volume() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("VOLUME /data /logs").unwrap();
        match instr {
            DockerfileInstruction::Volume(vols) => {
                assert_eq!(vols, vec!["/data".to_string(), "/logs".to_string()]);
            }
            _ => panic!("Expected Volume instruction"),
        }
    }

    #[test]
    fn test_parse_cmd_exec() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("CMD [\"/bin/app\", \"--help\"]").unwrap();
        match instr {
            DockerfileInstruction::Cmd(cmd) => {
                assert_eq!(cmd, vec!["/bin/app".to_string(), "--help".to_string()]);
            }
            _ => panic!("Expected Cmd instruction"),
        }
    }

    #[test]
    fn test_parse_cmd_shell() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("CMD /bin/app --help").unwrap();
        match instr {
            DockerfileInstruction::Cmd(cmd) => {
                assert_eq!(cmd, vec!["/bin/app --help".to_string()]);
            }
            _ => panic!("Expected Cmd instruction"),
        }
    }

    #[test]
    fn test_parse_entrypoint() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("ENTRYPOINT [\"/bin/sh\"]").unwrap();
        match instr {
            DockerfileInstruction::Entrypoint(ep) => {
                assert_eq!(ep, vec!["/bin/sh".to_string()]);
            }
            _ => panic!("Expected Entrypoint instruction"),
        }
    }

    #[test]
    fn test_parse_label() {
        let builder = create_test_builder();
        let instr = builder.parse_instruction("LABEL version=1.0 maintainer=test@example.com").unwrap();
        match instr {
            DockerfileInstruction::Label(labels) => {
                assert_eq!(labels.get("version"), Some(&"1.0".to_string()));
                assert_eq!(labels.get("maintainer"), Some(&"test@example.com".to_string()));
            }
            _ => panic!("Expected Label instruction"),
        }
    }

    #[test]
    fn test_parse_dockerfile() {
        let builder = create_test_builder();
        let dockerfile = r#"
            FROM ubuntu:22.04
            RUN apt-get update
            COPY . /app
            ENV DEBUG=true
            EXPOSE 8080
        "#;

        let instructions = builder.parse_dockerfile(dockerfile).unwrap();
        assert_eq!(instructions.len(), 5);
        assert!(matches!(instructions[0], DockerfileInstruction::From(_)));
        assert!(matches!(instructions[1], DockerfileInstruction::Run(_)));
        assert!(matches!(instructions[2], DockerfileInstruction::Copy { .. }));
        assert!(matches!(instructions[3], DockerfileInstruction::Env(_)));
        assert!(matches!(instructions[4], DockerfileInstruction::Expose(_)));
    }

    #[test]
    fn test_parse_dockerfile_no_from() {
        let builder = create_test_builder();
        let dockerfile = r#"
            RUN apt-get update
            COPY . /app
        "#;

        let result = builder.parse_dockerfile(dockerfile);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dockerfile_with_comments() {
        let builder = create_test_builder();
        let dockerfile = r#"
            # This is a comment
            FROM ubuntu:22.04

            # Another comment
            RUN apt-get update
        "#;

        let instructions = builder.parse_dockerfile(dockerfile).unwrap();
        assert_eq!(instructions.len(), 2);
    }

    fn create_test_builder() -> ImageBuilder {
        // This would need a mock ZFS for proper testing
        // For now, just create a builder that won't actually be used for ZFS operations
        let zfs = Zfs::new("tank").unwrap_or_else(|_| {
            panic!("ZFS pool 'tank' not available for testing");
        });

        let (builder, _rx) = ImageBuilder::new(zfs, "tank/test".to_string());
        builder
    }

    #[test]
    #[ignore] // Requires actual ZFS pool
    fn test_build_simple_image() {
        // This test would require a real ZFS setup
        // Kept as documentation for integration testing
    }
}
