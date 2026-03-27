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
        dlog!("systemd", "SystemdManager created");
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
                let ver = stdout
                    .lines()
                    .next()
                    .and_then(|line| {
                        line.split_whitespace()
                            .find_map(|w| w.parse::<u32>().ok())
                    })
                    .unwrap_or(0);
                dlog!("systemd", "systemd version: {}", ver);
                ver
            }
            _ => {
                dlog!("systemd", "Failed to detect systemd version");
                0
            }
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
        dlog!("systemd", "start() called - binary: {}, tokens: {}", binary_path.display(), tokens.len());

        if Command::new("systemctl").arg("--version").output().is_err() {
            dlog!("systemd", "systemctl not found");
            return Err("systemctl not found. This tool requires systemd.".into());
        }

        dlog!("systemd", "Creating directories...");
        std::fs::create_dir_all(&self.paths.log_dir)
            .map_err(|e| format!("Cannot create log dir: {}", e))?;
        if let Some(parent) = self.paths.service_file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create systemd dir: {}", e))?;
        }

        let wrapper = Self::generate_wrapper(binary_path, tokens);
        dlog!("systemd", "Writing wrapper to: {}", self.paths.wrapper_script.display());
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

        dlog!("systemd", "Stopping existing service...");
        let _ = Command::new("systemctl")
            .args(["--user", "stop", SERVICE_NAME])
            .output();

        let service = self.generate_service();
        dlog!("systemd", "Writing service file to: {}", self.paths.service_file.display());
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

        dlog!("systemd", "Running daemon-reload...");
        let r = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output()
            .map_err(|e| format!("daemon-reload failed: {}", e))?;
        if !r.status.success() {
            dlog!("systemd", "daemon-reload failed");
            return Err("systemctl daemon-reload failed".into());
        }

        dlog!("systemd", "Enabling service...");
        let r = Command::new("systemctl")
            .args(["--user", "enable", SERVICE_NAME])
            .output()
            .map_err(|e| format!("enable failed: {}", e))?;
        if !r.status.success() {
            dlog!("systemd", "enable failed");
            return Err("systemctl enable failed".into());
        }

        // Truncate error log before starting so we only capture fresh errors
        let error_log_path = self.paths.log_dir.join("cokacdir.error.log");
        let _ = std::fs::File::create(&error_log_path);

        dlog!("systemd", "Restarting service...");
        let r = Command::new("systemctl")
            .args(["--user", "restart", SERVICE_NAME])
            .output()
            .map_err(|e| format!("restart failed: {}", e))?;
        if !r.status.success() {
            dlog!("systemd", "restart failed");
            return Err("systemctl restart failed".into());
        }

        if let Some(user) = std::env::var("USER").ok() {
            dlog!("systemd", "Enabling linger for user: {}", user);
            let _ = Command::new("loginctl")
                .args(["enable-linger", &user])
                .output();
        }

        // Check if service actually stays running
        std::thread::sleep(std::time::Duration::from_millis(2000));
        let status = self.status();
        if status != ServiceStatus::Running {
            let err_output = std::fs::read_to_string(&error_log_path).unwrap_or_default();
            let tail: String = err_output.lines().rev().take(10)
                .collect::<Vec<_>>().into_iter().rev()
                .collect::<Vec<_>>().join("\n");
            dlog!("systemd", "Service not running after restart. Error log: '{}'", tail.trim());
            if !tail.trim().is_empty() {
                return Err(tail.trim().to_string());
            }
            return Err("Service started but exited immediately".into());
        }

        dlog!("systemd", "start() completed successfully");
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        dlog!("systemd", "stop() called");
        let r = Command::new("systemctl")
            .args(["--user", "stop", SERVICE_NAME])
            .output()
            .map_err(|e| format!("stop failed: {}", e))?;
        if !r.status.success() {
            let stderr = String::from_utf8_lossy(&r.stderr);
            if !stderr.contains("not loaded") && !stderr.contains("not found") {
                dlog!("systemd", "stop() failed: {}", stderr);
                return Err(format!("systemctl stop failed: {}", stderr));
            }
            dlog!("systemd", "stop(): service was not loaded");
        } else {
            dlog!("systemd", "stop() success");
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        dlog!("systemd", "remove() called");
        let _ = Command::new("systemctl")
            .args(["--user", "stop", SERVICE_NAME])
            .output();
        let _ = Command::new("systemctl")
            .args(["--user", "disable", SERVICE_NAME])
            .output();
        if self.paths.service_file.exists() {
            dlog!("systemd", "Removing service file: {}", self.paths.service_file.display());
            std::fs::remove_file(&self.paths.service_file)
                .map_err(|e| format!("Cannot remove service file: {}", e))?;
        }
        if self.paths.wrapper_script.exists() {
            dlog!("systemd", "Removing wrapper: {}", self.paths.wrapper_script.display());
            std::fs::remove_file(&self.paths.wrapper_script).ok();
        }
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
        dlog!("systemd", "remove() complete");
        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        dlog!("systemd", "status() called");
        if !self.paths.service_file.exists() {
            dlog!("systemd", "status(): service file not found -> NotInstalled");
            return ServiceStatus::NotInstalled;
        }
        let output = Command::new("systemctl")
            .args(["--user", "is-active", SERVICE_NAME])
            .output();
        match output {
            Ok(out) => {
                let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
                dlog!("systemd", "status(): systemctl is-active = '{}'", state);
                match state.as_str() {
                    "active" => ServiceStatus::Running,
                    "inactive" | "failed" => ServiceStatus::Stopped,
                    _ => ServiceStatus::Unknown(state),
                }
            }
            Err(e) => {
                dlog!("systemd", "status() query failed: {}", e);
                ServiceStatus::Unknown("Cannot query systemctl".into())
            }
        }
    }

    fn log_path(&self) -> Option<PathBuf> {
        Some(self.paths.log_file.clone())
    }
}
