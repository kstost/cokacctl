use super::{ServiceManager, ServiceStatus};
use crate::core::platform::ServicePaths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::CommandExt as UnixCommandExt;

const SERVICE_NAME: &str = "cokacdir";

/// Tracks the last-logged "status outcome key" so we don't spam the debug log
/// every 5 s with identical lines while the user session is in a steady state.
/// Only state transitions are logged.
static LAST_STATUS_KEY: Mutex<String> = Mutex::new(String::new());

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirectProcessRecord {
    #[serde(default = "direct_record_schema_version")]
    schema_version: u32,
    pid: u32,
    #[serde(default)]
    process_group_id: Option<u32>,
    #[serde(default)]
    binary_path: String,
    #[serde(default)]
    token_count: usize,
    #[serde(default)]
    started_at_epoch_s: u64,
}

fn direct_record_schema_version() -> u32 {
    1
}

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

    /// Current user ID. Used to derive the standard user-bus path
    /// (`/run/user/$UID/bus`) when the launcher's environment is missing it.
    #[cfg(unix)]
    fn current_uid() -> u32 {
        // Safety: getuid() is async-signal-safe and never fails.
        unsafe { libc::getuid() }
    }

    #[cfg(not(unix))]
    fn current_uid() -> u32 {
        0
    }

    /// XDG runtime directory for the current user.
    ///
    /// Honors `XDG_RUNTIME_DIR` if set, otherwise falls back to the systemd
    /// default at `/run/user/$UID`. Used as the base for deriving the
    /// user-bus socket location and for backfilling child env.
    fn user_runtime_dir() -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", Self::current_uid())))
    }

    /// Filesystem path to the user D-Bus session socket, if it can be
    /// determined as a unix path.
    ///
    /// Resolution order:
    /// 1. `DBUS_SESSION_BUS_ADDRESS` parsed as `unix:path=...` — wins if set.
    ///    For abstract sockets or other transports (tcp, unixexec, etc.)
    ///    we return `None` because the socket has no stat-able path.
    /// 2. `$XDG_RUNTIME_DIR/bus` — systemd default location, derived via
    ///    `user_runtime_dir()`.
    ///
    /// `None` means "cannot precheck"; callers should skip filesystem
    /// existence checks and let `systemctl --user` surface its own error.
    fn user_bus_socket_path() -> Option<PathBuf> {
        if let Some(addr) = std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .ok()
            .filter(|s| !s.is_empty())
        {
            // dbus addresses may be `;`-separated alternatives — take the first.
            let first = addr.split(';').next().unwrap_or(&addr);
            if let Some(rest) = first.strip_prefix("unix:path=") {
                // Per-address parameters are `,`-separated, e.g. `unix:path=/x,guid=...`.
                let path = rest.split(',').next().unwrap_or(rest);
                return Some(PathBuf::from(path));
            }
            // Non-path transport (unix:abstract=, tcp:, unixexec:, etc.) —
            // no filesystem entry to check.
            return None;
        }
        Some(Self::user_runtime_dir().join("bus"))
    }

    fn container_runtime() -> Option<&'static str> {
        if Path::new("/.dockerenv").exists() {
            return Some("Docker");
        }
        if Path::new("/run/.containerenv").exists() {
            return Some("container");
        }

        for cgroup in ["/proc/1/cgroup", "/proc/self/cgroup"] {
            let Ok(content) = std::fs::read_to_string(cgroup) else {
                continue;
            };
            let lower = content.to_ascii_lowercase();
            if lower.contains("docker") {
                return Some("Docker");
            }
            if lower.contains("kubepods") {
                return Some("Kubernetes");
            }
            if lower.contains("containerd") {
                return Some("containerd");
            }
            if lower.contains("podman") || lower.contains("libpod") {
                return Some("Podman");
            }
        }

        None
    }

    fn service_manager_unavailable_short(cause: &str) -> String {
        if let Some(runtime) = Self::container_runtime() {
        format!(
                "service manager unavailable in {} container ({})",
                runtime, cause
            )
        } else {
            format!("service manager unavailable ({})", cause)
        }
    }

    fn service_manager_unavailable_long(cause: &str) -> String {
        if let Some(runtime) = Self::container_runtime() {
            format!(
                "Service registration is unavailable in this {} container ({}).\n\
                 cokacctl's Linux service backend requires systemd user services \
                 via `systemctl --user`.\n\
             \n\
                 Run cokacdir as the container's foreground process or manage it \
                 with the container runtime/host service manager instead.",
                runtime, cause
            )
        } else {
            format!(
                "Service registration is unavailable in this Linux session ({}).\n\
                 cokacctl's Linux service backend requires systemd user services \
                 via `systemctl --user`.",
                cause
        )
    }
    }

    fn systemctl_exec_error_detail(e: &std::io::Error) -> String {
        if e.kind() == std::io::ErrorKind::NotFound {
            "systemctl not found".into()
        } else {
            format!("cannot run systemctl: {}", e)
        }
    }

    fn probe_systemctl() -> Result<Output, String> {
        let out = Command::new("systemctl")
            .arg("--version")
            .output()
            .map_err(|e| Self::systemctl_exec_error_detail(&e))?;
        if out.status.success() {
            Ok(out)
        } else {
            Err(format!(
                "systemctl --version failed ({})",
                Self::output_detail(&out)
            ))
        }
    }

    fn output_mentions_user_bus_failure(output: &Output) -> bool {
        let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        let combined = format!("{}\n{}", stderr, stdout);
        (combined.contains("failed to connect") && combined.contains("bus"))
            || combined.contains("no medium found")
            || (combined.contains("no such file or directory") && combined.contains("bus"))
            || (combined.contains("host is down") && combined.contains("bus"))
    }

    fn service_manager_unavailable_cause() -> Option<String> {
        if let Err(cause) = Self::probe_systemctl() {
            return Some(cause);
        }

        if let Some(bus_path) = Self::user_bus_socket_path() {
            if !bus_path.exists() {
                return Some("user systemd bus unavailable".into());
            }
        }

        match Self::user_systemctl_cmd(["show-environment"]).output() {
            Ok(out) if out.status.success() => None,
            Ok(out) if Self::output_mentions_user_bus_failure(&out) => {
                Some("user systemd bus unavailable".into())
            }
            Ok(out) => Some(format!(
                "systemctl --user unavailable ({})",
                Self::output_detail(&out)
            )),
            Err(e) => Some(Self::systemctl_exec_error_detail(&e)),
        }
    }

    fn direct_status(&self, cause: &str) -> ServiceStatus {
        let reason = Self::service_manager_unavailable_short(cause);
        if self.direct_managed_process_running() {
            ServiceStatus::DirectRunning(reason)
        } else {
            ServiceStatus::DirectStopped(reason)
        }
    }

    fn direct_pid_file(&self) -> PathBuf {
        self.paths.log_dir.join("cokacdir.pid")
    }

    fn read_direct_pid_record(&self) -> Option<DirectProcessRecord> {
        let content = std::fs::read_to_string(self.direct_pid_file()).ok()?;
        if let Ok(record) = serde_json::from_str::<DirectProcessRecord>(&content) {
            if record.pid > 0 {
                return Some(record);
            }
        }

        let mut lines = content.lines();
        let pid = lines.next()?.trim().parse::<u32>().ok()?;
        let binary_path = lines
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_default();
        Some(DirectProcessRecord {
            schema_version: direct_record_schema_version(),
            pid,
            process_group_id: None,
            binary_path,
            token_count: 0,
            started_at_epoch_s: 0,
        })
    }

    fn write_direct_pid_record(&self, record: &DirectProcessRecord) -> Result<(), String> {
        let content = serde_json::to_string_pretty(record)
            .map_err(|e| format!("Cannot serialize direct process metadata: {}", e))?;
        let pid_file = self.direct_pid_file();
        let tmp = pid_file.with_extension("pid.tmp");
        std::fs::write(&tmp, content)
            .map_err(|e| format!("Cannot write direct pid temp file: {}", e))?;
        std::fs::rename(&tmp, &pid_file)
            .map_err(|e| format!("Cannot finalize direct pid file: {}", e))
    }

    fn pid_running(pid: u32) -> bool {
        let proc_dir = format!("/proc/{}", pid);
        if !Path::new(&proc_dir).exists() {
            return false;
        }
        if let Ok(status) = std::fs::read_to_string(format!("{}/status", proc_dir)) {
            if status
                .lines()
                .find(|line| line.starts_with("State:"))
                .map(|line| line.split_whitespace().nth(1) == Some("Z"))
                .unwrap_or(false)
            {
                return false;
            }
        }
        true
    }

    fn process_args(pid: u32) -> Option<Vec<String>> {
        let bytes = std::fs::read(format!("/proc/{}/cmdline", pid)).ok()?;
        let args = bytes
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8_lossy(arg).into_owned())
            .collect::<Vec<_>>();
        if args.is_empty() {
            None
        } else {
            Some(args)
        }
    }

    fn program_name_is_cokacdir(program: &str) -> bool {
        Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name == SERVICE_NAME)
            .unwrap_or(false)
    }

    fn args_match_direct_record(args: &[String], record: &DirectProcessRecord) -> bool {
        if record.binary_path.is_empty() {
            let Some(program) = args.first() else {
                return false;
            };
            if !Self::program_name_is_cokacdir(program) {
                return false;
            }
            return Self::args_have_ccserver_marker_after(args, 0);
        }

        args.iter()
            .position(|arg| arg == &record.binary_path)
            .map(|pos| Self::args_have_ccserver_marker_after(args, pos))
            .unwrap_or(false)
    }

    fn args_have_ccserver_marker_after(args: &[String], program_pos: usize) -> bool {
        args.get(program_pos + 1).map(String::as_str) == Some("--ccserver")
            && args.get(program_pos + 2).map(String::as_str) == Some("--")
    }

    fn pid_matches_direct_record(record: &DirectProcessRecord) -> bool {
        let pid = record.pid;
        if !Self::pid_running(pid) {
            return false;
        }

        if let Some(args) = Self::process_args(pid) {
            return Self::args_match_direct_record(&args, record);
        }

        if !record.binary_path.is_empty() {
            dlog!(
                "systemd",
                "direct pid cmdline unavailable; refusing to verify pid {} with binary metadata",
                pid
            );
            return false;
        }

        if let Some(group_pid) = record.process_group_id {
            if group_pid != pid {
                return false;
            }
        }

        std::fs::read_to_string(format!("/proc/{}/comm", pid))
            .map(|s| s.trim() == SERVICE_NAME)
            .unwrap_or(false)
    }

    fn direct_managed_process_running(&self) -> bool {
        let Some(record) = self.read_direct_pid_record() else {
            return false;
        };
        let running = Self::pid_matches_direct_record(&record);
        if !running {
            dlog!(
                "systemd",
                "direct process metadata is stale or unverified (pid={})",
                record.pid
            );
            let _ = std::fs::remove_file(self.direct_pid_file());
        }
        running
    }

    #[cfg(unix)]
    fn terminate_pid(pid: u32) -> Result<(), String> {
        let pid = pid as libc::pid_t;
        if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
            if !Self::pid_running(pid as u32) {
                return Ok(());
            }
            return Err(format!("cannot terminate direct process pid {}", pid));
        }

        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !Self::pid_running(pid as u32) {
                return Ok(());
            }
        }

        if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
            if !Self::pid_running(pid as u32) {
                return Ok(());
            }
            return Err(format!("cannot kill direct process pid {}", pid));
        }
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !Self::pid_running(pid as u32) {
                return Ok(());
            }
        }
        if Self::pid_running(pid as u32) {
            return Err(format!("direct process pid {} is still alive", pid));
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn terminate_pid(pid: u32) -> Result<(), String> {
        Err(format!("cannot terminate direct process pid {}", pid))
    }

    #[cfg(unix)]
    fn configure_direct_command(command: &mut Command) {
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }

    #[cfg(not(unix))]
    fn configure_direct_command(_command: &mut Command) {}

    #[cfg(unix)]
    fn terminate_process_group(pid: u32) -> Result<(), String> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(format!("invalid direct process group pid {}", pid));
        }

        let pgid = -(pid as libc::pid_t);
        if unsafe { libc::kill(pgid, libc::SIGTERM) } != 0 && Self::pid_running(pid) {
            dlog!(
                "systemd",
                "direct process group SIGTERM failed for pgid {}: {}",
                pgid,
                std::io::Error::last_os_error()
            );
        }

        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !Self::pid_running(pid) {
                return Ok(());
            }
        }

        if unsafe { libc::kill(pgid, libc::SIGKILL) } != 0 && Self::pid_running(pid) {
            return Err(format!(
                "cannot kill direct process group {}: {}",
                pid,
                std::io::Error::last_os_error()
            ));
        }

        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !Self::pid_running(pid) {
                return Ok(());
            }
        }
        if Self::pid_running(pid) {
            return Err(format!("direct process group {} is still alive", pid));
        }

        Ok(())
    }

    #[cfg(not(unix))]
    fn terminate_process_group(pid: u32) -> Result<(), String> {
        Self::terminate_pid(pid)
    }

    fn terminate_direct_record(record: &DirectProcessRecord) -> Result<(), String> {
        if record.process_group_id == Some(record.pid) {
            match Self::terminate_process_group(record.pid) {
                Ok(()) => Ok(()),
                Err(e) if Self::pid_running(record.pid) => {
                    dlog!(
                        "systemd",
                        "direct process group termination failed, falling back to pid: {}",
                        e
                    );
                    Self::terminate_pid(record.pid)
                }
                Err(_) => Ok(()),
            }
        } else {
            Self::terminate_pid(record.pid)
        }
    }

    fn current_epoch_s() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn direct_error_tail(&self) -> String {
        let err_bytes = std::fs::read(&self.paths.error_log_file).unwrap_or_default();
        let err_output = String::from_utf8_lossy(&err_bytes);
        err_output
            .lines()
            .rev()
            .take(10)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn direct_start(
        &self,
        binary_path: &Path,
        tokens: &[String],
        cause: &str,
    ) -> Result<(), String> {
        dlog!(
            "systemd",
            "direct_start(): service manager unavailable: {}",
            cause
        );
        std::fs::create_dir_all(&self.paths.log_dir)
            .map_err(|e| format!("Cannot create log dir: {}", e))?;

        let stop_result = self.direct_stop("pre-start cleanup");
        if Self::cokacdir_process_running(false) {
            return Err(match stop_result {
                Ok(_) => "cokacdir is already running".to_string(),
                Err(e) => format!("Cannot stop existing direct process: {}", e),
            });
        }
        let stdout = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.paths.log_file)
            .map_err(|e| format!("Cannot open direct log: {}", e))?;
        let stderr = std::fs::File::create(&self.paths.error_log_file)
            .map_err(|e| format!("Cannot open direct error log: {}", e))?;

        dlog!(
            "systemd",
            "direct_start(): spawning detached direct process"
        );
        let mut command = Command::new(binary_path);
        command
            .arg("--ccserver")
            .arg("--")
            .args(tokens)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(stdout))
            .stderr(std::process::Stdio::from(stderr));
        Self::configure_direct_command(&mut command);
        let child = command
            .spawn()
            .map_err(|e| format!("direct start failed: {}", e))?;
        let pid = child.id();
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        let record = DirectProcessRecord {
            schema_version: direct_record_schema_version(),
            pid,
            process_group_id: Some(pid),
            binary_path: binary_path.to_string_lossy().to_string(),
            token_count: tokens.len(),
            started_at_epoch_s: Self::current_epoch_s(),
        };
        if let Err(e) = self.write_direct_pid_record(&record) {
            let _ = Self::terminate_process_group(pid);
            return Err(e);
        }

        std::thread::sleep(std::time::Duration::from_millis(500));
        if self.direct_managed_process_running() {
            dlog!(
                "systemd",
                "direct_start(): process is running (pid={})",
                pid
            );
            Ok(())
        } else {
            let _ = Self::terminate_process_group(pid);
            let _ = std::fs::remove_file(self.direct_pid_file());
            let tail = self.direct_error_tail();
            if tail.trim().is_empty() {
                Err("cokacdir direct process did not stay running".into())
            } else {
                Err(tail.trim().to_string())
            }
        }
    }

    fn direct_stop(&self, cause: &str) -> Result<(), String> {
        dlog!("systemd", "direct_stop(): {}", cause);
        if let Some(record) = self.read_direct_pid_record() {
            if Self::pid_matches_direct_record(&record) {
                Self::terminate_direct_record(&record)?;
            } else {
                dlog!(
                    "systemd",
                    "direct_stop(): refusing to terminate unverified pid {}",
                    record.pid
                );
            }
            let _ = std::fs::remove_file(self.direct_pid_file());
        }

        Ok(())
    }

    fn cokacdir_process_running(log: bool) -> bool {
        if log {
            dlog!(
                "systemd",
                "is_any_running(): checking pgrep {}...",
                SERVICE_NAME
            );
        }
        match Command::new("pgrep").args(["-x", SERVICE_NAME]).output() {
            Ok(output) => {
                if log {
                    crate::core::debug::log_output(
                        "systemd",
                        &format!("pgrep -x {}", SERVICE_NAME),
                        &output,
                    );
                }
                let stdout = String::from_utf8_lossy(&output.stdout);
                let pids = stdout.trim();
                let found = output.status.success();
                if log {
                    dlog!(
                        "systemd",
                        "is_any_running(): pgrep exit={}, pids='{}', found={}",
                        output.status.code().unwrap_or(-1),
                        pids,
                        found
                    );
                }
                found
            }
            Err(e) => {
                if log {
                    dlog!("systemd", "is_any_running(): pgrep failed: {}", e);
                }
                false
            }
        }
    }

    /// Backfill `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS` for child
    /// processes that need to reach the user systemd / D-Bus instance.
    ///
    /// cokacctl is sometimes launched from a non-login shell (e.g., a freshly
    /// opened WSL terminal) that lacks these variables. Without them,
    /// `systemctl --user` aborts with "Failed to connect to bus: No medium
    /// found" even though the user manager is up. We backfill from the
    /// resolved runtime dir only when the parent env doesn't already supply
    /// the variable, so an explicit setting always wins.
    fn apply_user_env(cmd: &mut Command) {
        let runtime_dir = Self::user_runtime_dir();
        if std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .is_none()
        {
            cmd.env("XDG_RUNTIME_DIR", &runtime_dir);
        }
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS")
            .filter(|v| !v.is_empty())
            .is_none()
        {
            cmd.env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", runtime_dir.join("bus").display()),
            );
        }
    }

    /// Build a `systemctl --user <args>` command with the user-session env
    /// variables ensured (see `apply_user_env`).
    fn user_systemctl_cmd<I, S>(args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut cmd = Command::new("systemctl");
        cmd.arg("--user").args(args);
        Self::apply_user_env(&mut cmd);
        cmd
    }

    /// Build a `journalctl --user <args>` command with the user-session env
    /// variables ensured.
    fn user_journalctl_cmd<I, S>(args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut cmd = Command::new("journalctl");
        cmd.arg("--user").args(args);
        Self::apply_user_env(&mut cmd);
        cmd
    }

    /// Returns true and updates the cache when `key` differs from the last
    /// logged status key. Used to suppress repeated identical lines from the
    /// 5-second status poll.
    fn status_key_changed(key: &str) -> bool {
        // Recover from a poisoned lock (would mean another thread panicked
        // while holding it) rather than propagating the panic — the cached
        // value is purely a dedup hint, not correctness-critical.
        let mut last = LAST_STATUS_KEY.lock().unwrap_or_else(|e| e.into_inner());
        if *last != key {
            *last = key.to_string();
            true
        } else {
            false
        }
    }

    fn log_status_transition(key: &str, msg: &str) {
        if Self::status_key_changed(key) {
            crate::core::debug::log("systemd", msg);
        }
    }

    fn escape_systemd_arg(s: &str) -> String {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "$$")
            .replace('%', "%%");
        format!("\"{}\"", escaped)
    }

    fn output_detail(output: &Output) -> String {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "no output".to_string()
        };

        match output.status.code() {
            Some(code) => format!("exit {}: {}", code, detail),
            None => format!("terminated by signal: {}", detail),
        }
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
        let output = Command::new("systemctl").arg("--version").output().ok();
        match output {
            Some(out) => {
                crate::core::debug::log_output("systemd", "systemctl --version", &out);
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let ver = stdout
                        .lines()
                        .next()
                        .and_then(|line| {
                            line.split_whitespace().find_map(|w| w.parse::<u32>().ok())
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
        let log_dir = self
            .paths
            .log_dir
            .to_string_lossy()
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
        dlog!(
            "systemd",
            "start() called - binary: {}, tokens: {}",
            binary_path.display(),
            tokens.len()
        );
        dlog!(
            "systemd",
            "paths: log_dir={}, wrapper={}, service={}, log_file={}",
            self.paths.log_dir.display(),
            self.paths.wrapper_script.display(),
            self.paths.service_file.display(),
            self.paths.log_file.display()
        );

        // Pre-flight: verify binary exists and is executable so we fail early
        // with a clear message instead of relying on systemd's error reporting.
        match std::fs::metadata(binary_path) {
            Ok(m) => dlog!(
                "systemd",
                "binary metadata: is_file={}, len={}B, readonly={}",
                m.is_file(),
                m.len(),
                m.permissions().readonly()
            ),
            Err(e) => dlog!("systemd", "binary metadata probe failed: {}", e),
        }

        dlog!("systemd", "start(): checking service manager availability");
        if let Some(cause) = Self::service_manager_unavailable_cause() {
                    dlog!(
                        "systemd",
                "start(): service manager unavailable, using direct mode: {}",
                cause
                    );
            return self.direct_start(binary_path, tokens, &cause);
            }

                dlog!(
                    "systemd",
            "Creating log dir: {}",
            self.paths.log_dir.display()
                );
        std::fs::create_dir_all(&self.paths.log_dir).map_err(|e| {
                dlog!("systemd", "create_dir_all(log_dir) failed: {}", e);
                format!("Cannot create log dir: {}", e)
            })?;
        dlog!("systemd", "log dir ready");
        if let Some(parent) = self.paths.service_file.parent() {
            dlog!("systemd", "Creating systemd unit dir: {}", parent.display());
            std::fs::create_dir_all(parent).map_err(|e| {
                    dlog!("systemd", "create_dir_all(unit) failed: {}", e);
                    format!("Cannot create systemd dir: {}", e)
                })?;
            dlog!("systemd", "systemd unit dir ready");
        }

        let wrapper = Self::generate_wrapper(binary_path, tokens);
        dlog!("systemd", "wrapper generated: {} bytes", wrapper.len());
        dlog!(
            "systemd",
            "Writing wrapper to: {}",
            self.paths.wrapper_script.display()
        );
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
                file.write_all(wrapper.as_bytes()).map_err(|e| {
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
            std::fs::rename(&tmp, &self.paths.wrapper_script).map_err(|e| {
                    dlog!("systemd", "wrapper rename failed: {}", e);
                    format!("Cannot finalize wrapper: {}", e)
                })?;
            dlog!(
                "systemd",
                "wrapper ready at {}",
                self.paths.wrapper_script.display()
            );
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
        dlog!(
            "systemd",
            "Writing service file to: {}",
            self.paths.service_file.display()
        );
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
                file.write_all(service.as_bytes()).map_err(|e| {
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
            std::fs::rename(&tmp, &self.paths.service_file).map_err(|e| {
                    dlog!("systemd", "service rename failed: {}", e);
                    format!("Cannot finalize service file: {}", e)
                })?;
            dlog!(
                "systemd",
                "service file ready at {}",
                self.paths.service_file.display()
            );
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&self.paths.service_file, &service)
                .map_err(|e| format!("Cannot write service file: {}", e))?;
        }

        dlog!("systemd", "Running daemon-reload...");
        let r = Self::user_systemctl_cmd(["daemon-reload"])
            .output()
            .map_err(|e| format!("daemon-reload failed: {}", e))?;
        crate::core::debug::log_output("systemd", "systemctl --user daemon-reload", &r);
        if !r.status.success() {
            dlog!("systemd", "daemon-reload failed");
            return Err(format!(
                "systemctl --user daemon-reload failed ({})",
                Self::output_detail(&r)
            ));
        }

        dlog!("systemd", "Enabling service...");
        let r = Self::user_systemctl_cmd(["enable", SERVICE_NAME])
            .output()
            .map_err(|e| format!("enable failed: {}", e))?;
        crate::core::debug::log_output("systemd", "systemctl --user enable cokacdir", &r);
        if !r.status.success() {
            dlog!("systemd", "enable failed");
            return Err(format!(
                "systemctl --user enable failed ({})",
                Self::output_detail(&r)
            ));
        }

        // Truncate error log before starting so we only capture fresh errors
        let error_log_path = self.paths.log_dir.join("cokacdir.error.log");
        dlog!(
            "systemd",
            "Truncating error log: {}",
            error_log_path.display()
        );
        let _ = std::fs::File::create(&error_log_path);

        dlog!("systemd", "Restarting service...");
        let r = Self::user_systemctl_cmd(["restart", SERVICE_NAME])
            .output()
            .map_err(|e| format!("restart failed: {}", e))?;
        crate::core::debug::log_output("systemd", "systemctl --user restart cokacdir", &r);
        if !r.status.success() {
            dlog!("systemd", "restart failed");
            return Err(format!(
                "systemctl --user restart failed ({})",
                Self::output_detail(&r)
            ));
        }

        if let Ok(user) = std::env::var("USER") {
            dlog!("systemd", "Enabling linger for user: {}", user);
            match Command::new("loginctl")
                .args(["enable-linger", &user])
                .output()
            {
                Ok(out) => {
                    crate::core::debug::log_output("systemd", "loginctl enable-linger", &out)
                }
                Err(e) => dlog!("systemd", "loginctl enable-linger exec failed: {}", e),
            }
        } else {
            dlog!(
                "systemd",
                "USER env var not set; skipping loginctl enable-linger"
            );
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
            dlog!(
                "systemd",
                "Reading error log for diagnostics: {}",
                error_log_path.display()
            );
            let err_bytes = std::fs::read(&error_log_path).unwrap_or_else(|e| {
                dlog!("systemd", "error log read failed: {}", e);
                Vec::new()
            });
            dlog!("systemd", "error log size: {}B", err_bytes.len());
            let err_output = String::from_utf8_lossy(&err_bytes);
            let tail: String = err_output
                .lines()
                .rev()
                .take(10)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            dlog!(
                "systemd",
                "Service not running after restart. Error log tail:\n{}",
                tail.trim()
            );
            // Also capture systemd's own view of the failed unit for root cause.
            match Self::user_systemctl_cmd(["status", SERVICE_NAME, "--no-pager", "--lines=30"])
                .output()
            {
                Ok(out) => crate::core::debug::log_output(
                    "systemd",
                    "systemctl --user status cokacdir (post-fail)",
                    &out,
                ),
                Err(e) => dlog!("systemd", "post-fail status query exec failed: {}", e),
            }
            match Self::user_journalctl_cmd(["-u", SERVICE_NAME, "-n", "30", "--no-pager"]).output()
            {
                Ok(out) => crate::core::debug::log_output(
                    "systemd",
                    "journalctl --user -u cokacdir -n 30 (post-fail)",
                    &out,
                ),
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
        if let Some(cause) = Self::service_manager_unavailable_cause() {
            return self.direct_stop(&cause);
        }

        let mut service_err: Option<String> = None;
        // Only attempt `systemctl stop` when the unit file exists. This avoids
        // depending on locale-specific stderr ("not loaded"/"not found") text
        // to distinguish "already absent" from real failures.
        if self.paths.service_file.exists() {
            match Self::user_systemctl_cmd(["stop", SERVICE_NAME]).output() {
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
            dlog!(
                "systemd",
                "stop(): service file absent, skipping systemctl stop"
            );
        }

        // Always kill externally running cokacdir processes regardless of service stop result
        dlog!(
            "systemd",
            "stop(): killing external {} processes via pkill...",
            SERVICE_NAME
        );
        match Command::new("pkill").arg(SERVICE_NAME).output() {
            Ok(out) => {
                crate::core::debug::log_output("systemd", &format!("pkill {}", SERVICE_NAME), &out);
                dlog!(
                    "systemd",
                    "stop(): pkill exit={} (0=killed, 1=none found)",
                    out.status.code().unwrap_or(-1)
                );
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
        if let Some(cause) = Self::service_manager_unavailable_cause() {
            return Err(Self::service_manager_unavailable_long(&cause));
        }

        let _ = self.stop();
        dlog!("systemd", "remove(): disabling service");
        match Self::user_systemctl_cmd(["disable", SERVICE_NAME]).output() {
            Ok(out) => {
                crate::core::debug::log_output("systemd", "systemctl --user disable cokacdir", &out)
            }
            Err(e) => dlog!("systemd", "remove(): disable exec failed: {}", e),
        }
        if self.paths.service_file.exists() {
            dlog!(
                "systemd",
                "Removing service file: {}",
                self.paths.service_file.display()
            );
            std::fs::remove_file(&self.paths.service_file)
                .map_err(|e| format!("Cannot remove service file: {}", e))?;
            dlog!("systemd", "Removed service file");
        } else {
            dlog!(
                "systemd",
                "Service file already absent: {}",
                self.paths.service_file.display()
            );
        }
        if self.paths.wrapper_script.exists() {
            dlog!(
                "systemd",
                "Removing wrapper: {}",
                self.paths.wrapper_script.display()
            );
            match std::fs::remove_file(&self.paths.wrapper_script) {
                Ok(_) => dlog!("systemd", "Removed wrapper script"),
                Err(e) => dlog!("systemd", "Failed to remove wrapper: {}", e),
            }
        } else {
            dlog!(
                "systemd",
                "Wrapper script already absent: {}",
                self.paths.wrapper_script.display()
            );
        }
        dlog!("systemd", "remove(): running daemon-reload");
        match Self::user_systemctl_cmd(["daemon-reload"]).output() {
            Ok(out) => crate::core::debug::log_output(
                "systemd",
                "systemctl --user daemon-reload (remove)",
                &out,
            ),
            Err(e) => dlog!("systemd", "remove(): daemon-reload exec failed: {}", e),
        }
        dlog!("systemd", "remove() complete");
        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        // Compute outcome silently, then log only on state transitions so the
        // 5-second poll doesn't fill the debug log with identical lines.
        if let Some(cause) = Self::service_manager_unavailable_cause() {
            let result = self.direct_status(&cause);
            let key = format!("direct_mode:{:?}", result);
            if Self::status_key_changed(&key) {
                crate::core::debug::log(
                    "systemd",
                    &format!(
                        "status(): service manager unavailable: {} -> {:?}",
                        cause, result
                    ),
                );
            }
            return result;
        }

        // Same precheck as start(): if the user D-Bus socket is missing,
        // every `systemctl --user` call below would just return the cryptic
        // "Failed to connect to bus: No medium found". Surface a clearer
        // line in the dashboard instead. (For non-path transports we can't
        // precheck, so we fall through to the actual systemctl call.)
        if let Some(bus_path) = Self::user_bus_socket_path() {
            if !bus_path.exists() {
                Self::log_status_transition(
                    "bus_missing",
                    &format!("status(): user bus socket missing ({})", bus_path.display()),
                );
                return self.direct_status("user systemd bus unavailable");
            }
        }
        if !self.paths.service_file.exists() {
            Self::log_status_transition(
                "not_installed",
                "status(): service file not found -> NotInstalled",
            );
            return ServiceStatus::NotInstalled;
        }
        let output = Self::user_systemctl_cmd(["is-active", SERVICE_NAME]).output();
        match output {
            Ok(out) => {
                let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let result = if Self::output_mentions_user_bus_failure(&out) {
                    self.direct_status("user systemd bus unavailable")
                } else {
                    match state.as_str() {
                    "active" => ServiceStatus::Running,
                    "inactive" | "failed" => ServiceStatus::Stopped,
                    _ => ServiceStatus::Unknown(Self::output_detail(&out)),
                    }
                };
                let key = format!("{:?}", result);
                if Self::status_key_changed(&key) {
                    crate::core::debug::log_output(
                        "systemd",
                        "systemctl --user is-active cokacdir",
                        &out,
                    );
                    dlog!("systemd", "status(): is-active='{}' -> {:?}", state, result);
                }
                result
            }
            Err(e) => {
                Self::log_status_transition(
                    "exec_failed",
                    &format!("status() query failed: {}", e),
                );
                self.direct_status(&Self::systemctl_exec_error_detail(&e))
            }
        }
    }

    fn is_any_running(&self) -> bool {
        Self::cokacdir_process_running(true)
    }

    fn log_path(&self) -> Option<PathBuf> {
        Some(self.paths.log_file.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct_record(binary_path: &str) -> DirectProcessRecord {
        DirectProcessRecord {
            schema_version: direct_record_schema_version(),
            pid: 1234,
            process_group_id: Some(1234),
            binary_path: binary_path.to_string(),
            token_count: 2,
            started_at_epoch_s: 1,
        }
    }

    #[test]
    fn direct_record_identity_requires_binary_and_ccserver_args() {
        let record = direct_record("/opt/bin/cokacdir");
        let args = vec![
            "/opt/bin/cokacdir".to_string(),
            "--ccserver".to_string(),
            "--".to_string(),
            "token-a".to_string(),
        ];

        assert!(SystemdManager::args_match_direct_record(&args, &record));
    }

    #[test]
    fn direct_record_identity_rejects_reused_pid_binary_mismatch() {
        let record = direct_record("/opt/bin/cokacdir");
        let args = vec![
            "/tmp/other/cokacdir".to_string(),
            "--ccserver".to_string(),
            "--".to_string(),
        ];

        assert!(!SystemdManager::args_match_direct_record(&args, &record));
    }

    #[test]
    fn direct_record_identity_accepts_interpreter_script_cmdline() {
        let record = direct_record("/tmp/bin/cokacdir");
        let args = vec![
            "/usr/bin/bash".to_string(),
            "/tmp/bin/cokacdir".to_string(),
            "--ccserver".to_string(),
            "--".to_string(),
            "token-a".to_string(),
        ];

        assert!(SystemdManager::args_match_direct_record(&args, &record));
    }

    #[test]
    fn legacy_direct_record_requires_cokacdir_ccserver_shape() {
        let mut record = direct_record("");
        record.process_group_id = None;
        let args = vec![
            "/usr/local/bin/cokacdir".to_string(),
            "--ccserver".to_string(),
            "--".to_string(),
        ];
        let wrong_args = vec!["/usr/local/bin/cokacdir".to_string(), "--help".to_string()];

        assert!(SystemdManager::args_match_direct_record(&args, &record));
        assert!(!SystemdManager::args_match_direct_record(
            &wrong_args,
            &record
        ));
    }
}
