mod cli;
mod core;
mod service;
mod tui;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(command) => run_cli(command),
        None => run_tui(),
    }
}

fn run_cli(command: Commands) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    let result = match command {
        Commands::Install => rt.block_on(cli::install::run()),
        Commands::Update => rt.block_on(cli::update::run()),
        Commands::Service { action } => cli::service::run(action),
        Commands::Status => run_status(),
    };

    if let Err(e) = result {
        eprintln!("\x1b[31m  Error: {}\x1b[0m", e);
        std::process::exit(1);
    }
}

fn run_status() -> Result<(), String> {
    let os = core::platform::Os::detect();
    let arch = core::platform::Arch::detect();
    println!("  Platform:  {}/{}", os.as_str(), arch.as_str());
    println!("  cokacctl:  v{}", env!("CARGO_PKG_VERSION"));

    match core::platform::find_cokacdir() {
        Some(path) => {
            let version = core::version::installed_version(&path)
                .unwrap_or_else(|| "unknown".to_string());
            println!("  cokacdir:  v{} ({})", version, path.display());
        }
        None => {
            println!("  cokacdir:  not installed");
        }
    }

    let mgr = service::manager();
    let status = mgr.status();
    let symbol = match &status {
        service::ServiceStatus::Running => "\x1b[32m●\x1b[0m",
        service::ServiceStatus::Stopped => "\x1b[31m●\x1b[0m",
        service::ServiceStatus::NotInstalled => "\x1b[90m○\x1b[0m",
        service::ServiceStatus::Unknown(_) => "\x1b[33m●\x1b[0m",
    };
    println!("  Service:   {} {}", symbol, status);

    let config = core::config::Config::load();
    if !config.tokens.is_empty() {
        println!("  Tokens:    {} bot(s)", config.tokens.len());
    }
    if let Some(log) = mgr.log_path() {
        if log.exists() {
            println!("  Log:       {}", log.display());
        }
    }

    Ok(())
}

fn run_tui() {
    use crossterm::{
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::sync::mpsc;

    // Setup panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            LeaveAlternateScreen,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
            crossterm::cursor::MoveTo(0, 0),
            crossterm::cursor::Show
        );
        original_hook(panic_info);
    }));

    // Clear screen before entering TUI
    print!("\x1B[2J\x1B[3J\x1B[H");
    std::io::Write::flush(&mut std::io::stdout()).ok();

    // Setup terminal
    enable_raw_mode().expect("Failed to enable raw mode");
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0),
        EnterAlternateScreen
    )
    .expect("Failed to enter alternate screen");
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");
    terminal.clear().expect("Failed to clear terminal");

    // Create app
    let mut app = tui::app::App::new();

    // Check for updates in background thread
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(core::version::latest_version());
        tx.send(result).ok();
    });

    // Load initial log lines
    let mgr = service::manager();
    if let Some(log_path) = mgr.log_path() {
        app.log_lines = tui::log_viewer::load_log_lines(&log_path, 200);
    }
    let mut log_file_size: u64 = mgr
        .log_path()
        .and_then(|p| std::fs::metadata(&p).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    let mut tick_count: u32 = 0;

    // Main loop
    loop {
        // Draw
        terminal
            .draw(|f| tui::draw::draw(f, &app))
            .expect("Failed to draw");

        // Handle events
        if !tui::event::handle_events(&mut app) {
            break;
        }
        if !app.running {
            break;
        }

        // Poll progress messages
        app.poll_progress();

        // Expire status messages
        app.expire_status();

        // Check if update check completed
        if app.checking_update {
            if let Ok(result) = rx.try_recv() {
                app.checking_update = false;
                app.latest_version = result;
            }
        }

        // Periodic tasks
        tick_count += 1;
        if tick_count % 10 == 0 {
            // Refresh log every ~2 seconds
            if let Some(log_path) = mgr.log_path() {
                let new_lines = tui::log_viewer::read_new_lines(&log_path, &mut log_file_size);
                app.log_lines.extend(new_lines);
                if app.log_lines.len() > 500 {
                    let excess = app.log_lines.len() - 500;
                    app.log_lines.drain(..excess);
                }
                // Clamp scroll offset
                if app.log_scroll_offset > app.log_lines.len() {
                    app.log_scroll_offset = app.log_lines.len();
                }
            }
        }
        if tick_count % 25 == 0 {
            // Refresh service status every ~5 seconds
            app.refresh_status();
        }
    }

    // Restore terminal
    disable_raw_mode().expect("Failed to disable raw mode");
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0),
        crossterm::cursor::Show
    )
    .expect("Failed to restore terminal");
}
