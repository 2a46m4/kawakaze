//! Integration tests for port forwarding functionality
//!
//! These tests verify that port forwarding rules are properly configured
//! using pf (Packet Filter).

use std::process::Command;
use std::io::Write;

/// Helper to check if pf is enabled
fn pf_enabled() -> bool {
    let output = Command::new("pfctl")
        .arg("-s")
        .arg("info")
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.contains("Status: Enabled")
        }
        Err(_) => false,
    }
}

/// Helper to get all NAT rules from kawakaze_forwarding anchor
fn get_forwarding_rules() -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("pfctl")
        .arg("-a")
        .arg("kawakaze_forwarding")
        .arg("-s")
        .arg("nat")
        .output()?;

    if !output.status.success() {
        return Err(format!("pfctl failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Helper to flush all port forwarding rules
fn flush_forwarding_rules() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("pfctl")
        .arg("-a")
        .arg("kawakaze_forwarding")
        .arg("-F")
        .arg("all")
        .output()?;

    if !output.status.success() {
        return Err(format!("pfctl flush failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }

    Ok(())
}

/// Helper to add a port forwarding rule
fn add_forwarding_rule(host_port: u16, container_ip: &str, container_port: u16, protocol: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Get the external interface
    let external_interface = get_default_interface()
        .ok_or("Could not determine external interface")?;

    let rule = format!(
        "rdr pass on {} inet proto {} from any to any port {} -> {} port {}",
        external_interface, protocol, host_port, container_ip, container_port
    );

    let output = Command::new("pfctl")
        .arg("-a")
        .arg("kawakaze_forwarding")
        .arg("-f")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .spawn()?
        .stdin
        .ok_or("Failed to open stdin")?
        .write_all(rule.as_bytes())?;

    Ok(())
}

/// Get the default network interface
fn get_default_interface() -> Option<String> {
    let output = Command::new("netstat")
        .arg("-nr")
        .arg("-f")
        .arg("inet")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        if line.starts_with("default") || line.starts_with("0.0.0.0") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                return Some(parts[3].to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Skip if pf is not available
    fn test_pf_enabled() {
        if !pf_enabled() {
            eprintln!("Skipping test: pf is not enabled");
            return;
        }
        assert!(pf_enabled());
    }

    #[test]
    #[ignore]
    fn test_get_default_interface() {
        let interface = get_default_interface();
        assert!(interface.is_some(), "Could not determine default interface");

        let iface_name = interface.unwrap();
        assert!(!iface_name.is_empty(), "Interface name is empty");

        // Common FreeBSD interface names
        let valid_prefixes = vec!["em", "igb", "re", "vtnet", "xn", "alc", "bge"];
        let has_valid_prefix = valid_prefixes.iter().any(|prefix| iface_name.starts_with(prefix));
        assert!(has_valid_prefix, "Interface {} doesn't match expected patterns", iface_name);
    }

    #[test]
    #[ignore]
    fn test_flush_forwarding_rules() {
        if !pf_enabled() {
            eprintln!("Skipping test: pf is not enabled");
            return;
        }

        // Add a test rule first
        let _ = add_forwarding_rule(9999, "10.11.0.100", 80, "tcp");

        // Flush all rules
        let result = flush_forwarding_rules();
        assert!(result.is_ok(), "Failed to flush forwarding rules: {:?}", result.err());

        // Verify no rules remain
        let rules = get_forwarding_rules().unwrap();
        assert!(rules.trim().is_empty() || rules.trim() == "No NAT rules", "Rules should be empty after flush");
    }

    #[test]
    #[ignore]
    fn test_add_and_list_forwarding_rule() {
        if !pf_enabled() {
            eprintln!("Skipping test: pf is not enabled");
            return;
        }

        // Clean up any existing rules
        let _ = flush_forwarding_rules();

        // Add a test rule
        let result = add_forwarding_rule(9998, "10.11.0.101", 8080, "tcp");
        assert!(result.is_ok(), "Failed to add forwarding rule: {:?}", result.err());

        // Verify the rule was added
        let rules = get_forwarding_rules().unwrap();
        assert!(rules.contains("9998"), "Rule should contain port 9998");
        assert!(rules.contains("10.11.0.101"), "Rule should contain IP 10.11.0.101");
        assert!(rules.contains("8080"), "Rule should contain port 8080");

        // Clean up
        let _ = flush_forwarding_rules();
    }

    #[test]
    #[ignore]
    fn test_multiple_forwarding_rules() {
        if !pf_enabled() {
            eprintln!("Skipping test: pf is not enabled");
            return;
        }

        // Clean up any existing rules
        let _ = flush_forwarding_rules();

        // Add multiple rules
        let rules_to_add = vec![
            (8001, "10.11.0.10", 80, "tcp"),
            (8002, "10.11.0.11", 80, "tcp"),
            (3306, "10.11.0.12", 3306, "tcp"),
        ];

        for (host_port, container_ip, container_port, protocol) in &rules_to_add {
            let result = add_forwarding_rule(*host_port, container_ip, *container_port, protocol);
            assert!(result.is_ok(), "Failed to add rule for port {}: {:?}", host_port, result.err());
        }

        // Verify all rules were added
        let rules = get_forwarding_rules().unwrap();
        for (host_port, container_ip, container_port, _) in &rules_to_add {
            assert!(rules.contains(&host_port.to_string()), "Missing rule for port {}", host_port);
            assert!(rules.contains(*container_ip), "Missing rule for IP {}", container_ip);
            assert!(rules.contains(&container_port.to_string()), "Missing rule for port {}", container_port);
        }

        // Clean up
        let _ = flush_forwarding_rules();
    }

    #[test]
    #[ignore]
    fn test_udp_forwarding_rule() {
        if !pf_enabled() {
            eprintln!("Skipping test: pf is not enabled");
            return;
        }

        // Clean up any existing rules
        let _ = flush_forwarding_rules();

        // Add a UDP rule
        let result = add_forwarding_rule(5353, "10.11.0.200", 53, "udp");
        assert!(result.is_ok(), "Failed to add UDP rule: {:?}", result.err());

        // Verify the UDP rule was added
        let rules = get_forwarding_rules().unwrap();
        assert!(rules.contains("udp"), "Rule should contain 'udp'");
        assert!(rules.contains("5353"), "Rule should contain port 5353");
        assert!(rules.contains("53"), "Rule should contain port 53");

        // Clean up
        let _ = flush_forwarding_rules();
    }

    #[test]
    #[ignore]
    fn test_forwarding_rule_format() {
        if !pf_enabled() {
            eprintln!("Skipping test: pf is not enabled");
            return;
        }

        // Clean up
        let _ = flush_forwarding_rules();

        // Add a rule
        let _ = add_forwarding_rule(8888, "10.11.0.50", 80, "tcp");

        // Check rule format
        let rules = get_forwarding_rules().unwrap();

        // Verify key components are present
        assert!(rules.contains("rdr"), "Rule should contain 'rdr'");
        assert!(rules.contains("pass"), "Rule should contain 'pass'");
        assert!(rules.contains("proto"), "Rule should contain 'proto'");
        assert!(rules.contains("tcp"), "Rule should contain 'tcp'");
        assert!(rules.contains("->"), "Rule should contain '->'");

        // Clean up
        let _ = flush_forwarding_rules();
    }
}
