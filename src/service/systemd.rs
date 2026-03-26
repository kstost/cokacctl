use super::{ServiceManager, ServiceStatus};
use crate::core::platform::ServicePaths;
use std::path::{Path, PathBuf};
use std::process::Command;

const SERVICE_NAME: &str = "cokacdir";

pub struct SystemdManager {
    paths: ServicePaths,
}

impl SystemdManager {
    pub fn new() -> Self {
        SystemdManager {
            paths: ServicePaths::for_current_os(),
        }
    }

    fn escape_shell_arg(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\\''"))
    }

    fn escape_systemd_arg(s: &str) -> String {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "$$")
            .replace('%', "%%");
        format!("\"{}\"", escaped)
    }

    fn generate_wrapper(binary_path: &Path, tokens: &[String]) -> String {
        let args: Vec<String> = tokens.iter().map(|t| Self::escape_shell_arg(t)).collect();
        format!(
            "#!/bin/bash -i\nexec {} --ccserver -- {}\n",
            Self::escape_shell_arg(&binary_path.to_string_lossy()),
            args.join(" ")
        )
    }

    fn systemd_version() -> u32 {
        let output = Command::new("systemctl")
            .arg("--version")
            .output()
            .ok();
        match output {
            Some(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout
                    .lines()
                    .next()
                    .and_then(|line| {
                        line.split_whitespace()
                            .find_map(|w| w.parse::<u32>().ok())
                    })
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn generate_service(&self) -> String {
        let wrapper = Self::escape_systemd_arg(&self.paths.wrapper_script.to_string_lossy());
        let log_dir = self.paths.log_dir.to_string_lossy()
            .replace('$', "$$")
            .replace('%', "%%");

        let version = Self::systemd_version();
        let stdout_directive = if version >= 240 {
            format!("append:{}/cokacdir.log", log_dir)
        } else if version >= 236 {
            format!("file:{}/cokacdir.log", log_dir)
        } else {
            "journal".to_string()
        };
        let stderr_directive = if version >= 240 {
            format!("append:{}/cokacdir.error.log", log_dir)
        } else if version >= 236 {
            format!("file:{}/cokacdir.error.log", log_dir)
        } else {
            "journal".to_string()
        };

        format!(
            "[Unit]\n\
             Description=Cokacdir Server Service\n\
             After=network.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             ExecStart={exec}\n\
             Restart=always\n\
             RestartSec=5\n\
             StandardOutput={stdout}\n\
             StandardError={stderr}\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            exec = wrapper,
            stdout = stdout_directive,
            stderr = stderr_directive,
        )
    }
}

impl ServiceManager for SystemdManager {
    fn start(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String> {
        // Check systemctl
        if Command::new("systemctl").arg("--version").output().is_err() {
            return Err("systemctl not found. This tool requires systemd.".into());
        }

        // Create directories
        std::fs::create_dir_all(&self.paths.log_dir)
            .map_err(|e| format!("Cannot create log dir: {}", e))?;
        if let Some(parent) = self.paths.service_file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create systemd dir: {}", e))?;
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

        // Stop existing
        let _ = Command::new("systemctl")
            .args(["--user", "stop", SERVICE_NAME])
            .output();

        // Write service file
        let service = self.generate_service();
        std::fs::write(&self.paths.service_file, &service)
            .map_err(|e| format!("Cannot write service file: {}", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &self.paths.service_file,
                std::fs::Permissions::from_mode(0o600),
            )
            .ok();
        }

        // daemon-reload
        let r = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output()
            .map_err(|e| format!("daemon-reload failed: {}", e))?;
        if !r.status.success() {
            return Err("systemctl daemon-reload failed".into());
        }

        // enable
        let r = Command::new("systemctl")
            .args(["--user", "enable", SERVICE_NAME])
            .output()
            .map_err(|e| format!("enable failed: {}", e))?;
        if !r.status.success() {
            return Err("systemctl enable failed".into());
        }

        // restart
        let r = Command::new("systemctl")
            .args(["--user", "restart", SERVICE_NAME])
            .output()
            .map_err(|e| format!("restart failed: {}", e))?;
        if !r.status.success() {
            return Err("systemctl restart failed".into());
        }

        // enable-linger
        if let Some(user) = std::env::var("USER").ok() {
            let _ = Command::new("loginctl")
                .args(["enable-linger", &user])
                .output();
        }

        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        let r = Command::new("systemctl")
            .args(["--user", "stop", SERVICE_NAME])
            .output()
            .map_err(|e| format!("stop failed: {}", e))?;
        if !r.status.success() {
            let stderr = String::from_utf8_lossy(&r.stderr);
            if !stderr.contains("not loaded") && !stderr.contains("not found") {
                return Err(format!("systemctl stop failed: {}", stderr));
            }
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        let _ = Command::new("systemctl")
            .args(["--user", "stop", SERVICE_NAME])
            .output();
        let _ = Command::new("systemctl")
            .args(["--user", "disable", SERVICE_NAME])
            .output();
        if self.paths.service_file.exists() {
            std::fs::remove_file(&self.paths.service_file)
                .map_err(|e| format!("Cannot remove service file: {}", e))?;
        }
        if self.paths.wrapper_script.exists() {
            std::fs::remove_file(&self.paths.wrapper_script).ok();
        }
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        if !self.paths.service_file.exists() {
            return ServiceStatus::NotInstalled;
        }
        let output = Command::new("systemctl")
            .args(["--user", "is-active", SERVICE_NAME])
            .output();
        match output {
            Ok(out) => {
                let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
                match state.as_str() {
                    "active" => ServiceStatus::Running,
                    "inactive" | "failed" => ServiceStatus::Stopped,
                    _ => ServiceStatus::Unknown(state),
                }
            }
            Err(_) => ServiceStatus::Unknown("Cannot query systemctl".into()),
        }
    }

    fn log_path(&self) -> Option<PathBuf> {
        Some(self.paths.log_file.clone())
    }
}
