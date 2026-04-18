use std::path::PathBuf;

/// Load the last N lines from a log file.
///
/// Uses a lossy UTF-8 decode so the viewer stays usable even when the log
/// momentarily contains non-UTF8 bytes (partial multi-byte writes, etc.).
pub fn load_log_lines(path: &PathBuf, max_lines: usize) -> Vec<String> {
    dlog!("log_viewer", "load_log_lines: path={}, max={}", path.display(), max_lines);
    let bytes = match std::fs::read(path) {
        Ok(b) => {
            dlog!("log_viewer", "load_log_lines: read {} bytes", b.len());
            b
        }
        Err(e) => {
            dlog!("log_viewer", "load_log_lines: read failed: {}", e);
            return Vec::new();
        }
    };
    let content = String::from_utf8_lossy(&bytes);
    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    dlog!("log_viewer", "load_log_lines: parsed {} lines", lines.len());
    if lines.len() > max_lines {
        lines[lines.len() - max_lines..].to_vec()
    } else {
        lines
    }
}

/// Check if the file has grown since the given position. Returns new lines.
pub fn read_new_lines(path: &PathBuf, last_size: &mut u64) -> Vec<String> {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            dlog!("log_viewer", "read_new_lines: metadata failed ({}): {}", path.display(), e);
            return Vec::new();
        }
    };
    let current_size = metadata.len();
    if current_size < *last_size {
        dlog!(
            "log_viewer",
            "read_new_lines: file shrunk ({} -> {}), resetting offset",
            *last_size,
            current_size
        );
        *last_size = 0;
    }
    if current_size <= *last_size {
        return Vec::new();
    }

    // Read raw bytes so non-UTF8 content (e.g. partial multi-byte writes)
    // doesn't cause the tail to silently go dead.
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            dlog!("log_viewer", "read_new_lines: read failed: {}", e);
            return Vec::new();
        }
    };
    let start = *last_size as usize;
    if start >= bytes.len() {
        dlog!(
            "log_viewer",
            "read_new_lines: start ({}) >= bytes.len ({}), no new content",
            start,
            bytes.len()
        );
        *last_size = current_size;
        return Vec::new();
    }
    let new_content = String::from_utf8_lossy(&bytes[start..]);
    *last_size = current_size;
    let result: Vec<String> = new_content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();
    dlog!(
        "log_viewer",
        "read_new_lines: {}B new, {} new lines (total offset now {})",
        bytes.len() - start,
        result.len(),
        *last_size
    );
    result
}
