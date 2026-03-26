use crate::cli::ServiceAction;
use crate::core::config::Config;
use crate::core::platform;
use crate::service::{self, ServiceManager, ServiceStatus};
use std::io::{BufRead, BufReader};

pub fn run(action: ServiceAction) -> Result<(), String> {
    let mgr = service::manager();

    match action {
        ServiceAction::Start { tokens } => {
            let tokens = dedup_tokens(tokens);
            let binary_path = platform::find_cokacdir().ok_or(
                "cokacdir not found in PATH. Run 'cokacctl install' first.".to_string(),
            )?;

            println!("  Starting cokacdir service...");
            println!("  Binary: {}", binary_path.display());
            println!("  Tokens: {} bot(s)", tokens.len());

            mgr.start(&binary_path, &tokens)?;

            // Save tokens to config
            let mut config = Config::load();
            config.tokens = tokens;
            config.install_path = Some(binary_path.to_string_lossy().to_string());
            config.save()?;

            println!("  Service started.");
            print_management_hints(&*mgr);
            Ok(())
        }

        ServiceAction::Stop => {
            println!("  Stopping cokacdir service...");
            mgr.stop()?;
            println!("  Service stopped.");
            Ok(())
        }

        ServiceAction::Restart => {
            let config = Config::load();
            if config.tokens.is_empty() {
                return Err(
                    "No tokens configured. Use 'cokacctl service start <TOKEN>' first.".into(),
                );
            }
            let binary_path = platform::find_cokacdir().ok_or(
                "cokacdir not found in PATH.".to_string(),
            )?;

            println!("  Restarting cokacdir service...");
            mgr.restart(&binary_path, &config.tokens)?;
            println!("  Service restarted.");
            Ok(())
        }

        ServiceAction::Remove => {
            println!("  Removing cokacdir service...");
            mgr.remove()?;
            println!("  Service removed.");
            Ok(())
        }

        ServiceAction::Status => {
            let status = mgr.status();
            let symbol = match &status {
                ServiceStatus::Running => "\x1b[32m●\x1b[0m",
                ServiceStatus::Stopped => "\x1b[31m●\x1b[0m",
                ServiceStatus::NotInstalled => "\x1b[90m○\x1b[0m",
                ServiceStatus::Unknown(_) => "\x1b[33m●\x1b[0m",
            };
            println!("  Service: {} {}", symbol, status);

            let config = Config::load();
            if !config.tokens.is_empty() {
                println!("  Tokens:  {} bot(s) configured", config.tokens.len());
            }
            if let Some(log) = mgr.log_path() {
                println!("  Log:     {}", log.display());
            }
            Ok(())
        }

        ServiceAction::Log => {
            let log_path = mgr
                .log_path()
                .ok_or("Log file path not available.".to_string())?;
            if !log_path.exists() {
                return Err(format!("Log file not found: {}", log_path.display()));
            }
            println!("  Tailing {}...\n", log_path.display());
            tail_file(&log_path)
        }

        ServiceAction::Token { tokens } => {
            let tokens = dedup_tokens(tokens);
            let binary_path = platform::find_cokacdir().ok_or(
                "cokacdir not found in PATH.".to_string(),
            )?;

            println!("  Updating tokens ({} bot(s))...", tokens.len());
            mgr.restart(&binary_path, &tokens)?;

            let mut config = Config::load();
            config.tokens = tokens;
            config.save()?;

            println!("  Tokens updated and service restarted.");
            Ok(())
        }
    }
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
        println!("  Log: cokacctl service log");
        println!("       {}", log.display());
    }
}

fn tail_file(path: &std::path::Path) -> Result<(), String> {
    // Print last 20 lines, then follow
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read log: {}", e))?;
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > 20 { lines.len() - 20 } else { 0 };
    for line in &lines[start..] {
        println!("{}", line);
    }

    // Follow new lines
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Cannot open log: {}", e))?;
    let metadata = file.metadata()
        .map_err(|e| format!("Cannot get file metadata: {}", e))?;
    let mut pos = metadata.len();

    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let current_len = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };
        if current_len > pos {
            let file = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let mut reader = BufReader::new(file);
            use std::io::Seek;
            if reader.seek(std::io::SeekFrom::Start(pos)).is_err() {
                continue;
            }
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                print!("{}", line);
                line.clear();
            }
            pos = current_len;
        }
    }
}
