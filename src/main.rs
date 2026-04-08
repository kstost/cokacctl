#[macro_use]
mod core;
mod cli;
mod service;
mod tui;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    dlog!("main", "=== cokacctl started (v{}) ===", env!("CARGO_PKG_VERSION"));
    let cli = Cli::parse();

    match cli.command {
        Some(command) => {
            dlog!("main", "CLI mode: {:?}", command);
            run_cli(command);
        }
        None => {
            dlog!("main", "TUI mode");
            run_tui();
        }
    }
    dlog!("main", "=== cokacctl exiting ===");
}

fn run_cli(command: Commands) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    dlog!("main", "Tokio runtime created");

    let result = match command {
        Commands::Install => {
            dlog!("main::cli", "Running install command");
            rt.block_on(cli::install::run())
        }
        Commands::Update => {
            dlog!("main::cli", "Running update command");
            rt.block_on(cli::update::run())
        }
        Commands::Status => {
            dlog!("main::cli", "Running status command");
            run_status()
        }
        Commands::Start => {
            dlog!("main::cli", "Running start command");
            cli::service::start()
        }
        Commands::Stop => {
            dlog!("main::cli", "Running stop command");
            cli::service::stop()
        }
        Commands::Restart => {
            dlog!("main::cli", "Running restart command");
            cli::service::restart()
        }
        Commands::Remove => {
            dlog!("main::cli", "Running remove command");
            cli::service::remove()
        }
        Commands::Log => {
            dlog!("main::cli", "Running log command");
            cli::service::log()
        }
        Commands::Token { tokens } => {
            dlog!("main::cli", "Running token command");
            cli::service::token(tokens)
        }
        Commands::Uninstall { yes } => {
            dlog!("main::cli", "Running uninstall command (yes={})", yes);
            cli::uninstall::run(yes)
        }
    };

    if let Err(e) = result {
        dlog!("main::cli", "Command failed: {}", e);
        eprintln!("\x1b[31m  Error: {}\x1b[0m", e);
        std::process::exit(1);
    }
    dlog!("main::cli", "Command completed successfully");
}

fn run_status() -> Result<(), String> {
    dlog!("main::status", "Detecting platform...");
    let os = core::platform::Os::detect();
    let arch = core::platform::Arch::detect();
    dlog!("main::status", "Platform: {}/{}", os.as_str(), arch.as_str());
    println!("  Platform:  {}/{}", os.as_str(), arch.as_str());
    println!("  cokacctl:  v{}", env!("CARGO_PKG_VERSION"));

    dlog!("main::status", "Finding cokacdir...");
    match core::platform::find_cokacdir() {
        Some(path) => {
            dlog!("main::status", "cokacdir found at: {}", path.display());
            let version = core::version::installed_version(&path)
                .unwrap_or_else(|| "unknown".to_string());
            dlog!("main::status", "cokacdir version: {}", version);
            println!("  cokacdir:  v{} ({})", version, path.display());
        }
        None => {
            dlog!("main::status", "cokacdir not found");
            println!("  cokacdir:  not installed");
        }
    }

    dlog!("main::status", "Querying service status...");
    let mgr = service::manager();
    let status = mgr.status();
    dlog!("main::status", "Service status: {}", status);
    let symbol = match &status {
        service::ServiceStatus::Running => "\x1b[32m●\x1b[0m",
        service::ServiceStatus::Stopped => "\x1b[31m●\x1b[0m",
        service::ServiceStatus::NotInstalled => "\x1b[90m○\x1b[0m",
        service::ServiceStatus::Unknown(_) => "\x1b[33m●\x1b[0m",
    };
    println!("  Service:   {} {}", symbol, status);

    let config = core::config::Config::load();
    let active = config.active_tokens();
    dlog!("main::status", "Config loaded, tokens: {}/{}", active.len(), config.tokens.len());
    if !config.tokens.is_empty() {
        println!("  Tokens:    {} bot(s)", active.len());
    }
    if let Some(log) = mgr.log_path() {
        dlog!("main::status", "Log path: {}", log.display());
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

    dlog!("tui", "Setting up panic hook");
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

    dlog!("tui", "Clearing screen");
    print!("\x1B[2J\x1B[3J\x1B[H");
    std::io::Write::flush(&mut std::io::stdout()).ok();

    dlog!("tui", "Enabling raw mode");
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
    dlog!("tui", "Terminal setup complete");

    dlog!("tui", "Creating App...");
    let mut app = tui::app::App::new();
    dlog!("tui", "App created. Initial view: {:?}", app.view);

    dlog!("tui", "Starting update check thread");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        dlog!("tui::update_thread", "Thread started");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(core::version::latest_version());
        dlog!("tui::update_thread", "Result: {:?}", result);
        tx.send(result).ok();
    });

    dlog!("tui", "Loading initial log lines");
    let mgr = service::manager();
    let cached_log_path = mgr.log_path();
    dlog!("tui", "Log path: {:?}", cached_log_path);
    if let Some(ref log_path) = cached_log_path {
        app.log_lines = tui::log_viewer::load_log_lines(log_path, 200);
        dlog!("tui", "Loaded {} log lines", app.log_lines.len());
    }
    let mut log_file_size: u64 = cached_log_path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    let mut tick_count: u32 = 0;
    let mut prev_view = app.view.clone();

    dlog!("tui", "Entering main loop");
    loop {
        if app.view != prev_view {
            dlog!("tui", "View changed: {:?} -> {:?}, clearing terminal", prev_view, app.view);
            terminal.clear().ok();
            prev_view = app.view.clone();
        }
        terminal
            .draw(|f| tui::draw::draw(f, &app))
            .expect("Failed to draw");

        if !tui::event::handle_events(&mut app) {
            dlog!("tui", "handle_events returned false, breaking");
            break;
        }
        if !app.running {
            dlog!("tui", "app.running is false, breaking");
            break;
        }

        app.poll_progress();
        app.poll_service_action();
        app.expire_status();

        if app.checking_update {
            if let Ok(result) = rx.try_recv() {
                app.checking_update = false;
                dlog!("tui", "Update check completed: {:?}", result);
                app.latest_version = result;
            }
        }

        tick_count += 1;
        if tick_count % 10 == 0 {
            if let Some(ref log_path) = cached_log_path {
                let new_lines = tui::log_viewer::read_new_lines(log_path, &mut log_file_size);
                if !new_lines.is_empty() {
                    dlog!("tui", "Read {} new log lines", new_lines.len());
                }
                app.log_lines.extend(new_lines);
                if app.log_lines.len() > 500 {
                    let excess = app.log_lines.len() - 500;
                    app.log_lines.drain(..excess);
                }
                if app.log_scroll_offset > app.log_lines.len() {
                    app.log_scroll_offset = app.log_lines.len();
                }
            }
        }
        if tick_count % 25 == 0 {
            dlog!("tui", "Periodic status refresh (tick {})", tick_count);
            app.refresh_status();
        }
    }

    dlog!("tui", "Restoring terminal");
    disable_raw_mode().expect("Failed to disable raw mode");
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0),
        crossterm::cursor::Show
    )
    .expect("Failed to restore terminal");
    dlog!("tui", "Terminal restored");
}
