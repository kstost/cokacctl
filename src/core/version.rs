use std::process::Command;

const CARGO_TOML_URL: &str =
    "https://raw.githubusercontent.com/kstost/cokacdir/refs/heads/main/Cargo.toml";

/// Get the installed cokacdir version by running `cokacdir --version`.
pub fn installed_version(binary_path: &std::path::Path) -> Option<String> {
    dlog!("version", "Getting installed version from: {}", binary_path.display());
    let label = format!("{} --version", binary_path.display());
    let output = match Command::new(binary_path).arg("--version").output() {
        Ok(o) => {
            crate::core::debug::log_output("version", &label, &o);
            o
        }
        Err(e) => {
            dlog!("version", "exec failed: {}", e);
            return None;
        }
    };
    if !output.status.success() {
        dlog!("version", "cokacdir --version failed (exit: {})", output.status);
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let ver = stdout
        .trim()
        .strip_prefix("cokacdir ")
        .map(|v| v.trim().to_string());
    dlog!("version", "Installed version: {:?}", ver);
    ver
}

/// Fetch the latest cokacdir version from the GitHub Cargo.toml.
pub async fn latest_version() -> Option<String> {
    dlog!("version", "Fetching latest version from {}", CARGO_TOML_URL);
    let started = std::time::Instant::now();
    let client = reqwest::Client::new();
    let resp = match client
        .get(CARGO_TOML_URL)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            dlog!("version", "HTTP request failed: {}", e);
            return None;
        }
    };
    let status = resp.status();
    let headers = resp.headers().clone();
    dlog!("version", "HTTP response status: {}", status);
    dlog!("version", "HTTP response headers: {:?}", headers);
    let text = match resp.text().await {
        Ok(t) => {
            dlog!("version", "HTTP body received: {} bytes in {:?}", t.len(), started.elapsed());
            t
        }
        Err(e) => {
            dlog!("version", "HTTP body read failed: {}", e);
            return None;
        }
    };
    let ver = parse_version_from_cargo_toml(&text);
    dlog!("version", "Latest version parsed: {:?}", ver);
    ver
}

/// Parse `version = "x.x.x"` from Cargo.toml content.
fn parse_version_from_cargo_toml(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("version") {
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if start < end {
                        return Some(line[start + 1..end].to_string());
                    }
                }
            }
        }
    }
    None
}

/// Compare two semver-like version strings. Returns true if `a` > `b`.
pub fn is_newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.').filter_map(|p| p.parse().ok()).collect()
    };
    parse(a) > parse(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        assert!(is_newer("0.4.68", "0.4.67"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.4.67", "0.4.67"));
        assert!(!is_newer("0.4.66", "0.4.67"));
    }

    #[test]
    fn test_parse_version() {
        let toml = r#"
[package]
name = "cokacdir"
version = "0.4.67"
edition = "2021"
"#;
        assert_eq!(
            parse_version_from_cargo_toml(toml),
            Some("0.4.67".to_string())
        );
    }
}
