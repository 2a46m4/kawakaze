//! Unit tests for networking module functionality
//!
//! These tests verify the networking module's parsing logic and configuration.

/// Test helper function for parsing netstat output
/// This is a copy of the logic from NetworkManager::get_default_interface
fn parse_default_interface(netstat_output: &str) -> Option<String> {
    for line in netstat_output.lines() {
        if line.starts_with("default") || line.starts_with("0.0.0.0") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // FreeBSD netstat format: Destination Gateway Flags Netif [Expire]
            // Interface is typically at index 3 (4th column)
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
    fn test_parse_default_interface_freebsd_vtnet() {
        let output = r#"Routing tables

Internet:
Destination        Gateway            Flags      Netif Expire
default            10.0.2.2           UGSc        vtnet0
10.0.2.0/24        link#1             UCS         vtnet0
127.0.0.1          link#2             UH          lo0
"#;

        let result = parse_default_interface(output);
        assert_eq!(result, Some("vtnet0".to_string()));
    }

    #[test]
    fn test_parse_default_interface_freebsd_em() {
        let output = r#"Routing tables

Internet:
Destination        Gateway            Flags      Netif Expire
default            192.168.1.1        UGSc        em0
192.168.1.0/24     link#1             UCS         em0
127.0.0.1          link#2             UH          lo0
"#;

        let result = parse_default_interface(output);
        assert_eq!(result, Some("em0".to_string()));
    }

    #[test]
    fn test_parse_default_interface_0_0_0_0_format() {
        let output = r#"Routing tables

Internet:
Destination        Gateway            Flags      Netif Expire
0.0.0.0/0          192.168.1.254     UGSc        igb0
192.168.1.0/24     link#1             UCS         igb0
127.0.0.1          link#2             UH          lo0
"#;

        let result = parse_default_interface(output);
        assert_eq!(result, Some("igb0".to_string()));
    }

    #[test]
    fn test_parse_default_interface_multiple_columns() {
        // Test with extra columns (like Expire column)
        let output = r#"Routing tables

Internet:
Destination        Gateway            Flags      Netif Expire
default            10.0.2.2           UGSc        vtnet0      100
10.0.2.0/24        link#1             UCS         vtnet0
127.0.0.1          link#2             UH          lo0
"#;

        let result = parse_default_interface(output);
        assert_eq!(result, Some("vtnet0".to_string()));
    }

    #[test]
    fn test_parse_default_interface_no_default_route() {
        let output = r#"Routing tables

Internet:
Destination        Gateway            Flags      Netif Expire
10.0.2.0/24        link#1             UCS         vtnet0
127.0.0.1          link#2             UH          lo0
"#;

        let result = parse_default_interface(output);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_default_interface_insufficient_columns() {
        let output = r#"Routing tables

Internet:
Destination        Gateway            Flags
default            10.0.2.2           UGSc
"#;

        let result = parse_default_interface(output);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_default_interface_empty_output() {
        let output = "";
        let result = parse_default_interface(output);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_default_interface_with_comments() {
        let output = r#"# This is a comment
Routing tables

Internet:
Destination        Gateway            Flags      Netif Expire
default            10.0.2.2           UGSc        vtnet0
127.0.0.1          link#2             UH          lo0
"#;

        let result = parse_default_interface(output);
        assert_eq!(result, Some("vtnet0".to_string()));
    }

    #[test]
    fn test_parse_default_interface_first_default_wins() {
        // Multiple default routes - should return the first one
        let output = r#"Routing tables

Internet:
Destination        Gateway            Flags      Netif Expire
default            10.0.2.2           UGSc        vtnet0
default            192.168.1.1        UGSc        em0
127.0.0.1          link#2             UH          lo0
"#;

        let result = parse_default_interface(output);
        assert_eq!(result, Some("vtnet0".to_string()));
    }

    #[test]
    fn test_parse_default_interface_various_whitespace() {
        // Test with tabs instead of spaces
        let output = "Destination\tGateway\tFlags\tNetif\ndefault\t10.0.2.2\tUGSc\tvtnet0\n";

        let result = parse_default_interface(output);
        assert_eq!(result, Some("vtnet0".to_string()));
    }

    #[test]
    fn test_parse_default_interface_re0() {
        let output = r#"Routing tables

Internet:
Destination        Gateway            Flags      Netif Expire
default            192.168.0.1        UGSc        re0
192.168.0.0/24     link#1             UCS         re0
127.0.0.1          link#2             UH          lo0
"#;

        let result = parse_default_interface(output);
        assert_eq!(result, Some("re0".to_string()));
    }
}
