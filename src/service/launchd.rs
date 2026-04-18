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
        dlog!("launchd", "paths: log_dir={}, wrapper={}, plist={}, log_file={}",
            self.paths.log_dir.display(),
            self.paths.wrapper_script.display(),
            self.paths.service_file.display(),
            self.paths.log_file.display());

        // Pre-flight: verify binary exists and is executable so we fail early
        // with a clear message instead of letting launchd spawn-then-exit.
        match std::fs::metadata(binary_path) {
            Ok(m) => dlog!("launchd", "binary metadata: is_file={}, len={}B, readonly={}",
                m.is_file(), m.len(), m.permissions().readonly()),
            Err(e) => dlog!("launchd", "binary metadata probe failed: {}", e),
        }

        dlog!("launchd", "Creating log dir: {}", self.paths.log_dir.display());
        std::fs::create_dir_all(&self.paths.log_dir)
            .map_err(|e| {
                dlog!("launchd", "create_dir_all(log_dir) failed: {}", e);
                format!("Cannot create log dir: {}", e)
            })?;
        dlog!("launchd", "log dir ready");

        if let Some(parent) = self.paths.service_file.parent() {
            dlog!("launchd", "Creating LaunchAgents dir: {}", parent.display());
            std::fs::create_dir_all(parent)
                .map_err(|e| {
                    dlog!("launchd", "create_dir_all(LaunchAgents) failed: {}", e);
                    format!("Cannot create LaunchAgents dir: {}", e)
                })?;
            dlog!("launchd", "LaunchAgents dir ready");
        }

        let wrapper = Self::generate_wrapper(binary_path, tokens);
        dlog!("launchd", "wrapper generated: {} bytes", wrapper.len());
        dlog!("launchd", "Writing wrapper to: {}", self.paths.wrapper_script.display());
        // Atomic write with restricted mode from creation so tokens are never
        // briefly visible under the default umask (0644).
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let tmp = self.paths.wrapper_script.with_extension("sh.tmp");
            dlog!("launchd", "wrapper tmp: {}", tmp.display());
            match std::fs::remove_file(&tmp) {
                Ok(_) => dlog!("launchd", "wrapper tmp: cleared stale"),
                Err(e) => dlog!("launchd", "wrapper tmp cleanup: {} (ok if nonexistent)", e),
            }
            {
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o700)
                    .open(&tmp)
                    .map_err(|e| {
                        dlog!("launchd", "wrapper tmp open(0o700) failed: {}", e);
                        format!("Cannot create wrapper temp: {}", e)
                    })?;
                dlog!("launchd", "wrapper tmp opened (mode 0o700)");
                file.write_all(wrapper.as_bytes())
                    .map_err(|e| {
                        dlog!("launchd", "wrapper tmp write_all failed: {}", e);
                        format!("Cannot write wrapper: {}", e)
                    })?;
                dlog!("launchd", "wrapper tmp: wrote {} bytes", wrapper.len());
                match file.sync_all() {
                    Ok(_) => dlog!("launchd", "wrapper tmp fsync OK"),
                    Err(e) => dlog!("launchd", "wrapper tmp fsync failed (non-fatal): {}", e),
                }
            }
            dlog!("launchd", "wrapper tmp -> final rename");
            std::fs::rename(&tmp, &self.paths.wrapper_script)
                .map_err(|e| {
                    dlog!("launchd", "wrapper rename failed: {}", e);
                    format!("Cannot finalize wrapper: {}", e)
                })?;
            dlog!("launchd", "wrapper ready at {}", self.paths.wrapper_script.display());
        }
        // Fallback for non-Unix targets — launchd manager isn't selected on
        // these platforms but the module must still compile.
        #[cfg(not(unix))]
        {
            std::fs::write(&self.paths.wrapper_script, &wrapper)
                .map_err(|e| format!("Cannot write wrapper: {}", e))?;
        }

        dlog!("launchd", "Stopping existing service...");
        let _ = self.stop();

        let plist = self.generate_plist(&self.paths.wrapper_script);
        dlog!("launchd", "plist generated: {} bytes", plist.len());
        dlog!("launchd", "Writing plist to: {}", self.paths.service_file.display());
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let tmp = self.paths.service_file.with_extension("plist.tmp");
            dlog!("launchd", "plist tmp: {}", tmp.display());
            match std::fs::remove_file(&tmp) {
                Ok(_) => dlog!("launchd", "plist tmp: cleared stale"),
                Err(e) => dlog!("launchd", "plist tmp cleanup: {} (ok if nonexistent)", e),
            }
            {
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(&tmp)
                    .map_err(|e| {
                        dlog!("launchd", "plist tmp open(0o600) failed: {}", e);
                        format!("Cannot create plist temp: {}", e)
                    })?;
                dlog!("launchd", "plist tmp opened (mode 0o600)");
                file.write_all(plist.as_bytes())
                    .map_err(|e| {
                        dlog!("launchd", "plist tmp write_all failed: {}", e);
                        format!("Cannot write plist: {}", e)
                    })?;
                dlog!("launchd", "plist tmp: wrote {} bytes", plist.len());
                match file.sync_all() {
                    Ok(_) => dlog!("launchd", "plist tmp fsync OK"),
                    Err(e) => dlog!("launchd", "plist tmp fsync failed (non-fatal): {}", e),
                }
            }
            dlog!("launchd", "plist tmp -> final rename");
            std::fs::rename(&tmp, &self.paths.service_file)
                .map_err(|e| {
                    dlog!("launchd", "plist rename failed: {}", e);
                    format!("Cannot finalize plist: {}", e)
                })?;
            dlog!("launchd", "plist ready at {}", self.paths.service_file.display());
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&self.paths.service_file, &plist)
                .map_err(|e| format!("Cannot write plist: {}", e))?;
        }

        let domain = Self::domain();
        dlog!("launchd", "Enabling service in domain: {}", domain);
        match Command::new("launchctl")
            .args(["enable", &format!("{}/{}", domain, LABEL)])
            .output()
        {
            Ok(out) => crate::core::debug::log_output("launchd", "launchctl enable", &out),
            Err(e) => dlog!("launchd", "launchctl enable exec failed: {}", e),
        }

        // Truncate error log before starting so we only capture fresh errors
        let error_log_path = self.paths.log_dir.join("cokacdir.error.log");
        dlog!("launchd", "Truncating error log: {}", error_log_path.display());
        let _ = std::fs::File::create(&error_log_path);

        dlog!("launchd", "Bootstrapping service...");
        let result = Command::new("launchctl")
            .args([
                "bootstrap",
                &domain,
                &self.paths.service_file.to_string_lossy(),
            ])
            .output()
            .map_err(|e| format!("launchctl bootstrap failed: {}", e))?;
        crate::core::debug::log_output("launchd", "launchctl bootstrap", &result);

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            dlog!("launchd", "Bootstrap failed: {}", stderr);
            return Err(format!("launchctl bootstrap failed: {}", stderr));
        }

        // Check if service actually stays running
        dlog!("launchd", "Sleeping 2000ms for service to stabilize...");
        std::thread::sleep(std::time::Duration::from_millis(2000));
        dlog!("launchd", "Querying post-start status...");
        let status = self.status();
        dlog!("launchd", "post-start status = {:?}", status);
        if status != ServiceStatus::Running {
            // Lossy decode so non-UTF8 bytes in the error log don't wipe out
            // the diagnostic message shown to the user.
            dlog!("launchd", "Reading error log for diagnostics: {}", error_log_path.display());
            let err_bytes = std::fs::read(&error_log_path).unwrap_or_else(|e| {
                dlog!("launchd", "error log read failed: {}", e);
                Vec::new()
            });
            dlog!("launchd", "error log size: {}B", err_bytes.len());
            let err_output = String::from_utf8_lossy(&err_bytes);
            let tail: String = err_output.lines().rev().take(10)
                .collect::<Vec<_>>().into_iter().rev()
                .collect::<Vec<_>>().join("\n");
            dlog!("launchd", "Service not running after bootstrap. Error log tail:\n{}", tail.trim());
            // Also probe with launchctl print for full service snapshot
            let target = format!("{}/{}", Self::domain(), LABEL);
            if let Ok(out) = Command::new("launchctl").args(["print", &target]).output() {
                crate::core::debug::log_output("launchd", "launchctl print (post-fail snapshot)", &out);
            }
            if !tail.trim().is_empty() {
                return Err(tail.trim().to_string());
            }
            return Err("Service started but exited immediately".into());
        }

        dlog!("launchd", "start() completed successfully");
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        dlog!("launchd", "stop() called");
        let domain = Self::domain();
        let target = format!("{}/{}", domain, LABEL);
        let mut service_err: Option<String> = None;
        // Only attempt bootout when the service is actually known to launchd.
        // `launchctl print` returns non-zero when the service is absent; this
        // avoids depending on locale-specific stderr matching.
        dlog!("launchd", "stop(): probing load state via launchctl print {}", target);
        let is_loaded = match Command::new("launchctl")
            .args(["print", &target])
            .output()
        {
            Ok(out) => {
                crate::core::debug::log_output("launchd", "launchctl print (probe)", &out);
                out.status.success()
            }
            Err(e) => {
                dlog!("launchd", "stop(): launchctl print exec failed: {}", e);
                false
            }
        };
        if is_loaded {
            match Command::new("launchctl")
                .args(["bootout", &target])
                .output()
            {
                Ok(result) => {
                    crate::core::debug::log_output("launchd", "launchctl bootout", &result);
                    if !result.status.success() {
                        let stderr = String::from_utf8_lossy(&result.stderr);
                        dlog!("launchd", "stop() failed: {}", stderr);
                        service_err = Some(format!("launchctl bootout failed: {}", stderr.trim()));
                    } else {
                        dlog!("launchd", "stop() success");
                    }
                }
                Err(e) => {
                    dlog!("launchd", "stop(): launchctl exec failed: {}", e);
                    service_err = Some(format!("launchctl bootout failed: {}", e));
                }
            }
        } else {
            dlog!("launchd", "stop(): service not loaded, skipping bootout");
        }

        // Always kill externally running cokacdir processes regardless of service stop result
        dlog!("launchd", "stop(): killing external cokacdir processes via pkill...");
        match Command::new("pkill").arg("cokacdir").output() {
            Ok(out) => {
                crate::core::debug::log_output("launchd", "pkill cokacdir", &out);
                dlog!("launchd", "stop(): pkill exit={} (0=killed, 1=none found)", out.status.code().unwrap_or(-1));
            }
            Err(e) => {
                dlog!("launchd", "stop(): pkill failed: {}", e);
            }
        }

        if let Some(err) = service_err {
            return Err(err);
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
        // `launchctl print <target>` gives a structured "state = running"
        // / "state = not running" line and its exit code distinguishes
        // loaded-but-no-PID from actually running. This is more accurate than
        // a substring check over `launchctl list` output.
        let target = format!("{}/{}", Self::domain(), LABEL);
        match Command::new("launchctl").args(["print", &target]).output() {
            Ok(out) if out.status.success() => {
                crate::core::debug::log_output("launchd", "launchctl print (status)", &out);
                let stdout = String::from_utf8_lossy(&out.stdout);
                let mut has_pid = false;
                let mut state_running = false;
                for line in stdout.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("pid = ") {
                        if rest.trim().parse::<u32>().is_ok() {
                            has_pid = true;
                        }
                    }
                    if trimmed == "state = running" {
                        state_running = true;
                    }
                }
                if state_running && has_pid {
                    dlog!("launchd", "status(): Running (pid + state=running)");
                    ServiceStatus::Running
                } else {
                    dlog!(
                        "launchd",
                        "status(): Stopped (loaded but no live pid; state_running={})",
                        state_running
                    );
                    ServiceStatus::Stopped
                }
            }
            Ok(out) => {
                crate::core::debug::log_output("launchd", "launchctl print (status, non-success)", &out);
                dlog!("launchd", "status(): launchctl print reports not loaded -> Stopped");
                ServiceStatus::Stopped
            }
            Err(e) => {
                dlog!("launchd", "status() query failed: {}", e);
                ServiceStatus::Unknown("Cannot query launchctl".into())
            }
        }
    }

    fn is_any_running(&self) -> bool {
        dlog!("launchd", "is_any_running(): checking pgrep cokacdir...");
        match Command::new("pgrep").arg("cokacdir").output() {
            Ok(output) => {
                crate::core::debug::log_output("launchd", "pgrep cokacdir", &output);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let pids = stdout.trim();
                let found = output.status.success();
                dlog!("launchd", "is_any_running(): pgrep exit={}, pids='{}', found={}", output.status.code().unwrap_or(-1), pids, found);
                found
            }
            Err(e) => {
                dlog!("launchd", "is_any_running(): pgrep failed: {}", e);
                false
            }
        }
    }

    fn log_path(&self) -> Option<PathBuf> {
        Some(self.paths.log_file.clone())
    }
}
