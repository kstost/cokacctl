use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;

/// Master switch for debug logging. Always on so every cokacctl action is
/// captured without needing environment setup.
pub const DEBUG_ENABLED: bool = true;

/// Rotate the active log file once it grows past this size. Rotated files are
/// renamed with a timestamp suffix and kept forever (no retention policy) —
/// operators are expected to manage disk usage manually.
const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024;

struct LogState {
    file: Option<std::fs::File>,
    path: PathBuf,
}

static LOG_STATE: OnceLock<Mutex<LogState>> = OnceLock::new();

fn get_log_state() -> &'static Mutex<LogState> {
    LOG_STATE.get_or_init(|| {
        let home = dirs::home_dir().unwrap_or_default();
        let dir = home.join(".cokacdir").join("debug");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("cokacctl.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        Mutex::new(LogState { file, path })
    })
}

/// Rotate the current log to `cokacctl.log.<timestamp>` when it exceeds
/// MAX_LOG_SIZE. Best-effort: any failure (permission, disk full, rename
/// conflict) leaves the current file in place so logging keeps working.
fn rotate_if_needed(state: &mut LogState) {
    let size = match state.file.as_ref().and_then(|f| f.metadata().ok()) {
        Some(m) => m.len(),
        None => return,
    };
    if size < MAX_LOG_SIZE {
        return;
    }

    // Drop the active handle so Windows rename isn't blocked by an open file.
    state.file = None;

    let ts = chrono::Local::now().format("%Y-%m-%d_%H%M%S%.3f").to_string();
    let rotated = state.path.with_extension(format!("log.{}", ts));
    let _ = std::fs::rename(&state.path, &rotated);

    // Reopen a fresh log file. If this fails we silently proceed — subsequent
    // writes will see `file = None` and skip.
    state.file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state.path)
        .ok();
}

pub fn log(module: &str, msg: &str) {
    if !DEBUG_ENABLED {
        return;
    }
    let guard = get_log_state();
    if let Ok(mut state) = guard.lock() {
        rotate_if_needed(&mut state);
        if let Some(ref mut f) = state.file {
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let _ = writeln!(f, "[{}] [{}] {}", now, module, msg);
            let _ = f.flush();
        }
    }
}

/// Convert command output bytes to a readable string.
/// On Windows, system commands output in the OEM code page (e.g. CP949 for Korean).
/// If UTF-8 decoding fails, fall back to a lossy latin1 decode so the log is still readable.
pub fn decode_output(bytes: &[u8]) -> String {
    match String::from_utf8(bytes.to_vec()) {
        Ok(s) => s,
        Err(_) => {
            // Fallback: decode each byte as latin1 (preserves all bytes as chars)
            bytes.iter().map(|&b| b as char).collect()
        }
    }
}

/// Log the full result of an external command: exit code and the entire
/// decoded stdout/stderr. Empty streams are omitted to keep the log readable.
/// Use this at every `Command::new(...).output()` site so failures can be
/// reconstructed from the log alone.
pub fn log_output(module: &str, label: &str, output: &std::process::Output) {
    if !DEBUG_ENABLED {
        return;
    }
    log(
        module,
        &format!(
            "[cmd] {} -> exit={:?} stdout={}B stderr={}B",
            label,
            output.status.code(),
            output.stdout.len(),
            output.stderr.len()
        ),
    );
    let stdout = decode_output(&output.stdout);
    let stdout_trim = stdout.trim();
    if !stdout_trim.is_empty() {
        log(module, &format!("[cmd] {} stdout:\n{}", label, stdout_trim));
    }
    let stderr = decode_output(&output.stderr);
    let stderr_trim = stderr.trim();
    if !stderr_trim.is_empty() {
        log(module, &format!("[cmd] {} stderr:\n{}", label, stderr_trim));
    }
}

/// Log the result of an external command that was invoked via `.status()` —
/// i.e. no stdout/stderr capture is available, only the exit code.
pub fn log_status(module: &str, label: &str, status: &std::process::ExitStatus) {
    if !DEBUG_ENABLED {
        return;
    }
    log(
        module,
        &format!(
            "[cmd] {} -> exit={:?} success={}",
            label,
            status.code(),
            status.success()
        ),
    );
}

#[macro_export]
macro_rules! dlog {
    ($module:expr, $($arg:tt)*) => {
        $crate::core::debug::log($module, &format!($($arg)*))
    };
}
