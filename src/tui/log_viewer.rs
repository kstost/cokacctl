use std::path::PathBuf;

/// Load the last N lines from a log file.
pub fn load_log_lines(path: &PathBuf, max_lines: usize) -> Vec<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
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
        Err(_) => return Vec::new(),
    };
    let current_size = metadata.len();
    if current_size <= *last_size {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let all_bytes = content.as_bytes();
    let start = *last_size as usize;
    if start >= all_bytes.len() {
        *last_size = current_size;
        return Vec::new();
    }
    let new_content = String::from_utf8_lossy(&all_bytes[start..]);
    *last_size = current_size;
    new_content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect()
}
