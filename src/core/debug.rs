use std::io::Write;
use std::sync::Mutex;
use std::sync::OnceLock;

/// Master switch for debug logging. Set to false to disable all debug output.
pub const DEBUG_ENABLED: bool = false;

static LOG_FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();

fn get_log_file() -> &'static Mutex<Option<std::fs::File>> {
    LOG_FILE.get_or_init(|| {
        if !DEBUG_ENABLED {
            return Mutex::new(None);
        }
        let home = dirs::home_dir().unwrap_or_default();
        let dir = home.join(".cokacdir").join("debug");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("cokacctl.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        Mutex::new(file)
    })
}

pub fn log(module: &str, msg: &str) {
    if !DEBUG_ENABLED {
        return;
    }
    let guard = get_log_file();
    if let Ok(mut lock) = guard.lock() {
        if let Some(ref mut f) = *lock {
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

#[macro_export]
macro_rules! dlog {
    ($module:expr, $($arg:tt)*) => {
        $crate::core::debug::log($module, &format!($($arg)*))
    };
}
