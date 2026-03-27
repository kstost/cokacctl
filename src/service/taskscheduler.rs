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

}

impl ServiceManager for TaskSchedulerManager {
    fn start(&self, binary_path: &Path, tokens: &[String]) -> Result<(), String> {
        dlog!("taskscheduler", "========== start() BEGIN ==========");
        dlog!("taskscheduler", "binary_path: '{}'", binary_path.display());
        dlog!("taskscheduler", "binary_path exists: {}", binary_path.exists());
        dlog!("taskscheduler", "tokens count: {}", tokens.len());

        // Remove existing task first
        dlog!("taskscheduler", "[step 1/3] Removing existing task...");
        let remove_result = self.remove();
        dlog!("taskscheduler", "remove result: {:?}", remove_result);

        // Prepare log directory
        let home = dirs::home_dir()
            .ok_or("Cannot determine home directory")?;
        let cokacdir_dir = home.join(".cokacdir");
        let log_dir = cokacdir_dir.join("logs");
        let _ = std::fs::create_dir_all(&log_dir);
        let error_log_path = log_dir.join("cokacdir.error.log");

        // Truncate error log so we only capture fresh errors
        let _ = std::fs::File::create(&error_log_path);

        // Register scheduled task with cokacdir.exe directly
        let escape_ps_single = |s: &str| -> String {
            s.replace('\'', "''")
        };
        let token_args = tokens.join(" ");
        let argument = format!("--ccserver -- {}", token_args);

        dlog!("taskscheduler", "[step 2/3] Registering scheduled task...");
        let script = format!(
            "$action = New-ScheduledTaskAction -Execute '{exe}' -Argument '{args}' -WorkingDirectory '{wd}'\n\
             $trigger = New-ScheduledTaskTrigger -AtLogon\n\
             $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType S4U -RunLevel Highest\n\
             Register-ScheduledTask -TaskName '{name}' -Action $action -Trigger $trigger -Principal $principal -Force",
            exe = escape_ps_single(&binary_path.to_string_lossy()),
            args = escape_ps_single(&argument),
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

        // Start the scheduled task immediately
        dlog!("taskscheduler", "[step 3/3] Starting scheduled task...");
        let start_output = Self::powershell(&format!(
            "Start-ScheduledTask -TaskName '{}'", TASK_NAME
        ))?;

        let start_stderr = decode_output(&start_output.stderr);
        dlog!("taskscheduler", "Start-ScheduledTask exit: {}, stderr: '{}'", start_output.status, start_stderr.trim());

        if !start_output.status.success() {
            dlog!("taskscheduler", "========== start() FAILED at Start ==========");
            return Err(format!("Task start failed: {}", start_stderr.trim()));
        }

        // Wait briefly and verify process is running
        std::thread::sleep(std::time::Duration::from_millis(2000));
        match Self::cmd("tasklist")
            .args(["/FI", "IMAGENAME eq cokacdir.exe", "/FO", "CSV", "/NH"])
            .output()
        {
            Ok(tl_output) => {
                let tl_stdout = decode_output(&tl_output.stdout);
                if !tl_stdout.contains("cokacdir.exe") {
                    let err_output = std::fs::read_to_string(&error_log_path).unwrap_or_default();
                    dlog!("taskscheduler", "Process not found after start: '{}'", err_output.trim());
                    return Err(format!("cokacdir exited immediately: {}", err_output.trim()));
                }
                dlog!("taskscheduler", "Process running after 2s - OK");
            }
            Err(e) => {
                dlog!("taskscheduler", "tasklist check failed: {}", e);
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
