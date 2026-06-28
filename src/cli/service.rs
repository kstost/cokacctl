use crate::core::config::Config;
use crate::core::platform;
use crate::service::{self, ServiceManager};

pub fn start() -> Result<(), String> {
    dlog!("cli::service", "start: begin");
    let config = Config::load();
    let tokens = config.active_tokens();
    if tokens.is_empty() {
        dlog!(
            "cli::service",
            "start: refused — no active tokens (total {})",
            config.tokens.len()
        );
        return Err("No active tokens configured. Use 'cokacctl token <TOKEN>' first.".into());
    }
    dlog!("cli::service", "start: {} active tokens", tokens.len());
    let binary_path = platform::find_cokacdir().ok_or_else(|| {
        dlog!("cli::service", "start: refused — cokacdir not found");
        "cokacdir not found in PATH. Run 'cokacctl install' first.".to_string()
    })?;
    let mgr = service::manager();
    let target = management_target(&*mgr);
    dlog!("cli::service", "Binary: {}", binary_path.display());

    println!("  Starting cokacdir {}...", target);
    println!("  Binary: {}", binary_path.display());
    println!("  Tokens: {} bot(s)", tokens.len());

    mgr.start(&binary_path, &tokens).map_err(|e| {
        dlog!("cli::service", "start: mgr.start failed: {}", e);
        e
    })?;

    dlog!("cli::service", "Service started");
    println!("  cokacdir {} started.", target);
    print_management_hints(&*mgr);
    Ok(())
}

pub fn stop() -> Result<(), String> {
    dlog!("cli::service", "stop: begin");
    let mgr = service::manager();
    let target = management_target(&*mgr);
    println!("  Stopping cokacdir {}...", target);
    mgr.stop().map_err(|e| {
        dlog!("cli::service", "stop: mgr.stop failed: {}", e);
        e
    })?;
    dlog!("cli::service", "Service stopped");
    println!("  cokacdir {} stopped.", target);
    Ok(())
}

pub fn restart() -> Result<(), String> {
    dlog!("cli::service", "restart: begin");
    let config = Config::load();
    let tokens = config.active_tokens();
    if tokens.is_empty() {
        dlog!("cli::service", "restart: refused — no active tokens");
        return Err("No active tokens configured. Use 'cokacctl token <TOKEN>' first.".into());
    }
    let binary_path = platform::find_cokacdir().ok_or_else(|| {
        dlog!("cli::service", "restart: refused — cokacdir not found");
        "cokacdir not found in PATH. Run 'cokacctl install' first.".to_string()
    })?;
    let mgr = service::manager();
    let target = management_target(&*mgr);
    dlog!(
        "cli::service",
        "restart: bin={} tokens={}",
        binary_path.display(),
        tokens.len()
    );

    println!("  Restarting cokacdir {}...", target);
    mgr.restart(&binary_path, &tokens).map_err(|e| {
        dlog!("cli::service", "restart: mgr.restart failed: {}", e);
        e
    })?;
    dlog!("cli::service", "Service restarted");
    println!("  cokacdir {} restarted.", target);
    Ok(())
}

pub fn remove() -> Result<(), String> {
    dlog!("cli::service", "remove: begin");
    let mgr = service::manager();
    if mgr.status().service_registration_unavailable() {
        return Err(
            "Service registration is unavailable in direct mode. Use 'cokacctl stop' to stop the direct process."
                .into(),
        );
    }
    println!("  Removing cokacdir service registration...");
    mgr.remove().map_err(|e| {
        dlog!("cli::service", "remove: mgr.remove failed: {}", e);
        e
    })?;
    dlog!("cli::service", "Service removed");
    println!("  Service registration removed.");
    Ok(())
}

pub fn log() -> Result<(), String> {
    dlog!("cli::service", "log: begin");
    let mgr = service::manager();
    let log_path = mgr.log_path().ok_or_else(|| {
        dlog!("cli::service", "log: log_path() returned None");
        "Log file path not available.".to_string()
    })?;
    if !log_path.exists() {
        dlog!(
            "cli::service",
            "log: file missing at {}",
            log_path.display()
        );
        return Err(format!("Log file not found: {}", log_path.display()));
    }
    dlog!("cli::service", "Tailing: {}", log_path.display());
    println!("  Tailing {}...\n", log_path.display());
    tail_file(&log_path)
}

pub fn token(tokens: Vec<String>) -> Result<(), String> {
    let raw_count = tokens.len();
    let tokens = dedup_tokens(tokens);
    dlog!(
        "cli::service",
        "token: raw={} dedup={}",
        raw_count,
        tokens.len()
    );

    let mut config = Config::load();
    let prev = config.tokens.len();
    config.tokens = tokens.clone();
    config.disabled_tokens.clear();
    config.token_names.retain(|t, _| tokens.contains(t));
    config.token_bot_info.retain(|t, _| tokens.contains(t));
    config.save().map_err(|e| {
        dlog!("cli::service", "token: config.save failed: {}", e);
        e
    })?;

    dlog!(
        "cli::service",
        "Tokens saved (prev={} new={})",
        prev,
        tokens.len()
    );
    println!("  {} bot token(s) registered.", tokens.len());
    Ok(())
}

fn dedup_tokens(tokens: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    for t in tokens {
        if !seen.contains(&t) {
            seen.push(t);
        }
    }
    seen
}

fn print_management_hints(mgr: &dyn ServiceManager) {
    if let Some(log) = mgr.log_path() {
        println!();
        println!("  Log: cokacctl log");
        println!("       {}", log.display());
    }
}

fn management_target(mgr: &dyn ServiceManager) -> &'static str {
    if mgr.status().service_registration_unavailable() {
        "direct process"
    } else {
        "service"
    }
}

fn tail_file(path: &std::path::Path) -> Result<(), String> {
    // Read bytes and lossy-decode so non-UTF8 bytes in the log don't cause
    // the whole command to fail.
    let bytes = std::fs::read(path).map_err(|e| {
        dlog!("cli::service", "tail: initial read failed: {}", e);
        format!("Cannot read log: {}", e)
    })?;
    let content = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > 20 {
        lines.len() - 20
    } else {
        0
    };
    dlog!(
        "cli::service",
        "tail: initial dump {} lines (of {})",
        lines.len() - start,
        lines.len()
    );
    for line in &lines[start..] {
        println!("{}", line);
    }

    let file = std::fs::File::open(path).map_err(|e| {
            dlog!("cli::service", "tail: open failed: {}", e);
            format!("Cannot open log: {}", e)
        })?;
    let metadata = file.metadata().map_err(|e| {
            dlog!("cli::service", "tail: metadata failed: {}", e);
            format!("Cannot get file metadata: {}", e)
        })?;
    let mut pos = metadata.len();
    dlog!(
        "cli::service",
        "tail: entering follow loop from offset {}",
        pos
    );

    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let current_len = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };
        if current_len < pos {
            dlog!(
                "cli::service",
                "tail: file shrank ({} -> {}), rewinding",
                pos,
                current_len
            );
            pos = 0;
        }
        if current_len > pos {
            let mut file = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            use std::io::{Read, Seek};
            if file.seek(std::io::SeekFrom::Start(pos)).is_err() {
                continue;
            }
            // Read raw bytes to avoid failing on non-UTF8 content, then
            // lossy-decode. This is the CLI tail, so printing the batch in
            // one go is acceptable.
            let mut buf = Vec::new();
            if file.read_to_end(&mut buf).is_ok() {
                print!("{}", String::from_utf8_lossy(&buf));
            }
            pos = current_len;
        }
    }
}
