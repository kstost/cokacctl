use super::{ServiceManager, ServiceStatus};
use crate::core::debug::decode_output;
use crate::core::platform::{ServicePaths, WindowsServiceState};
use std::path::{Path, PathBuf};
use std::process::Command;

const TASK_NAME: &str = "cokacdir";

pub struct TaskSchedulerManager {
    paths: ServicePaths,
}

impl TaskSchedulerManager {
    pub fn new() -> Self {
        dlog!("taskscheduler", "TaskSchedulerManager created");
        TaskSchedulerManager {
            paths: ServicePaths::for_current_os(),
        }
    }

    fn escape_bat_arg(s: &str) -> String {
        let escaped = s
            .replace('^', "^^")
            .replace('&', "^&")
            .replace('|', "^|")
            .replace('<', "^<")
            .replace('>', "^>")
            .replace('%', "%%");
        format!("\"{}\"", escaped)
    }

    fn generate_wrapper(binary_path: &Path, tokens: &[String], paths: &ServicePaths) -> String {
        let args: Vec<String> = tokens.iter().map(|t| Self::escape_bat_arg(t)).collect();
        format!(
            "@echo off\r\n{exe} --ccserver -- {args} >> \"{log}\" 2>> \"{elog}\"\r\n",
            exe = Self::escape_bat_arg(&binary_path.to_string_lossy()),
            args = args.join(" "),
            log = paths.log_file.to_string_lossy(),
            elog = paths.error_log_file.to_string_lossy(),
        )
    }

