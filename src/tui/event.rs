use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::time::Duration;

use super::app::{App, ProgressAction, View};
use crate::core::config::Config;
use crate::core::platform;
use crate::service;

/// Poll for keyboard events and handle them.
/// Returns true if the app should continue running.
pub fn handle_events(app: &mut App) -> bool {
    if event::poll(Duration::from_millis(200)).unwrap_or(false) {
        if let Ok(Event::Key(key)) = event::read() {
            if key.kind != KeyEventKind::Press {
                return true;
            }
            dlog!("event", "Key: {:?} (modifiers: {:?}), view: {:?}", key.code, key.modifiers, app.view);
            return handle_key(app, key);
        }
    }
    true
}

fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    // Clear status message on any key
    app.status_message = None;

    match app.view {
        View::Welcome => handle_welcome_key(app, key),
        View::TokenInput => handle_token_input_key(app, key),
        View::Progress => handle_progress_key(app, key),
        View::Dashboard => handle_dashboard_key(app, key),
        View::LogFullscreen => handle_log_key(app, key),
    }
}

fn handle_welcome_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            dlog!("event", "Welcome: quit");
            app.running = false;
            return false;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            dlog!("event", "Welcome: Ctrl+C");
            app.running = false;
            return false;
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            dlog!("event", "Welcome: install");
            app.start_progress(ProgressAction::Install);
        }
        _ => {}
    }
    true
}

fn handle_progress_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            dlog!("event", "Progress: Ctrl+C");
            app.running = false;
            return false;
        }
        _ => {
            // Only respond to keys when operation is done
            if app.progress_done.is_some() {
                let was_install = app.progress_action == Some(ProgressAction::Install);
                let succeeded = app.progress_done.as_ref().map(|r| r.is_ok()).unwrap_or(false);
                dlog!("event", "Progress done - was_install: {}, succeeded: {}", was_install, succeeded);

                app.refresh_cokacdir_info();
                app.progress_rx = None;

                if was_install && succeeded && app.cokacdir_version.is_some() {
                    if app.config.tokens.is_empty() {
                        dlog!("event", "Install done, entering token input");
                        app.enter_token_input();
                    } else {
                        dlog!("event", "Install done, going to dashboard");
                        app.view = View::Dashboard;
                        app.set_status("Install completed", false);
                    }
                } else if succeeded {
                    dlog!("event", "Update done, going to dashboard");
                    app.view = View::Dashboard;
                    app.set_status("Update completed", false);
                } else if was_install && app.cokacdir_version.is_none() {
                    dlog!("event", "Install failed, back to welcome");
                    app.view = View::Welcome;
                } else {
                    app.view = View::Dashboard;
                }
            }
        }
    }
    true
}

fn handle_token_input_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            dlog!("event", "TokenInput: Esc - saving {} tokens", app.token_list.len());
            // Save token changes to config
            let mut config = Config::load();
            config.tokens = app.token_list.clone();
            config.save().ok();

            app.token_input.clear();
            app.token_list.clear();
            app.token_cursor = None;
            app.refresh_status();
            if app.cokacdir_version.is_some() {
                app.view = View::Dashboard;
            } else {
                app.view = View::Welcome;
            }
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            dlog!("event", "TokenInput: Ctrl+C");
            app.running = false;
            return false;
        }
        KeyCode::Up => {
            match app.token_cursor {
                None => {
                    if !app.token_list.is_empty() {
                        app.token_cursor = Some(app.token_list.len() - 1);
                    }
                }
                Some(0) => {}
                Some(i) => {
                    app.token_cursor = Some(i - 1);
                }
            }
        }
        KeyCode::Down => {
            match app.token_cursor {
                Some(i) if i + 1 < app.token_list.len() => {
                    app.token_cursor = Some(i + 1);
                }
                Some(_) => {
                    app.token_cursor = None;
                }
                None => {}
            }
        }
        KeyCode::Delete | KeyCode::Backspace if app.token_cursor.is_some() => {
            if let Some(i) = app.token_cursor {
                dlog!("event", "TokenInput: removing token at index {}", i);
                app.token_list.remove(i);
                if app.token_list.is_empty() {
                    app.token_cursor = None;
                } else if i >= app.token_list.len() {
                    app.token_cursor = Some(app.token_list.len() - 1);
                }
            }
        }
        KeyCode::Enter => {
            if app.token_cursor.is_none() {
                let token = app.token_input.trim().to_string();
                if !token.is_empty() {
                    dlog!("event", "TokenInput: adding token (len {})", token.len());
                    if !app.token_list.contains(&token) {
                        app.token_list.push(token);
                    }
                    app.token_input.clear();
                }
            }
        }
        KeyCode::Backspace if app.token_cursor.is_none() => {
            app.token_input.pop();
        }
        KeyCode::Char(c) if app.token_cursor.is_none() => {
            app.token_input.push(c);
        }
        _ => {}
    }
    true
}

