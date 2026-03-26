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
        dlog!("launchd", "LaunchdManager created");
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
        dlog!("launchd", "start() called - binary: {}, tokens: {}", binary_path.display(), tokens.len());

        std::fs::create_dir_all(&self.paths.log_dir)
            .map_err(|e| format!("Cannot create log dir: {}", e))?;
        if let Some(parent) = self.paths.service_file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create LaunchAgents dir: {}", e))?;
        }

        let wrapper = Self::generate_wrapper(binary_path, tokens);
        dlog!("launchd", "Writing wrapper to: {}", self.paths.wrapper_script.display());
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

        dlog!("launchd", "Stopping existing service...");
        let _ = self.stop();

        let plist = self.generate_plist(&self.paths.wrapper_script);
        dlog!("launchd", "Writing plist to: {}", self.paths.service_file.display());
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

        let domain = Self::domain();
        dlog!("launchd", "Enabling service in domain: {}", domain);
        let _ = Command::new("launchctl")
            .args(["enable", &format!("{}/{}", domain, LABEL)])
            .output();

        dlog!("launchd", "Bootstrapping service...");
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
            dlog!("launchd", "Bootstrap failed: {}", stderr);
            return Err(format!("launchctl bootstrap failed: {}", stderr));
        }

        dlog!("launchd", "start() completed successfully");
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        dlog!("launchd", "stop() called");
        let domain = Self::domain();
        let result = Command::new("launchctl")
            .args(["bootout", &format!("{}/{}", domain, LABEL)])
            .output()
            .map_err(|e| format!("launchctl bootout failed: {}", e))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            if !stderr.contains("No such process") && !stderr.contains("Could not find service") {
                dlog!("launchd", "stop() failed: {}", stderr);
                return Err(format!("launchctl bootout failed: {}", stderr));
            }
            dlog!("launchd", "stop(): service was not running");
        } else {
            dlog!("launchd", "stop() success");
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        dlog!("launchd", "remove() called");
        self.stop().ok();
        if self.paths.service_file.exists() {
            dlog!("launchd", "Removing plist: {}", self.paths.service_file.display());
            std::fs::remove_file(&self.paths.service_file)
                .map_err(|e| format!("Cannot remove plist: {}", e))?;
        }
        if self.paths.wrapper_script.exists() {
            dlog!("launchd", "Removing wrapper: {}", self.paths.wrapper_script.display());
            std::fs::remove_file(&self.paths.wrapper_script).ok();
        }
        dlog!("launchd", "remove() complete");
        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        dlog!("launchd", "status() called");
        if !self.paths.service_file.exists() {
            dlog!("launchd", "status(): plist not found -> NotInstalled");
            return ServiceStatus::NotInstalled;
        }
        let output = Command::new("launchctl")
            .args(["list"])
            .output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.contains(LABEL) {
                    dlog!("launchd", "status(): Running");
                    ServiceStatus::Running
                } else {
                    dlog!("launchd", "status(): Stopped");
                    ServiceStatus::Stopped
                }
            }
            Err(e) => {
                dlog!("launchd", "status() query failed: {}", e);
                ServiceStatus::Unknown("Cannot query launchctl".into())
            }
        }
    }

    fn log_path(&self) -> Option<PathBuf> {
        Some(self.paths.log_file.clone())
    }
}