    /// Create a Command with CREATE_NO_WINDOW flag on Windows
    /// to prevent console windows from flashing during TUI operation.
    fn cmd<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
        let mut cmd = Command::new(program);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        cmd
    }

    fn powershell(script: &str) -> Result<std::process::Output, String> {
        dlog!("taskscheduler", "PowerShell: {}", script);
        Self::cmd("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
            .map_err(|e| format!("PowerShell execution failed: {}", e))
    }

    fn service_state(&self, binary_path: &Path, tokens: &[String]) -> WindowsServiceState {
        WindowsServiceState {
            schema_version: 1,
            task_name: TASK_NAME.to_string(),
            wrapper_script: self.paths.wrapper_script.to_string_lossy().to_string(),
            binary_path: binary_path.to_string_lossy().to_string(),
            token_count: tokens.len(),
        }
    }

    fn write_state_file(&self, state: &WindowsServiceState) -> Result<(), String> {
        if let Some(parent) = self.paths.state_file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create state dir: {}", e))?;
        }
        let content = serde_json::to_string_pretty(state)
            .map_err(|e| format!("Cannot serialize Windows service state: {}", e))?;
        std::fs::write(&self.paths.state_file, content)
            .map_err(|e| format!("Cannot write Windows service state: {}", e))
    }

    fn remove_state_file(&self) -> Result<(), String> {
        if self.paths.state_file.exists() {
            std::fs::remove_file(&self.paths.state_file)
                .map_err(|e| format!("Cannot remove Windows service state: {}", e))?;
        }
        Ok(())
    }

    fn clear_legacy_pid_file(&self) {
        let legacy_pid = self.paths.log_dir.join("cokacdir.pid");
        if legacy_pid.exists() {
            dlog!("taskscheduler", "Removing legacy PID file: {}", legacy_pid.display());
            let _ = std::fs::remove_file(legacy_pid);
        }
    }

    fn query_task_state(&self) -> Result<Option<String>, String> {
        let script = format!(
            "$task = Get-ScheduledTask -TaskName '{name}' -ErrorAction SilentlyContinue\n\
             if (-not $task) {{ Write-Output '__MISSING__'; exit 0 }}\n\
             Write-Output $task.State",
            name = TASK_NAME,
        );
        let output = Self::powershell(&script)?;
        let stdout = decode_output(&output.stdout);
        let stderr = decode_output(&output.stderr);
        dlog!("taskscheduler", "query_task_state exit={}, stdout='{}', stderr='{}'",
            output.status, stdout.trim(), stderr.trim());
        if !output.status.success() {
            return Err(format!("Cannot query Task Scheduler state: {}", stderr.trim()));
        }
        let state = stdout
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("__MISSING__");
        if state == "__MISSING__" {
            Ok(None)
        } else {
            Ok(Some(state.to_string()))
        }
    }

    fn task_exists(&self) -> Result<bool, String> {
        self.query_task_state().map(|state| state.is_some())
    }

    fn stop_task_if_present(&self) -> Result<(), String> {
        match self.query_task_state()? {
            None => {
                dlog!("taskscheduler", "stop_task_if_present: task missing");
                return Ok(());
            }
            Some(state) if state != "Running" => {
                dlog!("taskscheduler", "stop_task_if_present: task present but state={}, nothing to stop", state);
                return Ok(());
            }
            Some(_) => {}
        }
        let output = Self::powershell(&format!(
            "Stop-ScheduledTask -TaskName '{}'", TASK_NAME
        ))?;
        let stderr = decode_output(&output.stderr);
        dlog!("taskscheduler", "stop_task_if_present exit={}, stderr='{}'", output.status, stderr.trim());
        if !output.status.success() {
            return Err(format!("Task stop failed: {}", stderr.trim()));
        }
        Ok(())
    }

    fn delete_task_if_present(&self) -> Result<(), String> {
        if !self.task_exists()? {
            dlog!("taskscheduler", "delete_task_if_present: task missing");
            return Ok(());
        }
        let output = Self::cmd("schtasks")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .output()
            .map_err(|e| format!("Task deletion failed: {}", e))?;
        let stdout = decode_output(&output.stdout);
        let stderr = decode_output(&output.stderr);
        dlog!("taskscheduler", "delete_task_if_present exit={}, stdout='{}', stderr='{}'",
            output.status, stdout.trim(), stderr.trim());
        if !output.status.success() {
            return Err(format!("Task deletion failed: {}", stderr.trim()));
        }
        Ok(())
    }

    fn read_error_log_tail(&self, lines: usize) -> String {
        let content = std::fs::read_to_string(&self.paths.error_log_file).unwrap_or_default();
        content
            .lines()
            .rev()
            .take(lines)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn startup_log_indicates_success(&self, log_tail: &str) -> bool {
        let success_markers = [
            "Bot connected",
            "Listening for messages",
            "Scheduler started",
            "No pending updates",
        ];
        success_markers.iter().any(|marker| log_tail.contains(marker))
    }

    fn benign_error_log_only(&self, err_tail: &str) -> bool {
        let lines: Vec<&str> = err_tail
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        !lines.is_empty()
            && lines.iter().all(|line| {
                line.starts_with("[ccserver]")
            })
    }

    fn append_diagnostic_error(&self, message: &str) {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let line = format!("[cokacctl {}] {}", ts, message);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.paths.error_log_file)
        {
            use std::io::Write;
            let _ = writeln!(file, "{}", line);
        }
        dlog!("taskscheduler", "diagnostic: {}", message);
    }

    fn current_log_size(&self) -> u64 {
        std::fs::metadata(&self.paths.log_file)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    fn read_log_tail_since(&self, start_offset: u64, lines: usize) -> String {
        let content = match std::fs::read(&self.paths.log_file) {
            Ok(content) => content,
            Err(_) => return String::new(),
        };
        let slice = if start_offset >= content.len() as u64 {
            return String::new();
        } else {
            &content[start_offset as usize..]
        };
        let text = String::from_utf8_lossy(slice);
        text.lines()
            .rev()
            .take(lines)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn wait_for_running(&self) -> Result<(), String> {
        let start_log_offset = self.current_log_size();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        let mut saw_running = false;

        while std::time::Instant::now() < deadline {
            match self.query_task_state()? {
                Some(state) if state == "Running" => {
                    saw_running = true;
                }
                Some(state) if saw_running && state == "Ready" => {
                    let err_tail = self.read_error_log_tail(10);
                    let log_tail = self.read_log_tail_since(start_log_offset, 80);
                    if self.startup_log_indicates_success(&log_tail) && Self::is_cokacdir_running() {
                        self.append_diagnostic_error(
                            "Startup verification observed task return to Ready after Running, but service log shows successful startup markers. Treating as success.",
                        );
                        return Ok(());
                    }
                    if !err_tail.trim().is_empty() {
                        if self.benign_error_log_only(&err_tail) {
                            if Self::is_cokacdir_running() {
                                self.append_diagnostic_error(
                                    "Startup verification found only benign [ccserver] stderr output and a live process. Treating startup as success.",
                                );
                                return Ok(());
                            }
                            self.append_diagnostic_error(
                                "Startup verification found only benign [ccserver] stderr output, but no live cokacdir process was found.",
                            );
                            return Err("cokacdir exited immediately after launch.".into());
                        }
                        self.append_diagnostic_error(&format!(
                            "Startup verification failed after task returned to Ready. Recent error log:\n{}",
                            err_tail
                        ));
                        return Err(err_tail);
                    }
                    self.append_diagnostic_error(
                        "Startup verification failed: task reached Running and then Ready, but no success markers or concrete error log were found.",
                    );
                    return Err("cokacdir exited immediately after launch.".into());
                }
                Some(_) | None => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }

        let err_tail = self.read_error_log_tail(10);
        let log_tail = self.read_log_tail_since(start_log_offset, 80);
        if self.startup_log_indicates_success(&log_tail) && Self::is_cokacdir_running() {
            self.append_diagnostic_error(
                "Startup verification timed out waiting for Running state, but service log shows successful startup markers. Treating as success.",
            );
            return Ok(());
        }
        if !err_tail.trim().is_empty() {
            if self.benign_error_log_only(&err_tail) {
                if Self::is_cokacdir_running() {
                    self.append_diagnostic_error(
                        "Startup verification timed out with only benign [ccserver] stderr output, but a live process was found. Treating startup as success.",
                    );
                    return Ok(());
                }
                self.append_diagnostic_error(
                    "Startup verification timed out with only benign [ccserver] stderr output, but no live cokacdir process was found.",
                );
                return Err("cokacdir exited immediately after launch.".into());
            }
            self.append_diagnostic_error(&format!(
                "Startup verification failed before confirming success. Recent error log:\n{}",
                err_tail
            ));
            return Err(err_tail);
        }
        if saw_running {
            if Self::is_cokacdir_running() {
                self.append_diagnostic_error(
                    "Startup verification saw the scheduled task enter Running and a live process is present. Treating as success.",
                );
                Ok(())
            } else {
                self.append_diagnostic_error(
                    "Startup verification saw the scheduled task enter Running, but no live cokacdir process was found afterward.",
                );
                Err("cokacdir exited immediately after launch.".into())
            }
        } else {
            self.append_diagnostic_error(
                "Startup verification failed: scheduled task never reached Running state and no error output was recorded.",
            );
            Err("Scheduled task did not reach Running state.".into())
        }
    }

    fn is_cokacdir_running() -> bool {
        dlog!("taskscheduler", "is_cokacdir_running: checking tasklist...");
        match Self::cmd("tasklist").args(["/FO", "CSV", "/NH"]).output() {
            Ok(output) => {
                let stdout = decode_output(&output.stdout);
                let matching: Vec<&str> = stdout
                    .lines()
                    .filter(|line| line.to_lowercase().starts_with("\"cokacdir"))
                    .collect();
                let found = !matching.is_empty();
                if found {
                    for m in &matching {
                        dlog!("taskscheduler", "is_cokacdir_running: matched process: {}", m);
                    }
                }
                dlog!(
                    "taskscheduler",
                    "is_cokacdir_running: result={} (matched {} processes)",
                    found,
                    matching.len()
                );
                found
            }
            Err(e) => {
                dlog!("taskscheduler", "is_cokacdir_running: tasklist failed: {}", e);
                false
            }
        }
    }

    fn kill_cokacdir_processes(&self) {
        let script =
            "Get-Process | Where-Object { $_.ProcessName -like 'cokacdir*' } | Stop-Process -Force -ErrorAction SilentlyContinue";
        match Self::powershell(script) {
            Ok(out) => {
                let stderr = decode_output(&out.stderr);
                dlog!(
                    "taskscheduler",
                    "kill_cokacdir_processes exit={}, stderr='{}'",
                    out.status,
                    stderr.trim()
                );
            }
            Err(e) => {
                dlog!("taskscheduler", "kill_cokacdir_processes failed: {}", e);
            }
        }
    }
}

impl ServiceManager for TaskSchedulerManager {
    fn start(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String> {
        dlog!("taskscheduler", "========== start() BEGIN ==========");
        dlog!("taskscheduler", "binary_path: '{}'", binary_path.display());
        dlog!("taskscheduler", "binary_path exists: {}", binary_path.exists());
        dlog!("taskscheduler", "tokens count: {}", tokens.len());

        // Remove existing task first
        dlog!("taskscheduler", "[step 1/4] Removing existing task...");
        let remove_result = self.remove();
        dlog!("taskscheduler", "remove result: {:?}", remove_result);

        // Prepare log directory
        let home = dirs::home_dir()
            .ok_or("Cannot determine home directory")?;
        let _ = std::fs::create_dir_all(&self.paths.log_dir);
        if let Some(script_dir) = self.paths.wrapper_script.parent() {
            let _ = std::fs::create_dir_all(script_dir);
        }
        if let Some(state_dir) = self.paths.state_file.parent() {
            let _ = std::fs::create_dir_all(state_dir);
        }
        self.clear_legacy_pid_file();

        // Truncate error log so we only capture fresh errors
        let _ = std::fs::File::create(&self.paths.error_log_file);

        // Generate wrapper script that redirects stdout/stderr to log files
        dlog!("taskscheduler", "[step 2/4] Writing wrapper script...");
        let wrapper = Self::generate_wrapper(binary_path, tokens, &self.paths);
        dlog!("taskscheduler", "Wrapper path: {}", self.paths.wrapper_script.display());
        std::fs::write(&self.paths.wrapper_script, &wrapper)
            .map_err(|e| format!("Cannot write wrapper script: {}", e))?;

        // Register scheduled task to run the wrapper script
        let escape_ps_single = |s: &str| -> String {
            s.replace('\'', "''")
        };

        dlog!("taskscheduler", "[step 3/4] Registering scheduled task...");
        let wrapper_path = self.paths.wrapper_script.to_string_lossy();
        let script = format!(
            "$action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument '/c \"{wrapper}\"' -WorkingDirectory '{wd}'\n\
             $trigger = New-ScheduledTaskTrigger -AtLogon\n\
             $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType S4U -RunLevel Highest\n\
             Register-ScheduledTask -TaskName '{name}' -Action $action -Trigger $trigger -Principal $principal -Force",
            wrapper = escape_ps_single(&wrapper_path),
            wd = escape_ps_single(&home.to_string_lossy()),
            name = TASK_NAME,
        );

        let t0 = std::time::Instant::now();
        let output = Self::powershell(&script)?;
        let elapsed = t0.elapsed();

        let stdout = decode_output(&output.stdout);
        let stderr = decode_output(&output.stderr);
        dlog!("taskscheduler", "Register took: {:?}", elapsed);
        dlog!("taskscheduler", "Register exit code: {}", output.status);
        dlog!("taskscheduler", "Register stdout: '{}'", stdout.trim());
        dlog!("taskscheduler", "Register stderr: '{}'", stderr.trim());

        if !output.status.success() {
            dlog!("taskscheduler", "========== start() FAILED at Register ==========");
            return Err(format!("Task creation failed: {}", stderr.trim()));
        }

        let state = self.service_state(binary_path, tokens);
        if let Err(e) = self.write_state_file(&state) {
            let _ = self.delete_task_if_present();
            return Err(e);
        }

        // Start the scheduled task immediately
        dlog!("taskscheduler", "[step 4/4] Starting scheduled task...");
        let start_output = Self::powershell(&format!(
            "Start-ScheduledTask -TaskName '{}'", TASK_NAME
        ))?;

        let start_stderr = decode_output(&start_output.stderr);
        dlog!("taskscheduler", "Start-ScheduledTask exit: {}, stderr: '{}'", start_output.status, start_stderr.trim());

        if !start_output.status.success() {
            dlog!("taskscheduler", "========== start() FAILED at Start ==========");
            let _ = self.delete_task_if_present();
            let _ = self.remove_state_file();
            return Err(format!("Task start failed: {}", start_stderr.trim()));
        }

        if let Err(e) = self.wait_for_running() {
            let _ = self.delete_task_if_present();
            let _ = self.remove_state_file();
            return Err(e);
        }

        dlog!("taskscheduler", "========== start() SUCCESS ==========");
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        dlog!("taskscheduler", "stop() called");

        if let Err(e) = self.stop_task_if_present() {
            dlog!("taskscheduler", "stop(): stop_task_if_present failed: {}", e);
        }
        self.kill_cokacdir_processes();
        self.clear_legacy_pid_file();

        dlog!("taskscheduler", "stop() done");

        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        dlog!("taskscheduler", "remove() called");

        // Stop first
        if let Err(e) = self.stop_task_if_present() {
            dlog!("taskscheduler", "remove(): stop_task_if_present failed: {}", e);
        }
        self.kill_cokacdir_processes();
        if let Err(e) = self.delete_task_if_present() {
            dlog!("taskscheduler", "remove(): delete_task_if_present failed: {}", e);
        }
        if self.paths.wrapper_script.exists() {
            dlog!("taskscheduler", "Removing wrapper: {}", self.paths.wrapper_script.display());
            if let Err(e) = std::fs::remove_file(&self.paths.wrapper_script) {
                dlog!("taskscheduler", "remove(): wrapper removal failed: {}", e);
            }
        }
        if let Err(e) = self.remove_state_file() {
            dlog!("taskscheduler", "remove(): state file removal failed: {}", e);
        }
        self.clear_legacy_pid_file();
        dlog!("taskscheduler", "remove() done");

        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        dlog!("taskscheduler", "status() called");
        match self.query_task_state() {
            Ok(None) => ServiceStatus::NotInstalled,
            Ok(Some(state)) => {
                let process_running = Self::is_cokacdir_running();
                match state.as_str() {
                    "Running" => {
                        if process_running {
                            ServiceStatus::Running
                        } else {
                            ServiceStatus::Unknown(
                                "Task Scheduler reports Running but no cokacdir process was found"
                                    .into(),
                            )
                        }
                    }
                    // With the cmd.exe wrapper, the scheduled task may fall back to Ready
                    // even while the child process keeps running.
                    "Ready" | "Queued" => {
                        if process_running {
                            ServiceStatus::Running
                        } else {
                            ServiceStatus::Stopped
                        }
                    }
                    "Disabled" => ServiceStatus::Stopped,
                    other => ServiceStatus::Unknown(other.to_string()),
                }
            }
            Err(e) => ServiceStatus::Unknown(e),
        }
    }

    fn is_any_running(&self) -> bool {
        Self::is_cokacdir_running()
    }

    fn log_path(&self) -> Option<PathBuf> {
        dlog!("taskscheduler", "log_path: {}", self.paths.log_file.display());
        Some(self.paths.log_file.clone())
    }
}
