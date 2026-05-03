#[cfg_attr(not(feature = "stylos"), allow(dead_code))]
pub(crate) fn derive_local_instance_id() -> String {
    let base = {
        #[cfg(feature = "stylos")]
        {
            if let Some(hostname) = std::env::var_os("HOSTNAME").and_then(|v| v.into_string().ok()) {
                let hostname = hostname.trim();
                if !hostname.is_empty() {
                    hostname.to_string()
                } else if let Ok(output) = std::process::Command::new("hostname").output() {
                    if output.status.success() {
                        if let Ok(hostname) = String::from_utf8(output.stdout) {
                            let hostname = hostname.trim();
                            if !hostname.is_empty() {
                                hostname.to_string()
                            } else {
                                "local".to_string()
                            }
                        } else {
                            "local".to_string()
                        }
                    } else {
                        "local".to_string()
                    }
                } else {
                    "local".to_string()
                }
            } else if let Ok(output) = std::process::Command::new("hostname").output() {
                if output.status.success() {
                    if let Ok(hostname) = String::from_utf8(output.stdout) {
                        let hostname = hostname.trim();
                        if !hostname.is_empty() {
                            hostname.to_string()
                        } else {
                            "local".to_string()
                        }
                    } else {
                        "local".to_string()
                    }
                } else {
                    "local".to_string()
                }
            } else {
                "local".to_string()
            }
        }
        #[cfg(not(feature = "stylos"))]
        {
            "local".to_string()
        }
    };
    format!("{}:{}", base, std::process::id())
}
