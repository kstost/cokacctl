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
            app.running = false;
            return false;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.running = false;
            return false;
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            app.start_progress(ProgressAction::Install);
        }
        _ => {}
    }
    true
}

fn handle_progress_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.running = false;
            return false;
        }
        _ => {
            // Only respond to keys when operation is done
            if app.progress_done.is_some() {
                let was_install = app.progress_action == Some(ProgressAction::Install);
                let succeeded = app.progress_done.as_ref().map(|r| r.is_ok()).unwrap_or(false);

                app.refresh_cokacdir_info();
                app.progress_rx = None;

                if was_install && succeeded && app.cokacdir_version.is_some() {
                    if app.config.tokens.is_empty() {
                        app.enter_token_input();
                    } else {
                        app.view = View::Dashboard;
                        app.set_status("Install completed", false);
                    }
                } else if succeeded {
                    app.view = View::Dashboard;
                    app.set_status("Update completed", false);
                } else if was_install && app.cokacdir_version.is_none() {
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
            app.running = false;
            return false;
        }
        KeyCode::Up => {
            // Move cursor up: input → last token → ... → first token
            match app.token_cursor {
                None => {
                    // From input → select last token
                    if !app.token_list.is_empty() {
                        app.token_cursor = Some(app.token_list.len() - 1);
                    }
                }
                Some(0) => {
                    // Already at first token, stay
                }
                Some(i) => {
                    app.token_cursor = Some(i - 1);
                }
            }
        }
        KeyCode::Down => {
            // Move cursor down: first token → ... → last token → input
            match app.token_cursor {
                Some(i) if i + 1 < app.token_list.len() => {
                    app.token_cursor = Some(i + 1);
                }
                Some(_) => {
                    // Past last token → focus input
                    app.token_cursor = None;
                }
                None => {
                    // Already on input, stay
                }
            }
        }
        KeyCode::Delete | KeyCode::Backspace if app.token_cursor.is_some() => {
            // Remove selected token
            if let Some(i) = app.token_cursor {
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
                // On input field — add token
                let token = app.token_input.trim().to_string();
                if !token.is_empty() {
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
            app.running = false;
            return false;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.running = false;
            return false;
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            app.view = View::LogFullscreen;
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            action_start(app);
        }
        KeyCode::Char('t') | KeyCode::Char('T') => {
            action_stop(app);
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            action_restart(app);
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            action_remove(app);
        }
        KeyCode::Char('k') | KeyCode::Char('K') => {
            app.enter_token_input();
        }
        KeyCode::Char('u') | KeyCode::Char('U') => {
            app.start_progress(ProgressAction::Update);
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            app.start_progress(ProgressAction::Install);
        }
        _ => {}
    }
    true
}

fn handle_log_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Esc => {
            app.log_scroll_offset = 0;
            app.view = View::Dashboard;
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => {
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
    let config = Config::load();
    if config.tokens.is_empty() {
        app.enter_token_input();
        return;
    }
    let binary_path = match platform::find_cokacdir() {
        Some(p) => p,
        None => {
            app.set_status("cokacdir not found. Press [I] to install", true);
            return;
        }
    };
    let mgr = service::manager();
    match mgr.start(&binary_path, &config.tokens) {
        Ok(()) => {
            app.set_status("Service started", false);
            app.refresh_status();
        }
        Err(e) => app.set_status(&format!("Start failed: {}", e), true),
    }
}



fn action_stop(app: &mut App) {
    let mgr = service::manager();
    match mgr.stop() {
        Ok(()) => {
            app.set_status("Service stopped", false);
            app.refresh_status();
        }
        Err(e) => app.set_status(&format!("Stop failed: {}", e), true),
    }
}

fn action_restart(app: &mut App) {
    let config = Config::load();
    if config.tokens.is_empty() {
        app.set_status("No tokens configured", true);
        return;
    }
    let binary_path = match platform::find_cokacdir() {
        Some(p) => p,
        None => {
            app.set_status("cokacdir not found", true);
            return;
        }
    };
    let mgr = service::manager();
    match mgr.restart(&binary_path, &config.tokens) {
        Ok(()) => {
            app.set_status("Service restarted", false);
            app.refresh_status();
        }
        Err(e) => app.set_status(&format!("Restart failed: {}", e), true),
    }
}

fn action_remove(app: &mut App) {
    let mgr = service::manager();
    match mgr.remove() {
        Ok(()) => {
            app.set_status("Service removed", false);
            app.refresh_status();
        }
        Err(e) => app.set_status(&format!("Remove failed: {}", e), true),
    }
}
