use clap::Parser;
use std::path::PathBuf;

/// A simple CLI tool to parse Dockerfiles
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the Dockerfile to parse
    dockerfile: PathBuf,
}

fn main() {
    let args = Args::parse();

    // Read the Dockerfile content
    let content = match std::fs::read_to_string(&args.dockerfile) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading file '{}': {}", args.dockerfile.display(), e);
            std::process::exit(1);
        }
    };

    // Parse the Dockerfile
    let dockerfile = match dockerfile_parser::Dockerfile::parse(&content) {
        Ok(dockerfile) => dockerfile,
        Err(e) => {
            eprintln!("Error parsing Dockerfile: {}", e);
            std::process::exit(1);
        }
    };

    // Print out the parsed Dockerfile
    println!("{:#?}", dockerfile);
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_parse_simple_dockerfile() {
        let content = r#"
FROM ubuntu:22.04
RUN echo "hello"
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
        let dockerfile = result.unwrap();
        assert!(!dockerfile.instructions.is_empty());
    }

    #[test]
    fn test_parse_from_instruction() {
        let content = "FROM ubuntu:22.04\n";
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
        let dockerfile = result.unwrap();
        assert_eq!(dockerfile.instructions.len(), 1);
    }

    #[test]
    fn test_parse_multistage_dockerfile() {
        let content = r#"
FROM ubuntu:22.04 AS builder
RUN apt-get update

FROM alpine:3.18
COPY --from=builder /app /app
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
        let dockerfile = result.unwrap();
        assert!(dockerfile.instructions.len() >= 2);
    }

    #[test]
    fn test_parse_workdir_instruction() {
        let content = r#"
FROM ubuntu:22.04
WORKDIR /app
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_env_instruction() {
        let content = r#"
FROM ubuntu:22.04
ENV NODE_ENV=production
ENV PATH="/usr/local/bin:${PATH}"
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_copy_instruction() {
        let content = r#"
FROM ubuntu:22.04
COPY . /app
COPY config.json /etc/app/config.json
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_expose_instruction() {
        let content = r#"
FROM ubuntu:22.04
EXPOSE 8080
EXPOSE 8080/tcp 8443/udp
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_cmd_and_entrypoint() {
        let content = r#"
FROM ubuntu:22.04
ENTRYPOINT ["/app/start.sh"]
CMD ["--server"]
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_comments_and_blank_lines() {
        let content = r#"
# This is a comment
FROM ubuntu:22.04

# Another comment

RUN echo "test"
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_label_instruction() {
        let content = r#"
FROM ubuntu:22.04
LABEL maintainer="test@example.com"
LABEL version="1.0" description="Test app"
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_user_instruction() {
        let content = r#"
FROM ubuntu:22.04
USER appuser
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_volume_instruction() {
        let content = r#"
FROM ubuntu:22.04
VOLUME ["/data", "/logs"]
VOLUME /app
"#;
        let result = dockerfile_parser::Dockerfile::parse(content);
        assert!(result.is_ok());
    }
}
