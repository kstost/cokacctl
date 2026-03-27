use super::{ServiceManager, ServiceStatus};
use crate::core::debug::decode_output;
use std::path::{Path, PathBuf};
use std::process::Command;

const TASK_NAME: &str = "cokacdir";

pub struct TaskSchedulerManager;

impl TaskSchedulerManager {
    pub fn new() -> Self {
        dlog!("taskscheduler", "TaskSchedulerManager created");
        TaskSchedulerManager
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

    /// Generate a VBS wrapper that runs cokacdir with a hidden window.
    /// Used by both the scheduled task (logon auto-start) and ensures
    /// no console window flashes on screen.
    fn generate_wrapper(binary_path: &Path, tokens: &[String], log_path: &Path, error_log: &Path) -> String {
        let binary = binary_path.to_string_lossy();
        let log_path = log_path.to_string_lossy();
        let error_log = error_log.to_string_lossy();
        let token_args = tokens.join(" ");
        // Build: cmd /c "binary" --ccserver -- tokens >>"log" 2>>"error_log"
        let cmd = format!(
            "cmd /c \"{}\" --ccserver -- {} >>\"{}\" 2>>\"{}\"",
            binary, token_args, log_path, error_log
        );
        // In VBS strings, " is escaped as ""
        let vbs_cmd = cmd.replace('"', "\"\"");
        format!(
            "Set oShell = CreateObject(\"WScript.Shell\")\noShell.Run \"{}\", 0, True\n",
            vbs_cmd
        )
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

        // Prepare paths
        let home = dirs::home_dir()
            .ok_or("Cannot determine home directory")?;
        let cokacdir_dir = home.join(".cokacdir");
        let log_dir = cokacdir_dir.join("logs");
        let _ = std::fs::create_dir_all(&log_dir);
        let error_log_path = log_dir.join("cokacdir.error.log");

        // Truncate error log so we only capture fresh errors
        let _ = std::fs::File::create(&error_log_path);

        // Generate VBS wrapper for hidden execution (no console window)
        dlog!("taskscheduler", "[step 2/4] Generating VBS wrapper...");
        let wrapper_path = cokacdir_dir.join("run.vbs");
        let log_path = log_dir.join("cokacdir.log");
        let wrapper = Self::generate_wrapper(binary_path, tokens, &log_path, &error_log_path);
        std::fs::write(&wrapper_path, &wrapper)
            .map_err(|e| format!("Cannot write VBS wrapper: {}", e))?;
        dlog!("taskscheduler", "VBS wrapper written to: {}", wrapper_path.display());

        // Register scheduled task with wscript.exe for logon auto-start (no visible window)
        // PowerShell single-quoted strings: only ' needs escaping as ''
        let escape_ps_single = |s: &str| -> String {
            s.replace('\'', "''")
        };
        let wrapper_arg = format!("//B //Nologo \"{}\"", wrapper_path.to_string_lossy());

        dlog!("taskscheduler", "[step 3/4] Registering scheduled task...");
        let script = format!(
            "$action = New-ScheduledTaskAction -Execute 'wscript.exe' -Argument '{args}' -WorkingDirectory '{wd}'\n\
             $trigger = New-ScheduledTaskTrigger -AtLogon\n\
             $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)\n\
             Register-ScheduledTask -TaskName '{name}' -Action $action -Trigger $trigger -Settings $settings -Force",
            args = escape_ps_single(&wrapper_arg),
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

        // Start cokacdir directly with hidden window (no Start-ScheduledTask, no window flash)
        dlog!("taskscheduler", "[step 4/4] Starting cokacdir directly...");
        let stdout_stdio = std::fs::OpenOptions::new()
            .create(true).append(true).open(&log_path)
            .map(std::process::Stdio::from)
            .unwrap_or_else(|_| std::process::Stdio::null());
        let stderr_stdio = std::fs::File::create(&error_log_path)
            .map(std::process::Stdio::from)
            .unwrap_or_else(|_| std::process::Stdio::null());

        let child = Self::cmd(binary_path)
            .args(["--ccserver", "--"])
            .args(tokens)
            .stdin(std::process::Stdio::null())
            .stdout(stdout_stdio)
            .stderr(stderr_stdio)
            .spawn();

        match child {
            Ok(mut c) => {
                dlog!("taskscheduler", "Direct spawn OK, pid: {}", c.id());
                // Check if process survives startup
                std::thread::sleep(std::time::Duration::from_millis(2000));
                match c.try_wait() {
                    Ok(Some(exit_status)) => {
                        let err_output = std::fs::read_to_string(&error_log_path).unwrap_or_default();
                        dlog!("taskscheduler", "Process exited immediately: {}, stderr: '{}'", exit_status, err_output.trim());
                        return Err(format!("cokacdir exited immediately ({}): {}", exit_status, err_output.trim()));
                    }
                    Ok(None) => {
                        dlog!("taskscheduler", "Process still running after 2s - OK");
                    }
                    Err(e) => {
                        dlog!("taskscheduler", "try_wait error: {}", e);
                    }
                }
            }
            Err(e) => {
                dlog!("taskscheduler", "Direct spawn failed: {}", e);
                return Err(format!("Failed to start cokacdir: {}", e));
            }
        }

        dlog!("taskscheduler", "========== start() SUCCESS ==========");
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        dlog!("taskscheduler", "stop() called");

        // Stop the scheduled task
        let stop_result = Self::powershell(&format!(
            "Stop-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue", TASK_NAME
        ));
        if let Ok(ref out) = stop_result {
            let stderr = decode_output(&out.stderr);
            dlog!("taskscheduler", "Stop-ScheduledTask exit: {}, stderr: '{}'", out.status, stderr.trim());
        }

        // Also kill any running cokacdir process
        let kill_result = Self::cmd("taskkill")
            .args(["/IM", "cokacdir.exe", "/F"])
            .output();
        if let Ok(ref out) = kill_result {
            let stdout = decode_output(&out.stdout);
            let stderr = decode_output(&out.stderr);
            dlog!("taskscheduler", "taskkill exit: {}, stdout: '{}', stderr: '{}'",
                out.status, stdout.trim(), stderr.trim());
        }
        dlog!("taskscheduler", "stop() done");

        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        dlog!("taskscheduler", "remove() called");

        // Stop first
        let _ = self.stop();

        // Delete the scheduled task
        let del_result = Self::cmd("schtasks")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .output();
        if let Ok(ref out) = del_result {
            let stdout = decode_output(&out.stdout);
            let stderr = decode_output(&out.stderr);
            dlog!("taskscheduler", "schtasks /Delete exit: {}, stdout: '{}', stderr: '{}'",
                out.status, stdout.trim(), stderr.trim());
        }
        dlog!("taskscheduler", "remove() done");

        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        dlog!("taskscheduler", "status() called");

        // Check if cokacdir.exe process is actually running
        match Self::cmd("tasklist")
            .args(["/FI", "IMAGENAME eq cokacdir.exe", "/FO", "CSV", "/NH"])
            .output()
        {
            Ok(output) => {
                let stdout = decode_output(&output.stdout);
                let line = stdout.trim();
                dlog!("taskscheduler", "status() tasklist: '{}'", line);
                if line.contains("cokacdir.exe") {
                    dlog!("taskscheduler", "status(): Running");
                    return ServiceStatus::Running;
                }
            }
            Err(e) => {
                dlog!("taskscheduler", "status() tasklist failed: {}", e);
            }
        }

        // Check if the scheduled task exists
        match Self::cmd("schtasks")
            .args(["/Query", "/TN", TASK_NAME, "/FO", "CSV", "/NH"])
            .output()
        {
            Ok(output) => {
                let stdout = decode_output(&output.stdout);
                let stderr = decode_output(&output.stderr);
                if !output.status.success() {
                    dlog!("taskscheduler", "status(): NotInstalled (stderr: '{}')", stderr.trim());
                    ServiceStatus::NotInstalled
                } else {
                    dlog!("taskscheduler", "status(): Stopped (stdout: '{}')", stdout.trim());
                    ServiceStatus::Stopped
                }
            }
            Err(e) => {
                dlog!("taskscheduler", "status() schtasks failed: {}", e);
                ServiceStatus::Unknown("Cannot query Task Scheduler".into())
            }
        }
    }

    fn log_path(&self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        let path = home.join(".cokacdir").join("logs").join("cokacdir.log");
        dlog!("taskscheduler", "log_path: {}", path.display());
        Some(path)
    }
}
