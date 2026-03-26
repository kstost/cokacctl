use super::{ServiceManager, ServiceStatus};
use crate::core::platform::ServicePaths;
use std::path::{Path, PathBuf};
use std::process::Command;

const LABEL: &str = "com.cokacdir.server";

pub struct LaunchdManager {
    paths: ServicePaths,
}

impl LaunchdManager {
    pub fn new() -> Self {
        LaunchdManager {
            paths: ServicePaths::for_current_os(),
        }
    }

    fn uid() -> u32 {
        #[cfg(unix)]
        {
            unsafe { libc::getuid() }
        }
        #[cfg(not(unix))]
        {
            0
        }
    }

    fn domain() -> String {
        format!("gui/{}", Self::uid())
    }

    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn escape_shell_arg(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\\''"))
    }

    fn generate_wrapper(binary_path: &Path, tokens: &[String]) -> String {
        let args: Vec<String> = tokens.iter().map(|t| Self::escape_shell_arg(t)).collect();
        format!(
            "#!/bin/zsh\nexec {} --ccserver -- {}\n",
            Self::escape_shell_arg(&binary_path.to_string_lossy()),
            args.join(" ")
        )
    }

    fn generate_plist(&self, wrapper_path: &Path) -> String {
        let wrapper_str = Self::escape_xml(&wrapper_path.to_string_lossy());
        let log_dir = Self::escape_xml(&self.paths.log_dir.to_string_lossy());
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/zsh</string>
        <string>-li</string>
        <string>{wrapper}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log_dir}/cokacdir.log</string>
    <key>StandardErrorPath</key>
    <string>{log_dir}/cokacdir.error.log</string>
</dict>
</plist>
"#,
            label = LABEL,
            wrapper = wrapper_str,
            log_dir = log_dir,
        )
    }
}

impl ServiceManager for LaunchdManager {
    fn start(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String> {
        // Create directories
        std::fs::create_dir_all(&self.paths.log_dir)
            .map_err(|e| format!("Cannot create log dir: {}", e))?;
        if let Some(parent) = self.paths.service_file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create LaunchAgents dir: {}", e))?;
        }

        // Write wrapper script
        let wrapper = Self::generate_wrapper(binary_path, tokens);
        std::fs::write(&self.paths.wrapper_script, &wrapper)
            .map_err(|e| format!("Cannot write wrapper: {}", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &self.paths.wrapper_script,
                std::fs::Permissions::from_mode(0o700),
            )
            .ok();
        }

        // Stop existing service if running
        let _ = self.stop();

        // Write plist
        let plist = self.generate_plist(&self.paths.wrapper_script);
        std::fs::write(&self.paths.service_file, &plist)
            .map_err(|e| format!("Cannot write plist: {}", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &self.paths.service_file,
                std::fs::Permissions::from_mode(0o600),
            )
            .ok();
        }

        // Enable
        let domain = Self::domain();
        let _ = Command::new("launchctl")
            .args(["enable", &format!("{}/{}", domain, LABEL)])
            .output();

        // Bootstrap
        let result = Command::new("launchctl")
            .args([
                "bootstrap",
                &domain,
                &self.paths.service_file.to_string_lossy(),
            ])
            .output()
            .map_err(|e| format!("launchctl bootstrap failed: {}", e))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(format!("launchctl bootstrap failed: {}", stderr));
        }

        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        let domain = Self::domain();
        let result = Command::new("launchctl")
            .args(["bootout", &format!("{}/{}", domain, LABEL)])
            .output()
            .map_err(|e| format!("launchctl bootout failed: {}", e))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            // Not an error if service wasn't running
            if !stderr.contains("No such process") && !stderr.contains("Could not find service") {
                return Err(format!("launchctl bootout failed: {}", stderr));
            }
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        self.stop().ok();
        if self.paths.service_file.exists() {
            std::fs::remove_file(&self.paths.service_file)
                .map_err(|e| format!("Cannot remove plist: {}", e))?;
        }
        if self.paths.wrapper_script.exists() {
            std::fs::remove_file(&self.paths.wrapper_script).ok();
        }
        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        if !self.paths.service_file.exists() {
            return ServiceStatus::NotInstalled;
        }
        let output = Command::new("launchctl")
            .args(["list"])
            .output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.contains(LABEL) {
                    ServiceStatus::Running
                } else {
                    ServiceStatus::Stopped
                }
            }
            Err(_) => ServiceStatus::Unknown("Cannot query launchctl".into()),
        }
    }

    fn log_path(&self) -> Option<PathBuf> {
        Some(self.paths.log_file.clone())
    }
}
