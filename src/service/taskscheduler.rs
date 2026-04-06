use super::{ServiceManager, ServiceStatus};
use crate::core::debug::decode_output;
use crate::core::platform::ServicePaths;
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
        let script = format!(
            "$action = New-ScheduledTaskAction -Execute '{exe}' -WorkingDirectory '{wd}'\n\
             $trigger = New-ScheduledTaskTrigger -AtLogon\n\
             $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType S4U -RunLevel Highest\n\
             Register-ScheduledTask -TaskName '{name}' -Action $action -Trigger $trigger -Principal $principal -Force",
            exe = escape_ps_single(&self.paths.wrapper_script.to_string_lossy()),
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
        dlog!("taskscheduler", "[step 4/4] Starting scheduled task...");
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
                    let err_output = std::fs::read_to_string(&self.paths.error_log_file).unwrap_or_default();
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
        if self.paths.wrapper_script.exists() {
            dlog!("taskscheduler", "Removing wrapper: {}", self.paths.wrapper_script.display());
            std::fs::remove_file(&self.paths.wrapper_script).ok();
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
        dlog!("taskscheduler", "log_path: {}", self.paths.log_file.display());
        Some(self.paths.log_file.clone())
    }
}
