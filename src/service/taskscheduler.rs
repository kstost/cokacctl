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

    /// Encode a string for a `.bat` file using the system's OEM code page.
    ///
    /// cmd.exe parses `.bat` contents using the OEM code page (e.g. CP949 on
    /// Korean Windows). If we write UTF-8 bytes directly, non-ASCII characters
    /// in paths (e.g. a Korean username in `C:\Users\...`) are mis-decoded and
    /// cmd.exe fails to locate the executable, producing a silent exit 1.
    ///
    /// On non-Windows platforms this is a no-op (the file isn't parsed by cmd).
    #[cfg(windows)]
    fn encode_for_bat(s: &str) -> Vec<u8> {
        extern "system" {
            fn WideCharToMultiByte(
                CodePage: u32,
                dwFlags: u32,
                lpWideCharStr: *const u16,
                cchWideChar: i32,
                lpMultiByteStr: *mut u8,
                cbMultiByte: i32,
                lpDefaultChar: *const u8,
                lpUsedDefaultChar: *mut i32,
            ) -> i32;
        }
        const CP_OEMCP: u32 = 1;

        let wide: Vec<u16> = s.encode_utf16().collect();
        if wide.is_empty() {
            return Vec::new();
        }
        unsafe {
            let needed = WideCharToMultiByte(
                CP_OEMCP,
                0,
                wide.as_ptr(),
                wide.len() as i32,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            if needed <= 0 {
                return s.as_bytes().to_vec();
            }
            let mut buf = vec![0u8; needed as usize];
            let written = WideCharToMultiByte(
                CP_OEMCP,
                0,
                wide.as_ptr(),
                wide.len() as i32,
                buf.as_mut_ptr(),
                needed,
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            if written <= 0 {
                return s.as_bytes().to_vec();
            }
            buf.truncate(written as usize);
            buf
        }
    }

    #[cfg(not(windows))]
    fn encode_for_bat(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    #[cfg(windows)]
    fn oem_code_page() -> u32 {
        extern "system" {
            fn GetOEMCP() -> u32;
        }
        unsafe { GetOEMCP() }
    }

    #[cfg(not(windows))]
    fn oem_code_page() -> u32 {
        0
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

    // ------------------------------------------------------------------
    // Operation log (~/.cokacdir/logs/cokacctl.log)
    //
    // Records every step of service operations — PowerShell scripts sent,
    // exit codes, stdout, stderr, task state snapshots, environment info.
    // Never truncated; sessions are separated by headers.
    // ------------------------------------------------------------------

    fn ops_log_path(&self) -> PathBuf {
        self.paths.log_dir.join("cokacctl.log")
    }

    fn ops_now() -> String {
        chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S%.3f")
            .to_string()
    }

    fn ops_append_raw(&self, s: &str) {
        let _ = std::fs::create_dir_all(&self.paths.log_dir);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.ops_log_path())
        {
            use std::io::Write;
            let _ = f.write_all(s.as_bytes());
            let _ = f.flush();
        }
    }

    fn ops_log(&self, msg: &str) {
        let line = format!("[{}] {}\n", Self::ops_now(), msg);
        self.ops_append_raw(&line);
        dlog!("taskscheduler", "{}", msg);
    }

    fn ops_log_block(&self, header: &str, body: &str) {
        let mut out = format!("[{}] {}\n", Self::ops_now(), header);
        if body.is_empty() {
            out.push_str("    <empty>\n");
        } else {
            for line in body.lines() {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
        }
        self.ops_append_raw(&out);
    }

    fn ops_log_section(&self, op: &str, context: &str) {
        let sep = "=".repeat(72);
        let mut body = String::new();
        body.push('\n');
        body.push_str(&sep);
        body.push('\n');
        body.push_str(&format!("[{}] BEGIN {}\n", Self::ops_now(), op));
        if !context.is_empty() {
            for line in context.lines() {
                body.push_str("    ");
                body.push_str(line);
                body.push('\n');
            }
        }
        body.push_str(&sep);
        body.push('\n');
        self.ops_append_raw(&body);
    }

    fn ops_log_section_end(&self, op: &str, result: &str) {
        let body = format!("[{}] END {}: {}\n", Self::ops_now(), op, result);
        self.ops_append_raw(&body);
    }

    fn format_ps_execution(
        script: &str,
        output: &std::process::Output,
        elapsed: std::time::Duration,
    ) -> String {
        let stdout = decode_output(&output.stdout);
        let stderr = decode_output(&output.stderr);
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "<signal>".to_string());
        let mut s = String::new();
        s.push_str("ps-script:\n");
        for line in script.lines() {
            s.push_str("    | ");
            s.push_str(line);
            s.push('\n');
        }
        s.push_str(&format!("exit: {}\n", code));
        s.push_str(&format!("duration: {} ms\n", elapsed.as_millis()));
        s.push_str("stdout:\n");
        if stdout.trim().is_empty() {
            s.push_str("    <empty>\n");
        } else {
            for line in stdout.lines() {
                s.push_str("    | ");
                s.push_str(line);
                s.push('\n');
            }
        }
        s.push_str("stderr:\n");
        if stderr.trim().is_empty() {
            s.push_str("    <empty>\n");
        } else {
            for line in stderr.lines() {
                s.push_str("    | ");
                s.push_str(line);
                s.push('\n');
            }
        }
        s
    }

    fn run_ps_logged(
        &self,
        label: &str,
        script: &str,
    ) -> Result<std::process::Output, String> {
        let t0 = std::time::Instant::now();
        let result = Self::powershell(script);
        let elapsed = t0.elapsed();
        match &result {
            Ok(output) => {
                let block = Self::format_ps_execution(script, output, elapsed);
                self.ops_log_block(label, &block);
            }
            Err(e) => {
                let script_lines: String = script
                    .lines()
                    .map(|l| format!("    | {}\n", l))
                    .collect();
                let body = format!(
                    "ps-script:\n{}invocation-error: {}\nduration: {} ms\n",
                    script_lines,
                    e,
                    elapsed.as_millis()
                );
                self.ops_log_block(label, &body);
            }
        }
        result
    }

    fn snapshot_env(&self) -> String {
        let script = "$ErrorActionPreference='SilentlyContinue'\n\
            Write-Output \"user: $env:USERNAME\"\n\
            Write-Output \"computer: $env:COMPUTERNAME\"\n\
            Write-Output \"userprofile: $env:USERPROFILE\"\n\
            $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)\n\
            Write-Output \"is-admin: $isAdmin\"\n\
            Write-Output \"os-version: $([System.Environment]::OSVersion.Version)\"\n\
            Write-Output \"powershell: $($PSVersionTable.PSVersion)\"\n\
            $b = Get-CimInstance -ClassName Win32_Battery -ErrorAction SilentlyContinue\n\
            if ($b) { Write-Output \"battery.BatteryStatus: $($b.BatteryStatus)\" } else { Write-Output \"battery: none\" }\n\
            $sched = Get-Service -Name Schedule -ErrorAction SilentlyContinue\n\
            if ($sched) { Write-Output \"schedule-service: $($sched.Status)\" } else { Write-Output \"schedule-service: missing\" }\n\
            $dom = (Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction SilentlyContinue)\n\
            if ($dom) { Write-Output \"partofdomain: $($dom.PartOfDomain)\"; Write-Output \"domain: $($dom.Domain)\" }\n\
            $policy = Get-ExecutionPolicy -List | ForEach-Object { \"$($_.Scope)=$($_.ExecutionPolicy)\" }\n\
            Write-Output \"exec-policy: $($policy -join ', ')\"";
        match Self::powershell(script) {
            Ok(out) => {
                let stdout = decode_output(&out.stdout);
                let stderr = decode_output(&out.stderr);
                if stderr.trim().is_empty() {
                    stdout
                } else {
                    format!("{}\n<stderr>\n{}", stdout, stderr)
                }
            }
            Err(e) => format!("<query failed: {}>", e),
        }
    }

    fn snapshot_task(&self) -> String {
        let script = format!(
            "$ErrorActionPreference='SilentlyContinue'\n\
             $task = Get-ScheduledTask -TaskName '{name}'\n\
             if ($task) {{\n\
                 Write-Output \"exists: true\"\n\
                 Write-Output \"state: $($task.State)\"\n\
                 Write-Output \"principal.userid: $($task.Principal.UserId)\"\n\
                 Write-Output \"principal.logontype: $($task.Principal.LogonType)\"\n\
                 Write-Output \"principal.runlevel: $($task.Principal.RunLevel)\"\n\
                 Write-Output \"settings.DisallowStartIfOnBatteries: $($task.Settings.DisallowStartIfOnBatteries)\"\n\
                 Write-Output \"settings.StopIfGoingOnBatteries: $($task.Settings.StopIfGoingOnBatteries)\"\n\
                 Write-Output \"settings.ExecutionTimeLimit: $($task.Settings.ExecutionTimeLimit)\"\n\
                 Write-Output \"settings.Enabled: $($task.Settings.Enabled)\"\n\
                 Write-Output \"settings.AllowDemandStart: $($task.Settings.AllowDemandStart)\"\n\
                 foreach ($a in $task.Actions) {{\n\
                     Write-Output \"action.execute: $($a.Execute)\"\n\
                     Write-Output \"action.arguments: $($a.Arguments)\"\n\
                     Write-Output \"action.workingdir: $($a.WorkingDirectory)\"\n\
                 }}\n\
                 $info = Get-ScheduledTaskInfo -TaskName '{name}'\n\
                 if ($info) {{\n\
                     Write-Output (\"info.LastTaskResult: 0x{{0:X8}} ({{1}})\" -f $info.LastTaskResult, $info.LastTaskResult)\n\
                     Write-Output \"info.LastRunTime: $($info.LastRunTime)\"\n\
                     Write-Output \"info.NumberOfMissedRuns: $($info.NumberOfMissedRuns)\"\n\
                     Write-Output \"info.NextRunTime: $($info.NextRunTime)\"\n\
                 }} else {{\n\
                     Write-Output \"info: <unavailable>\"\n\
                 }}\n\
             }} else {{\n\
                 Write-Output \"exists: false\"\n\
             }}",
            name = TASK_NAME,
        );
        let t0 = std::time::Instant::now();
        match Self::powershell(&script) {
            Ok(out) => {
                let stdout = decode_output(&out.stdout);
                let stderr = decode_output(&out.stderr);
                let mut s = stdout;
                if !stderr.trim().is_empty() {
                    s.push_str("\n<stderr>\n");
                    s.push_str(&stderr);
                }
                s.push_str(&format!("\n<query-ms: {}>", t0.elapsed().as_millis()));
                s
            }
            Err(e) => format!("<query failed: {}>", e),
        }
    }

    fn snapshot_process(&self) -> String {
        let script = "$ErrorActionPreference='SilentlyContinue'\n\
            $procs = Get-Process | Where-Object { $_.ProcessName -like 'cokacdir*' }\n\
            if ($procs) {\n\
                $procs | ForEach-Object { Write-Output (\"pid={0} name={1} path={2}\" -f $_.Id, $_.ProcessName, $_.Path) }\n\
            } else {\n\
                Write-Output \"<no cokacdir process>\"\n\
            }";
        match Self::powershell(script) {
            Ok(out) => decode_output(&out.stdout),
            Err(e) => format!("<query failed: {}>", e),
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
                self.ops_log("stop_task_if_present: task missing, nothing to stop");
                dlog!("taskscheduler", "stop_task_if_present: task missing");
                return Ok(());
            }
            Some(state) if state != "Running" => {
                self.ops_log(&format!(
                    "stop_task_if_present: task present, state={}, nothing to stop",
                    state
                ));
                dlog!("taskscheduler", "stop_task_if_present: task present but state={}, nothing to stop", state);
                return Ok(());
            }
            Some(_) => {}
        }
        let script = format!("Stop-ScheduledTask -TaskName '{}'", TASK_NAME);
        let output = self.run_ps_logged("stop_task_if_present: Stop-ScheduledTask", &script)?;
        let stderr = decode_output(&output.stderr);
        dlog!("taskscheduler", "stop_task_if_present exit={}, stderr='{}'", output.status, stderr.trim());
        if !output.status.success() {
            let err = format!("Task stop failed: {}", stderr.trim());
            self.ops_log(&format!("stop_task_if_present: ERROR {}", err));
            return Err(err);
        }
        Ok(())
    }

    fn delete_task_if_present(&self) -> Result<(), String> {
        if !self.task_exists()? {
            self.ops_log("delete_task_if_present: task missing, nothing to delete");
            dlog!("taskscheduler", "delete_task_if_present: task missing");
            return Ok(());
        }
        let t0 = std::time::Instant::now();
        let output = Self::cmd("schtasks")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .output()
            .map_err(|e| format!("Task deletion failed: {}", e))?;
        let elapsed = t0.elapsed();
        let stdout = decode_output(&output.stdout);
        let stderr = decode_output(&output.stderr);
        self.ops_log_block(
            "delete_task_if_present: schtasks /Delete",
            &format!(
                "argv: schtasks /Delete /TN {} /F\n\
                 exit: {}\n\
                 duration: {} ms\n\
                 stdout:\n    | {}\n\
                 stderr:\n    | {}",
                TASK_NAME,
                output
                    .status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "<signal>".to_string()),
                elapsed.as_millis(),
                stdout.trim().replace('\n', "\n    | "),
                stderr.trim().replace('\n', "\n    | "),
            ),
        );
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
        let t_begin = std::time::Instant::now();
        let deadline = t_begin + std::time::Duration::from_secs(4);
        let mut saw_running = false;
        let mut poll_index: u32 = 0;

        self.ops_log("wait_for_running: begin (budget=4000ms, interval=250ms)");
        self.ops_log(&format!(
            "wait_for_running: log_file_offset_at_begin={} bytes",
            start_log_offset
        ));

        while std::time::Instant::now() < deadline {
            poll_index += 1;
            let elapsed_ms = t_begin.elapsed().as_millis();
            let state_result = self.query_task_state();
            match &state_result {
                Ok(Some(state)) => self.ops_log(&format!(
                    "poll#{} t+{}ms state={} saw_running={}",
                    poll_index, elapsed_ms, state, saw_running
                )),
                Ok(None) => self.ops_log(&format!(
                    "poll#{} t+{}ms task-missing (Get-ScheduledTask returns nothing) saw_running={}",
                    poll_index, elapsed_ms, saw_running
                )),
                Err(e) => self.ops_log(&format!(
                    "poll#{} t+{}ms query-error: {}",
                    poll_index, elapsed_ms, e
                )),
            }

            match state_result? {
                Some(state) if state == "Running" => {
                    if !saw_running {
                        self.ops_log(&format!(
                            "poll#{}: first time observing Running state",
                            poll_index
                        ));
                    }
                    saw_running = true;
                }
                Some(state) if saw_running && state == "Ready" => {
                    self.ops_log(
                        "observed Running -> Ready transition; evaluating success/error markers",
                    );
                    let err_tail = self.read_error_log_tail(10);
                    let log_tail = self.read_log_tail_since(start_log_offset, 80);
                    self.ops_log_block("service log tail (since begin)", log_tail.trim_end());
                    self.ops_log_block("error log tail", err_tail.trim_end());
                    if self.startup_log_indicates_success(&log_tail)
                        && Self::is_cokacdir_running()
                    {
                        self.ops_log(
                            "branch: Running->Ready, log indicates success and process alive -> OK",
                        );
                        self.append_diagnostic_error(
                            "Startup verification observed task return to Ready after Running, but service log shows successful startup markers. Treating as success.",
                        );
                        return Ok(());
                    }
                    if !err_tail.trim().is_empty() {
                        if self.benign_error_log_only(&err_tail) {
                            if Self::is_cokacdir_running() {
                                self.ops_log(
                                    "branch: Running->Ready, benign stderr, process alive -> OK",
                                );
                                self.append_diagnostic_error(
                                    "Startup verification found only benign [ccserver] stderr output and a live process. Treating startup as success.",
                                );
                                return Ok(());
                            }
                            self.ops_log(
                                "branch: Running->Ready, benign stderr, no process -> FAIL",
                            );
                            self.append_diagnostic_error(
                                "Startup verification found only benign [ccserver] stderr output, but no live cokacdir process was found.",
                            );
                            return Err("cokacdir exited immediately after launch.".into());
                        }
                        self.ops_log(
                            "branch: Running->Ready, real error output present -> FAIL",
                        );
                        self.append_diagnostic_error(&format!(
                            "Startup verification failed after task returned to Ready. Recent error log:\n{}",
                            err_tail
                        ));
                        return Err(err_tail);
                    }
                    self.ops_log(
                        "branch: Running->Ready, no success markers, no error log -> FAIL",
                    );
                    self.append_diagnostic_error(
                        "Startup verification failed: task reached Running and then Ready, but no success markers or concrete error log were found.",
                    );
                    return Err("cokacdir exited immediately after launch.".into());
                }
                Some(_) | None => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }

        self.ops_log(&format!(
            "wait_for_running: polling finished at t+{}ms, saw_running={}",
            t_begin.elapsed().as_millis(),
            saw_running
        ));

        let err_tail = self.read_error_log_tail(10);
        let log_tail = self.read_log_tail_since(start_log_offset, 80);
        let process_alive = Self::is_cokacdir_running();
        self.ops_log(&format!(
            "post-timeout: process_alive={}, error_log_has_content={}, log_has_success_markers={}",
            process_alive,
            !err_tail.trim().is_empty(),
            self.startup_log_indicates_success(&log_tail)
        ));
        self.ops_log_block("service log tail (since begin)", log_tail.trim_end());
        self.ops_log_block("error log tail", err_tail.trim_end());
        self.ops_log_block("task snapshot at timeout", &self.snapshot_task());
        self.ops_log_block("process snapshot at timeout", &self.snapshot_process());

        if self.startup_log_indicates_success(&log_tail) && process_alive {
            self.ops_log("branch (timeout): success markers + process alive -> OK");
            self.append_diagnostic_error(
                "Startup verification timed out waiting for Running state, but service log shows successful startup markers. Treating as success.",
            );
            return Ok(());
        }
        if !err_tail.trim().is_empty() {
            if self.benign_error_log_only(&err_tail) {
                if process_alive {
                    self.ops_log(
                        "branch (timeout): benign stderr + process alive -> OK",
                    );
                    self.append_diagnostic_error(
                        "Startup verification timed out with only benign [ccserver] stderr output, but a live process was found. Treating startup as success.",
                    );
                    return Ok(());
                }
                self.ops_log("branch (timeout): benign stderr + no process -> FAIL");
                self.append_diagnostic_error(
                    "Startup verification timed out with only benign [ccserver] stderr output, but no live cokacdir process was found.",
                );
                return Err("cokacdir exited immediately after launch.".into());
            }
            self.ops_log("branch (timeout): real error output present -> FAIL");
            self.append_diagnostic_error(&format!(
                "Startup verification failed before confirming success. Recent error log:\n{}",
                err_tail
            ));
            return Err(err_tail);
        }
        if saw_running {
            if process_alive {
                self.ops_log(
                    "branch (timeout): saw Running earlier, process alive -> OK",
                );
                self.append_diagnostic_error(
                    "Startup verification saw the scheduled task enter Running and a live process is present. Treating as success.",
                );
                Ok(())
            } else {
                self.ops_log(
                    "branch (timeout): saw Running earlier, but no process now -> FAIL",
                );
                self.append_diagnostic_error(
                    "Startup verification saw the scheduled task enter Running, but no live cokacdir process was found afterward.",
                );
                Err("cokacdir exited immediately after launch.".into())
            }
        } else {
            self.ops_log(
                "branch (timeout): never saw Running, no error log -> FAIL (root cause unclear — see task snapshot above for LastTaskResult)",
            );
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
        match self.run_ps_logged("kill_cokacdir_processes", script) {
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
        self.ops_log_section(
            "start",
            &format!(
                "binary_path: {}\n\
                 binary_exists: {}\n\
                 tokens: {}\n\
                 wrapper_script: {}\n\
                 log_file: {}\n\
                 error_log_file: {}\n\
                 state_file: {}\n\
                 ops_log_file: {}\n\
                 task_name: {}\n\
                 cokacctl_version: {}",
                binary_path.display(),
                binary_path.exists(),
                tokens.len(),
                self.paths.wrapper_script.display(),
                self.paths.log_file.display(),
                self.paths.error_log_file.display(),
                self.paths.state_file.display(),
                self.ops_log_path().display(),
                TASK_NAME,
                env!("CARGO_PKG_VERSION"),
            ),
        );
        self.ops_log_block("environment", &self.snapshot_env());
        self.ops_log_block("task before start", &self.snapshot_task());
        self.ops_log_block("processes before start", &self.snapshot_process());

        dlog!("taskscheduler", "========== start() BEGIN ==========");
        dlog!("taskscheduler", "binary_path: '{}'", binary_path.display());
        dlog!("taskscheduler", "binary_path exists: {}", binary_path.exists());
        dlog!("taskscheduler", "tokens count: {}", tokens.len());

        // Remove existing task first
        self.ops_log("[step 1/4] pre-clean via remove()");
        dlog!("taskscheduler", "[step 1/4] Removing existing task...");
        let remove_result = self.remove();
        self.ops_log(&format!("[step 1/4] remove() result: {:?}", remove_result));
        dlog!("taskscheduler", "remove result: {:?}", remove_result);

        // Prepare log directory
        let home = match dirs::home_dir() {
            Some(h) => h,
            None => {
                let err = "Cannot determine home directory".to_string();
                self.ops_log(&format!("ERROR at step 2/4: {}", err));
                self.ops_log_section_end("start", "FAIL: no home dir");
                return Err(err);
            }
        };
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
        self.ops_log(&format!(
            "[step 2/4] dirs ensured; error_log_file truncated: {}",
            self.paths.error_log_file.display()
        ));

        // Generate wrapper script that redirects stdout/stderr to log files.
        // cmd.exe parses .bat in the OEM code page, so UTF-8 bytes with
        // non-ASCII path chars (e.g. Korean username) get mis-decoded and
        // the task exits 1 before our redirection runs. Encode to OEM bytes.
        dlog!("taskscheduler", "[step 2/4] Writing wrapper script...");
        let wrapper = Self::generate_wrapper(binary_path, tokens, &self.paths);
        let wrapper_bytes = Self::encode_for_bat(&wrapper);
        let oem_cp = Self::oem_code_page();
        dlog!("taskscheduler", "Wrapper path: {}", self.paths.wrapper_script.display());
        if let Err(e) = std::fs::write(&self.paths.wrapper_script, &wrapper_bytes) {
            let err = format!("Cannot write wrapper script: {}", e);
            self.ops_log(&format!("ERROR at step 2/4: {}", err));
            self.ops_log_section_end("start", "FAIL: wrapper write");
            return Err(err);
        }
        self.ops_log_block(
            "[step 2/4] wrapper script written",
            &format!(
                "path: {}\nutf8_bytes: {}\noem_encoded_bytes: {}\noem_code_page: {}\ncontent (utf-8 view):\n{}",
                self.paths.wrapper_script.display(),
                wrapper.len(),
                wrapper_bytes.len(),
                oem_cp,
                wrapper.trim_end()
            ),
        );

        // Register scheduled task to run the wrapper script
        let escape_ps_single = |s: &str| -> String {
            s.replace('\'', "''")
        };

        dlog!("taskscheduler", "[step 3/4] Registering scheduled task...");
        let wrapper_path = self.paths.wrapper_script.to_string_lossy();
        let script = format!(
            "$ErrorActionPreference = 'Stop'\n\
             try {{\n\
                 $action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument '/c \"{wrapper}\"' -WorkingDirectory '{wd}'\n\
                 $trigger = New-ScheduledTaskTrigger -AtLogon\n\
                 $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType S4U -RunLevel Highest\n\
                 $task = Register-ScheduledTask -TaskName '{name}' -Action $action -Trigger $trigger -Principal $principal -Force\n\
                 Write-Output \"registered: true\"\n\
                 Write-Output \"state-after-register: $($task.State)\"\n\
             }} catch {{\n\
                 Write-Output \"registered: false\"\n\
                 Write-Error ($_ | Out-String)\n\
                 exit 1\n\
             }}",
            wrapper = escape_ps_single(&wrapper_path),
            wd = escape_ps_single(&home.to_string_lossy()),
            name = TASK_NAME,
        );

        let output = match self.run_ps_logged("[step 3/4] Register-ScheduledTask", &script) {
            Ok(o) => o,
            Err(e) => {
                self.ops_log(&format!("ERROR at step 3/4 invocation: {}", e));
                self.ops_log_section_end("start", &format!("FAIL: Register invocation: {}", e));
                return Err(e);
            }
        };

        if !output.status.success() {
            let stderr = decode_output(&output.stderr);
            let err = format!("Task creation failed: {}", stderr.trim());
            self.ops_log(&format!("ERROR at step 3/4: {}", err));
            self.ops_log_block("task snapshot after Register failure", &self.snapshot_task());
            self.ops_log_section_end("start", &format!("FAIL: {}", err));
            dlog!("taskscheduler", "========== start() FAILED at Register ==========");
            return Err(err);
        }

        // Post-register verification — registration can succeed (exit=0) yet
        // leave the task missing if PowerShell swallowed a non-terminating error.
        match self.query_task_state() {
            Ok(Some(state)) => {
                self.ops_log(&format!(
                    "[step 3/4] verified: Get-ScheduledTask finds task, state={}",
                    state
                ));
            }
            Ok(None) => {
                let err =
                    "Task creation reported success, but Get-ScheduledTask returns no task"
                        .to_string();
                self.ops_log(&format!("ERROR at step 3/4 verify: {}", err));
                self.ops_log_block("task snapshot after verify miss", &self.snapshot_task());
                self.ops_log_section_end("start", "FAIL: post-register verify (task missing)");
                return Err(err);
            }
            Err(e) => {
                self.ops_log(&format!(
                    "[step 3/4] verify query error (continuing anyway): {}",
                    e
                ));
            }
        }
        self.ops_log_block("[step 3/4] task snapshot after register", &self.snapshot_task());

        let state = self.service_state(binary_path, tokens);
        if let Err(e) = self.write_state_file(&state) {
            self.ops_log(&format!("ERROR writing state file: {}", e));
            let _ = self.delete_task_if_present();
            self.ops_log_section_end("start", &format!("FAIL: state file: {}", e));
            return Err(e);
        }

        // Start the scheduled task immediately
        dlog!("taskscheduler", "[step 4/4] Starting scheduled task...");
        let start_script = format!(
            "$ErrorActionPreference = 'Stop'\n\
             try {{\n\
                 Start-ScheduledTask -TaskName '{name}'\n\
                 Write-Output \"started: true\"\n\
             }} catch {{\n\
                 Write-Output \"started: false\"\n\
                 Write-Error ($_ | Out-String)\n\
                 exit 1\n\
             }}",
            name = TASK_NAME,
        );
        let start_output = match self.run_ps_logged("[step 4/4] Start-ScheduledTask", &start_script) {
            Ok(o) => o,
            Err(e) => {
                self.ops_log(&format!("ERROR at step 4/4 invocation: {}", e));
                self.ops_log_block("task snapshot after Start invocation error", &self.snapshot_task());
                let _ = self.delete_task_if_present();
                let _ = self.remove_state_file();
                self.ops_log_section_end("start", &format!("FAIL: Start invocation: {}", e));
                return Err(e);
            }
        };

        if !start_output.status.success() {
            let start_stderr = decode_output(&start_output.stderr);
            let err = format!("Task start failed: {}", start_stderr.trim());
            self.ops_log(&format!("ERROR at step 4/4: {}", err));
            self.ops_log_block("task snapshot after Start failure", &self.snapshot_task());
            dlog!("taskscheduler", "========== start() FAILED at Start ==========");
            let _ = self.delete_task_if_present();
            let _ = self.remove_state_file();
            self.ops_log_section_end("start", &format!("FAIL: {}", err));
            return Err(err);
        }

        self.ops_log_block(
            "[step 4/4] task snapshot immediately after Start",
            &self.snapshot_task(),
        );

        if let Err(e) = self.wait_for_running() {
            self.ops_log(&format!("ERROR at wait_for_running: {}", e));
            self.ops_log_block("final task snapshot", &self.snapshot_task());
            self.ops_log_block("final process snapshot", &self.snapshot_process());
            let _ = self.delete_task_if_present();
            let _ = self.remove_state_file();
            self.ops_log_section_end("start", &format!("FAIL: {}", e));
            return Err(e);
        }

        dlog!("taskscheduler", "========== start() SUCCESS ==========");
        self.ops_log_block("task snapshot on success", &self.snapshot_task());
        self.ops_log_block("process snapshot on success", &self.snapshot_process());
        self.ops_log_section_end("start", "OK");
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        self.ops_log_section(
            "stop",
            &format!(
                "task_name: {}\ncokacctl_version: {}",
                TASK_NAME,
                env!("CARGO_PKG_VERSION")
            ),
        );
        self.ops_log_block("task before stop", &self.snapshot_task());
        self.ops_log_block("processes before stop", &self.snapshot_process());
        dlog!("taskscheduler", "stop() called");

        if let Err(e) = self.stop_task_if_present() {
            self.ops_log(&format!("stop(): stop_task_if_present failed: {}", e));
            dlog!("taskscheduler", "stop(): stop_task_if_present failed: {}", e);
        }
        self.kill_cokacdir_processes();
        self.clear_legacy_pid_file();

        self.ops_log_block("task after stop", &self.snapshot_task());
        self.ops_log_block("processes after stop", &self.snapshot_process());
        dlog!("taskscheduler", "stop() done");
        self.ops_log_section_end("stop", "OK");

        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        self.ops_log_section(
            "remove",
            &format!(
                "task_name: {}\ncokacctl_version: {}",
                TASK_NAME,
                env!("CARGO_PKG_VERSION")
            ),
        );
        self.ops_log_block("task before remove", &self.snapshot_task());
        self.ops_log_block("processes before remove", &self.snapshot_process());
        dlog!("taskscheduler", "remove() called");

        // Stop first
        if let Err(e) = self.stop_task_if_present() {
            self.ops_log(&format!("remove(): stop_task_if_present failed: {}", e));
            dlog!("taskscheduler", "remove(): stop_task_if_present failed: {}", e);
        }
        self.kill_cokacdir_processes();
        if let Err(e) = self.delete_task_if_present() {
            self.ops_log(&format!("remove(): delete_task_if_present failed: {}", e));
            dlog!("taskscheduler", "remove(): delete_task_if_present failed: {}", e);
        }
        if self.paths.wrapper_script.exists() {
            self.ops_log(&format!(
                "remove(): deleting wrapper {}",
                self.paths.wrapper_script.display()
            ));
            dlog!("taskscheduler", "Removing wrapper: {}", self.paths.wrapper_script.display());
            if let Err(e) = std::fs::remove_file(&self.paths.wrapper_script) {
                self.ops_log(&format!("remove(): wrapper removal failed: {}", e));
                dlog!("taskscheduler", "remove(): wrapper removal failed: {}", e);
            }
        } else {
            self.ops_log("remove(): wrapper script absent, nothing to delete");
        }
        if let Err(e) = self.remove_state_file() {
            self.ops_log(&format!("remove(): state file removal failed: {}", e));
            dlog!("taskscheduler", "remove(): state file removal failed: {}", e);
        }
        self.clear_legacy_pid_file();
        self.ops_log_block("task after remove", &self.snapshot_task());
        self.ops_log_block("processes after remove", &self.snapshot_process());
        dlog!("taskscheduler", "remove() done");
        self.ops_log_section_end("remove", "OK");

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
