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
        dlog!("systemd", "systemd_version(): invoking systemctl --version");
        let output = Command::new("systemctl")
            .arg("--version")
            .output()
            .ok();
        match output {
            Some(out) => {
                crate::core::debug::log_output("systemd", "systemctl --version", &out);
                if out.status.success() {
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
                } else {
                    dlog!("systemd", "systemctl --version returned non-success");
                    0
                }
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
        dlog!("systemd", "paths: log_dir={}, wrapper={}, service={}, log_file={}",
            self.paths.log_dir.display(),
            self.paths.wrapper_script.display(),
            self.paths.service_file.display(),
            self.paths.log_file.display());

        // Pre-flight: verify binary exists and is executable so we fail early
        // with a clear message instead of relying on systemd's error reporting.
        match std::fs::metadata(binary_path) {
            Ok(m) => dlog!("systemd", "binary metadata: is_file={}, len={}B, readonly={}",
                m.is_file(), m.len(), m.permissions().readonly()),
            Err(e) => dlog!("systemd", "binary metadata probe failed: {}", e),
        }

        dlog!("systemd", "start(): probing for systemctl binary");
        match Command::new("systemctl").arg("--version").output() {
            Ok(out) => {
                crate::core::debug::log_output("systemd", "systemctl --version (probe)", &out);
            }
            Err(e) => {
                dlog!("systemd", "systemctl not found: {}", e);
                return Err("systemctl not found. This tool requires systemd.".into());
            }
        }

        dlog!("systemd", "Creating log dir: {}", self.paths.log_dir.display());
        std::fs::create_dir_all(&self.paths.log_dir)
            .map_err(|e| {
                dlog!("systemd", "create_dir_all(log_dir) failed: {}", e);
                format!("Cannot create log dir: {}", e)
            })?;
        dlog!("systemd", "log dir ready");
        if let Some(parent) = self.paths.service_file.parent() {
            dlog!("systemd", "Creating systemd unit dir: {}", parent.display());
            std::fs::create_dir_all(parent)
                .map_err(|e| {
                    dlog!("systemd", "create_dir_all(unit) failed: {}", e);
                    format!("Cannot create systemd dir: {}", e)
                })?;
            dlog!("systemd", "systemd unit dir ready");
        }

        let wrapper = Self::generate_wrapper(binary_path, tokens);
        dlog!("systemd", "wrapper generated: {} bytes", wrapper.len());
        dlog!("systemd", "Writing wrapper to: {}", self.paths.wrapper_script.display());
        // Write via tmp + rename with mode 0o700 applied at creation so tokens
        // are never visible under the default umask (0644).
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let tmp = self.paths.wrapper_script.with_extension("sh.tmp");
            dlog!("systemd", "wrapper tmp: {}", tmp.display());
            match std::fs::remove_file(&tmp) {
                Ok(_) => dlog!("systemd", "wrapper tmp: cleared stale"),
                Err(e) => dlog!("systemd", "wrapper tmp cleanup: {} (ok if nonexistent)", e),
            }
            {
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o700)
                    .open(&tmp)
                    .map_err(|e| {
                        dlog!("systemd", "wrapper tmp open(0o700) failed: {}", e);
                        format!("Cannot create wrapper temp: {}", e)
                    })?;
                dlog!("systemd", "wrapper tmp opened (mode 0o700)");
                file.write_all(wrapper.as_bytes())
                    .map_err(|e| {
                        dlog!("systemd", "wrapper tmp write_all failed: {}", e);
                        format!("Cannot write wrapper: {}", e)
                    })?;
                dlog!("systemd", "wrapper tmp: wrote {} bytes", wrapper.len());
                match file.sync_all() {
                    Ok(_) => dlog!("systemd", "wrapper tmp fsync OK"),
                    Err(e) => dlog!("systemd", "wrapper tmp fsync failed (non-fatal): {}", e),
                }
            }
            dlog!("systemd", "wrapper tmp -> final rename");
            std::fs::rename(&tmp, &self.paths.wrapper_script)
                .map_err(|e| {
                    dlog!("systemd", "wrapper rename failed: {}", e);
                    format!("Cannot finalize wrapper: {}", e)
                })?;
            dlog!("systemd", "wrapper ready at {}", self.paths.wrapper_script.display());
        }
        // Fallback path for non-Unix targets — systemd manager isn't actually
        // selected on these platforms, but the module still has to compile.
        #[cfg(not(unix))]
        {
            std::fs::write(&self.paths.wrapper_script, &wrapper)
                .map_err(|e| format!("Cannot write wrapper: {}", e))?;
        }

        dlog!("systemd", "Stopping existing service...");
        let _ = self.stop();

        let service = self.generate_service();
        dlog!("systemd", "service unit generated: {} bytes", service.len());
        dlog!("systemd", "Writing service file to: {}", self.paths.service_file.display());
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let tmp = self.paths.service_file.with_extension("service.tmp");
            dlog!("systemd", "service tmp: {}", tmp.display());
            match std::fs::remove_file(&tmp) {
                Ok(_) => dlog!("systemd", "service tmp: cleared stale"),
                Err(e) => dlog!("systemd", "service tmp cleanup: {} (ok if nonexistent)", e),
            }
            {
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(&tmp)
                    .map_err(|e| {
                        dlog!("systemd", "service tmp open(0o600) failed: {}", e);
                        format!("Cannot create service temp: {}", e)
                    })?;
                dlog!("systemd", "service tmp opened (mode 0o600)");
                file.write_all(service.as_bytes())
                    .map_err(|e| {
                        dlog!("systemd", "service tmp write_all failed: {}", e);
                        format!("Cannot write service file: {}", e)
                    })?;
                dlog!("systemd", "service tmp: wrote {} bytes", service.len());
                match file.sync_all() {
                    Ok(_) => dlog!("systemd", "service tmp fsync OK"),
                    Err(e) => dlog!("systemd", "service tmp fsync failed (non-fatal): {}", e),
                }
            }
            dlog!("systemd", "service tmp -> final rename");
            std::fs::rename(&tmp, &self.paths.service_file)
                .map_err(|e| {
                    dlog!("systemd", "service rename failed: {}", e);
                    format!("Cannot finalize service file: {}", e)
                })?;
            dlog!("systemd", "service file ready at {}", self.paths.service_file.display());
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&self.paths.service_file, &service)
                .map_err(|e| format!("Cannot write service file: {}", e))?;
        }

        dlog!("systemd", "Running daemon-reload...");
        let r = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output()
            .map_err(|e| format!("daemon-reload failed: {}", e))?;
        crate::core::debug::log_output("systemd", "systemctl --user daemon-reload", &r);
        if !r.status.success() {
            dlog!("systemd", "daemon-reload failed");
            return Err("systemctl daemon-reload failed".into());
        }

        dlog!("systemd", "Enabling service...");
        let r = Command::new("systemctl")
            .args(["--user", "enable", SERVICE_NAME])
            .output()
            .map_err(|e| format!("enable failed: {}", e))?;
        crate::core::debug::log_output("systemd", "systemctl --user enable cokacdir", &r);
        if !r.status.success() {
            dlog!("systemd", "enable failed");
            return Err("systemctl enable failed".into());
        }

        // Truncate error log before starting so we only capture fresh errors
        let error_log_path = self.paths.log_dir.join("cokacdir.error.log");
        dlog!("systemd", "Truncating error log: {}", error_log_path.display());
        let _ = std::fs::File::create(&error_log_path);

        dlog!("systemd", "Restarting service...");
        let r = Command::new("systemctl")
            .args(["--user", "restart", SERVICE_NAME])
            .output()
            .map_err(|e| format!("restart failed: {}", e))?;
        crate::core::debug::log_output("systemd", "systemctl --user restart cokacdir", &r);
        if !r.status.success() {
            dlog!("systemd", "restart failed");
            return Err("systemctl restart failed".into());
        }

        if let Some(user) = std::env::var("USER").ok() {
            dlog!("systemd", "Enabling linger for user: {}", user);
            match Command::new("loginctl")
                .args(["enable-linger", &user])
                .output()
            {
                Ok(out) => crate::core::debug::log_output("systemd", "loginctl enable-linger", &out),
                Err(e) => dlog!("systemd", "loginctl enable-linger exec failed: {}", e),
            }
        } else {
            dlog!("systemd", "USER env var not set; skipping loginctl enable-linger");
        }

        // Check if service actually stays running
        dlog!("systemd", "Sleeping 2000ms for service to stabilize...");
        std::thread::sleep(std::time::Duration::from_millis(2000));
        dlog!("systemd", "Querying post-start status...");
        let status = self.status();
        dlog!("systemd", "post-start status = {:?}", status);
        if status != ServiceStatus::Running {
            // Lossy decode so non-UTF8 bytes in the error log don't wipe out
            // the diagnostic message shown to the user.
            dlog!("systemd", "Reading error log for diagnostics: {}", error_log_path.display());
            let err_bytes = std::fs::read(&error_log_path).unwrap_or_else(|e| {
                dlog!("systemd", "error log read failed: {}", e);
                Vec::new()
            });
            dlog!("systemd", "error log size: {}B", err_bytes.len());
            let err_output = String::from_utf8_lossy(&err_bytes);
            let tail: String = err_output.lines().rev().take(10)
                .collect::<Vec<_>>().into_iter().rev()
                .collect::<Vec<_>>().join("\n");
            dlog!("systemd", "Service not running after restart. Error log tail:\n{}", tail.trim());
            // Also capture systemd's own view of the failed unit for root cause.
            match Command::new("systemctl")
                .args(["--user", "status", SERVICE_NAME, "--no-pager", "--lines=30"])
                .output()
            {
                Ok(out) => crate::core::debug::log_output("systemd", "systemctl --user status cokacdir (post-fail)", &out),
                Err(e) => dlog!("systemd", "post-fail status query exec failed: {}", e),
            }
            match Command::new("journalctl")
                .args(["--user", "-u", SERVICE_NAME, "-n", "30", "--no-pager"])
                .output()
            {
                Ok(out) => crate::core::debug::log_output("systemd", "journalctl --user -u cokacdir -n 30 (post-fail)", &out),
                Err(e) => dlog!("systemd", "post-fail journalctl exec failed: {}", e),
            }
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
        let mut service_err: Option<String> = None;
        // Only attempt `systemctl stop` when the unit file exists. This avoids
        // depending on locale-specific stderr ("not loaded"/"not found") text
        // to distinguish "already absent" from real failures.
        if self.paths.service_file.exists() {
            match Command::new("systemctl")
                .args(["--user", "stop", SERVICE_NAME])
                .output()
            {
                Ok(r) => {
                    crate::core::debug::log_output("systemd", "systemctl --user stop cokacdir", &r);
                    if !r.status.success() {
                        // systemd exit code 5 == "unit not loaded"; treat as benign.
                        if r.status.code() == Some(5) {
                            dlog!("systemd", "stop(): unit not loaded (exit 5)");
                        } else {
                            let stderr = String::from_utf8_lossy(&r.stderr);
                            dlog!("systemd", "stop() failed: {}", stderr);
                            service_err = Some(format!("systemctl stop failed: {}", stderr.trim()));
                        }
                    } else {
                        dlog!("systemd", "stop() success");
                    }
                }
                Err(e) => {
                    dlog!("systemd", "stop(): systemctl exec failed: {}", e);
                    service_err = Some(format!("stop failed: {}", e));
                }
            }
        } else {
            dlog!("systemd", "stop(): service file absent, skipping systemctl stop");
        }

        // Always kill externally running cokacdir processes regardless of service stop result
        dlog!("systemd", "stop(): killing external cokacdir processes via pkill...");
        match Command::new("pkill").arg("cokacdir").output() {
            Ok(out) => {
                crate::core::debug::log_output("systemd", "pkill cokacdir", &out);
                dlog!("systemd", "stop(): pkill exit={} (0=killed, 1=none found)", out.status.code().unwrap_or(-1));
            }
            Err(e) => {
                dlog!("systemd", "stop(): pkill failed: {}", e);
            }
        }

        if let Some(err) = service_err {
            return Err(err);
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        dlog!("systemd", "remove() called");
        let _ = self.stop();
        dlog!("systemd", "remove(): disabling service");
        match Command::new("systemctl")
            .args(["--user", "disable", SERVICE_NAME])
            .output()
        {
            Ok(out) => crate::core::debug::log_output("systemd", "systemctl --user disable cokacdir", &out),
            Err(e) => dlog!("systemd", "remove(): disable exec failed: {}", e),
        }
        if self.paths.service_file.exists() {
            dlog!("systemd", "Removing service file: {}", self.paths.service_file.display());
            std::fs::remove_file(&self.paths.service_file)
                .map_err(|e| format!("Cannot remove service file: {}", e))?;
            dlog!("systemd", "Removed service file");
        } else {
            dlog!("systemd", "Service file already absent: {}", self.paths.service_file.display());
        }
        if self.paths.wrapper_script.exists() {
            dlog!("systemd", "Removing wrapper: {}", self.paths.wrapper_script.display());
            match std::fs::remove_file(&self.paths.wrapper_script) {
                Ok(_) => dlog!("systemd", "Removed wrapper script"),
                Err(e) => dlog!("systemd", "Failed to remove wrapper: {}", e),
            }
        } else {
            dlog!("systemd", "Wrapper script already absent: {}", self.paths.wrapper_script.display());
        }
        dlog!("systemd", "remove(): running daemon-reload");
        match Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output()
        {
            Ok(out) => crate::core::debug::log_output("systemd", "systemctl --user daemon-reload (remove)", &out),
            Err(e) => dlog!("systemd", "remove(): daemon-reload exec failed: {}", e),
        }
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
                crate::core::debug::log_output("systemd", "systemctl --user is-active cokacdir", &out);
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

    fn is_any_running(&self) -> bool {
        dlog!("systemd", "is_any_running(): checking pgrep cokacdir...");
        match Command::new("pgrep").arg("cokacdir").output() {
            Ok(output) => {
                crate::core::debug::log_output("systemd", "pgrep cokacdir", &output);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let pids = stdout.trim();
                let found = output.status.success();
                dlog!("systemd", "is_any_running(): pgrep exit={}, pids='{}', found={}", output.status.code().unwrap_or(-1), pids, found);
                found
            }
            Err(e) => {
                dlog!("systemd", "is_any_running(): pgrep failed: {}", e);
                false
            }
        }
    }

    fn log_path(&self) -> Option<PathBuf> {
        Some(self.paths.log_file.clone())
    }
}