fn handle_dashboard_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            dlog!("event", "Dashboard: quit");
            app.running = false;
            return false;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            dlog!("event", "Dashboard: Ctrl+C");
            app.running = false;
            return false;
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            dlog!("event", "Dashboard: log fullscreen");
            app.view = View::LogFullscreen;
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            dlog!("event", "Dashboard: start service");
            action_start(app);
        }
        KeyCode::Char('t') | KeyCode::Char('T') => {
            dlog!("event", "Dashboard: stop service");
            action_stop(app);
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            dlog!("event", "Dashboard: restart service");
            action_restart(app);
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            dlog!("event", "Dashboard: remove service");
            action_remove(app);
        }
        KeyCode::Char('k') | KeyCode::Char('K') => {
            dlog!("event", "Dashboard: token input");
            app.enter_token_input();
        }
        KeyCode::Char('u') | KeyCode::Char('U') => {
            dlog!("event", "Dashboard: update");
            app.start_progress(ProgressAction::Update);
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            dlog!("event", "Dashboard: install");
            app.start_progress(ProgressAction::Install);
        }
        _ => {}
    }
    true
}

fn handle_log_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Esc => {
            dlog!("event", "Log: back to dashboard");
            app.log_scroll_offset = 0;
            app.view = View::Dashboard;
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            dlog!("event", "Log: quit");
            app.running = false;
            return false;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.running = false;
            return false;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.log_scroll_offset = app.log_scroll_offset.saturating_add(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.log_scroll_offset = app.log_scroll_offset.saturating_sub(1);
        }
        KeyCode::PageUp => {
            app.log_scroll_offset = app.log_scroll_offset.saturating_add(20);
        }
        KeyCode::PageDown => {
            app.log_scroll_offset = app.log_scroll_offset.saturating_sub(20);
        }
        KeyCode::Home => {
            app.log_scroll_offset = app.log_lines.len();
        }
        KeyCode::End => {
            app.log_scroll_offset = 0;
        }
        _ => {}
    }
    true
}

fn action_start(app: &mut App) {
    dlog!("event::action_start", "Loading config...");
    let config = Config::load();
    if config.tokens.is_empty() {
        dlog!("event::action_start", "No tokens, entering token input");
        app.enter_token_input();
        return;
    }
    dlog!("event::action_start", "Finding cokacdir binary...");
    let binary_path = match platform::find_cokacdir() {
        Some(p) => {
            dlog!("event::action_start", "Found: {}", p.display());
            p
        }
        None => {
            dlog!("event::action_start", "cokacdir not found");
            app.set_status("cokacdir not found. Press [I] to install", true);
            return;
        }
    };
    dlog!("event::action_start", "Calling mgr.start() with {} tokens...", config.tokens.len());
    let mgr = service::manager();
    match mgr.start(&binary_path, &config.tokens) {
        Ok(()) => {
            dlog!("event::action_start", "Service started successfully");
            app.set_status("Service started", false);
            app.refresh_status();
        }
        Err(e) => {
            dlog!("event::action_start", "Start failed: {}", e);
            app.set_status(&format!("Start failed: {}", e), true);
        }
    }
}



fn action_stop(app: &mut App) {
    dlog!("event::action_stop", "Stopping service...");
    let mgr = service::manager();
    match mgr.stop() {
        Ok(()) => {
            dlog!("event::action_stop", "Service stopped");
            app.set_status("Service stopped", false);
            app.refresh_status();
        }
        Err(e) => {
            dlog!("event::action_stop", "Stop failed: {}", e);
            app.set_status(&format!("Stop failed: {}", e), true);
        }
    }
}

fn action_restart(app: &mut App) {
    dlog!("event::action_restart", "Restarting service...");
    let config = Config::load();
    if config.tokens.is_empty() {
        dlog!("event::action_restart", "No tokens configured");
        app.set_status("No tokens configured", true);
        return;
    }
    let binary_path = match platform::find_cokacdir() {
        Some(p) => p,
        None => {
            dlog!("event::action_restart", "cokacdir not found");
            app.set_status("cokacdir not found", true);
            return;
        }
    };
    let mgr = service::manager();
    match mgr.restart(&binary_path, &config.tokens) {
        Ok(()) => {
            dlog!("event::action_restart", "Service restarted");
            app.set_status("Service restarted", false);
            app.refresh_status();
        }
        Err(e) => {
            dlog!("event::action_restart", "Restart failed: {}", e);
            app.set_status(&format!("Restart failed: {}", e), true);
        }
    }
}

fn action_remove(app: &mut App) {
    dlog!("event::action_remove", "Removing service...");
    let mgr = service::manager();
    match mgr.remove() {
        Ok(()) => {
            dlog!("event::action_remove", "Service removed");
            app.set_status("Service removed", false);
            app.refresh_status();
        }
        Err(e) => {
            dlog!("event::action_remove", "Remove failed: {}", e);
            app.set_status(&format!("Remove failed: {}", e), true);
        }
    }
}
