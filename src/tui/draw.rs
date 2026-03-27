use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{block::Title, Block, BorderType, Borders, Padding, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, View};
use crate::service::ServiceStatus;

// Color palette
const ACCENT: Color = Color::Indexed(75);   // Soft blue
const CYAN: Color = Color::Indexed(87);     // Bright cyan
const GREEN: Color = Color::Indexed(78);    // Soft green
const RED: Color = Color::Indexed(167);     // Soft red
const YELLOW: Color = Color::Indexed(220);  // Warning yellow
const DIM: Color = Color::Indexed(242);     // Muted text
const SUBTLE: Color = Color::Indexed(236);  // Very subtle border
const TEXT: Color = Color::Indexed(252);    // Primary text
const LABEL: Color = Color::Indexed(246);   // Label text

pub fn draw(f: &mut Frame, app: &App) {
    match app.view {
        View::Welcome => draw_welcome(f, app),
        View::TokenInput => draw_token_input(f, app),
        View::Progress => draw_progress(f, app),
        View::Dashboard => draw_dashboard(f, app),
        View::LogFullscreen => draw_log_fullscreen(f, app),
    }
}

// ── Dashboard ──────────────────────────────────────────────────

fn draw_dashboard(f: &mut Frame, app: &App) {
    let size = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // Version info
            Constraint::Length(5),  // Service status
            Constraint::Min(5),    // Logs
            Constraint::Length(1), // Status bar
        ])
        .split(size);

    draw_version_panel(f, app, chunks[0]);
    draw_service_panel(f, app, chunks[1]);
    draw_log_panel(f, app, chunks[2]);
    draw_status_bar(f, app, chunks[3]);
}

fn draw_version_panel(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE))
        .title(Span::styled(" cokacctl ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = Vec::new();

    // cokacdir version
    let version_str = app.cokacdir_version.as_deref().unwrap_or("not installed");
    let (ver_style, status_icon) = if app.cokacdir_version.is_some() {
        (Style::default().fg(GREEN), "  ")
    } else {
        (Style::default().fg(RED), "  ")
    };
    let path_str = app.cokacdir_path.as_deref().unwrap_or("");

    lines.push(Line::from(vec![
        Span::styled("cokacdir ", Style::default().fg(LABEL)),
        Span::styled(status_icon, ver_style),
        Span::styled(if app.cokacdir_version.is_some() { format!("v{}", version_str) } else { version_str.to_string() }, ver_style),
        Span::styled(format!("  {}", path_str), Style::default().fg(DIM)),
    ]));

    // cokacctl version
    lines.push(Line::from(vec![
        Span::styled("cokacctl ", Style::default().fg(LABEL)),
        Span::styled(format!("  v{}", env!("CARGO_PKG_VERSION")), Style::default().fg(DIM)),
    ]));

    // Update status
    if app.checking_update {
        lines.push(Line::from(vec![
            Span::styled("update   ", Style::default().fg(LABEL)),
            Span::styled("  checking...", Style::default().fg(DIM)),
        ]));
    } else if app.update_available() {
        let latest = app.latest_version.as_deref().unwrap_or("?");
        lines.push(Line::from(vec![
            Span::styled("update   ", Style::default().fg(LABEL)),
            Span::styled(format!("  v{} available ", latest), Style::default().fg(YELLOW)),
            Span::styled(" U ", Style::default().fg(Color::Black).bg(CYAN).add_modifier(Modifier::BOLD)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("update   ", Style::default().fg(LABEL)),
            Span::styled("  up to date", Style::default().fg(GREEN)),
        ]));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

fn draw_service_panel(f: &mut Frame, app: &App, area: Rect) {
    let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    let (status_color, status_text) = if app.service_busy {
        let frame = spinner_frames[app.service_busy_tick % spinner_frames.len()];
        (YELLOW, format!("{} {}...", frame, app.service_busy_label))
    } else {
        match &app.service_status {
            ServiceStatus::Running => (GREEN, "● Running".to_string()),
            ServiceStatus::Stopped => (RED, "○ Stopped".to_string()),
            ServiceStatus::NotInstalled => (DIM, "○ Not installed".to_string()),
            ServiceStatus::Unknown(_) => (YELLOW, "? Unknown".to_string()),
        }
    };

    let os_name = match crate::core::platform::Os::detect() {
        crate::core::platform::Os::MacOS => "launchd",
        crate::core::platform::Os::Linux => "systemd",
        crate::core::platform::Os::Windows => "Task Scheduler",
    };

    // Key hints - dimmed when busy
    let hint_style = if app.service_busy {
        Style::default().fg(Color::Indexed(239))
    } else {
        Style::default().fg(DIM)
    };
    let key_style = if app.service_busy {
        Style::default().fg(Color::Indexed(239)).bg(Color::Indexed(236))
    } else {
        Style::default().fg(Color::Black).bg(ACCENT)
    };

    let hints = Line::from(vec![
        Span::styled(" S ", key_style),
        Span::styled("tart ", hint_style),
        Span::styled(" T ", key_style),
        Span::styled("stop ", hint_style),
        Span::styled(" R ", key_style),
        Span::styled("estart ", hint_style),
        Span::styled(" D ", key_style),
        Span::styled("elete ", hint_style),
        Span::styled(" K ", key_style),
        Span::styled("eys ", hint_style),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE))
        .title(Span::styled(" Service ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
        .title(Title::from(hints).alignment(Alignment::Center).position(ratatui::widgets::block::Position::Bottom))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("status ", Style::default().fg(LABEL)),
        Span::styled(&status_text, Style::default().fg(status_color)),
        Span::styled(format!("  via {}", os_name), Style::default().fg(DIM)),
    ]));

    let token_str = if app.token_count() > 0 {
        format!("{} bot(s)", app.token_count())
    } else {
        "none".to_string()
    };
    lines.push(Line::from(vec![
        Span::styled("tokens ", Style::default().fg(LABEL)),
        Span::styled(
            token_str,
            Style::default().fg(if app.token_count() > 0 { GREEN } else { DIM }),
        ),
    ]));

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

fn draw_log_panel(f: &mut Frame, app: &App, area: Rect) {
    let log_title = crate::service::manager()
        .log_path()
        .map(|p| format!(" {} ", p.display()))
        .unwrap_or_else(|| " Logs ".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE))
        .title(Span::styled(log_title, Style::default().fg(DIM)));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.log_lines.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            " No log entries",
            Style::default().fg(DIM),
        )));
        f.render_widget(msg, inner);
    } else {
        let visible = inner.height as usize;
        let total = app.log_lines.len();
        let start = if total > visible { total - visible } else { 0 };
        let lines: Vec<Line> = app.log_lines[start..]
            .iter()
            .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(DIM))))
            .collect();
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(paragraph, inner);
    }
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let line = if let Some(msg) = &app.status_message {
        let color = if msg.is_error { RED } else { GREEN };
        Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(&msg.text, Style::default().fg(color)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(" Q ", Style::default().fg(Color::Black).bg(ACCENT)),
            Span::styled("uit  ", Style::default().fg(DIM)),
            Span::styled(" L ", Style::default().fg(Color::Black).bg(ACCENT)),
            Span::styled("og  ", Style::default().fg(DIM)),
            Span::styled(" I ", Style::default().fg(Color::Black).bg(ACCENT)),
            Span::styled("nstall  ", Style::default().fg(DIM)),
            Span::styled(" U ", Style::default().fg(Color::Black).bg(ACCENT)),
            Span::styled("pdate", Style::default().fg(DIM)),
        ])
    };
    let bar = Paragraph::new(line);
    f.render_widget(bar, area);
}

// ── Progress ───────────────────────────────────────────────────

fn draw_progress(f: &mut Frame, app: &App) {
    use super::app::ProgressAction;

    let area = f.area();
    let title = match &app.progress_action {
        Some(ProgressAction::Install) => " Installing ",
        Some(ProgressAction::Update) => " Updating ",
        None => " Progress ",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE))
        .title(Span::styled(title, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
        .padding(Padding::new(1, 1, 1, 0));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Progress lines
    let visible = (inner.height as usize).saturating_sub(4);
    let total = app.progress_lines.len();
    let start = if total > visible { total - visible } else { 0 };
    for line in &app.progress_lines[start..] {
        lines.push(Line::from(Span::styled(line.clone(), Style::default().fg(TEXT))));
    }

    // Completion status
    if let Some(result) = &app.progress_done {
        lines.push(Line::from(""));
        match result {
            Ok(()) => {
                lines.push(Line::from(vec![
                    Span::styled(" ", Style::default().fg(GREEN)),
                    Span::styled(" Done", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
                ]));
            }
            Err(e) => {
                lines.push(Line::from(vec![
                    Span::styled(" ", Style::default().fg(RED)),
                    Span::styled(format!(" {}", e), Style::default().fg(RED)),
                ]));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Press any key to continue...",
            Style::default().fg(DIM),
        )));
    } else {
        lines.push(Line::from(""));
        // Animated dots based on progress line count
        let dots = ".".repeat((app.progress_lines.len() % 3) + 1);
        lines.push(Line::from(Span::styled(
            format!("Working{:<3}", dots),
            Style::default().fg(YELLOW),
        )));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

// ── Welcome ────────────────────────────────────────────────────

fn draw_welcome(f: &mut Frame, _app: &App) {
    let area = f.area();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE))
        .title(
            Title::from(Span::styled(
                " cokacctl ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
        );
    let inner = block.inner(area);
    f.render_widget(block, area);

    let content_height: u16 = 7;
    let pad_top = if inner.height > content_height {
        (inner.height - content_height) / 2
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::new();
    for _ in 0..pad_top {
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        format!("v{}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(DIM),
    )).alignment(Alignment::Center));
    lines.push(Line::from(""));
    lines.push(
        Line::from(Span::styled(
            "cokacdir is not installed.",
            Style::default().fg(YELLOW),
        ))
        .alignment(Alignment::Center),
    );
    lines.push(Line::from(""));
    lines.push(
        Line::from(vec![
            Span::styled("Press ", Style::default().fg(TEXT)),
            Span::styled(" I ", Style::default().fg(Color::Black).bg(CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(" to install", Style::default().fg(TEXT)),
        ])
        .alignment(Alignment::Center),
    );
    lines.push(Line::from(""));
    lines.push(
        Line::from(vec![
            Span::styled(" Q ", Style::default().fg(DIM).bg(SUBTLE)),
            Span::styled(" Quit", Style::default().fg(DIM)),
        ])
        .alignment(Alignment::Center),
    );

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

// ── Token Input ────────────────────────────────────────────────

fn draw_token_input(f: &mut Frame, app: &App) {
    let size = f.area();
    let input_focused = app.token_cursor.is_none();

    // Clear entire area first to prevent ghosting
    f.render_widget(Block::default(), size);

    // Fixed layout — token list takes all remaining space
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),                  // guide panel
            Constraint::Min(5),                     // token list panel (flexible)
            Constraint::Length(3),                  // input panel
            Constraint::Length(1),                  // status bar
        ])
        .split(size);

    // ── Guide panel ──
    let guide_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE))
        .title(Span::styled(
            format!(" Tokens ({}) ", app.token_list.len()),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1));
    let guide_inner = guide_block.inner(chunks[0]);
    f.render_widget(guide_block, chunks[0]);
    let guide = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Telegram Bot Token", Style::default().fg(TEXT).add_modifier(Modifier::BOLD)),
            Span::styled(" - cokacdir runs as a Telegram bot.", Style::default().fg(DIM)),
        ]),
        Line::from(vec![
            Span::styled("Get a token from ", Style::default().fg(DIM)),
            Span::styled("@BotFather", Style::default().fg(CYAN)),
            Span::styled(" on Telegram (/newbot).", Style::default().fg(DIM)),
        ]),
    ]);
    f.render_widget(guide, guide_inner);

    // ── Token list panel ──
    let token_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE))
        .padding(Padding::horizontal(1));
    let token_inner = token_block.inner(chunks[1]);
    f.render_widget(token_block, chunks[1]);

    let mut token_lines: Vec<Line> = Vec::new();
    if app.token_list.is_empty() {
        token_lines.push(Line::from(Span::styled(
            "No tokens registered.",
            Style::default().fg(DIM),
        )));
    } else {
        for (i, token) in app.token_list.iter().enumerate() {
            let is_selected = app.token_cursor == Some(i);
            let display = mask_token(token);
            if is_selected {
                token_lines.push(Line::from(vec![
                    Span::styled(" > ", Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{}. ", i + 1), Style::default().fg(CYAN)),
                    Span::styled(display.clone(), Style::default().fg(TEXT)),
                    Span::styled("  ", Style::default()),
                    Span::styled(" Del ", Style::default().fg(Color::White).bg(RED)),
                ]));
            } else {
                token_lines.push(Line::from(vec![
                    Span::styled("   ", Style::default()),
                    Span::styled(format!("{}. ", i + 1), Style::default().fg(DIM)),
                    Span::styled(display.clone(), Style::default().fg(DIM)),
                ]));
            }
        }
    }
    f.render_widget(Paragraph::new(token_lines), token_inner);

    // ── Input panel ──
    let mut hint_spans = vec![];
    if input_focused {
        hint_spans.push(Span::styled(" Enter ", Style::default().fg(Color::Black).bg(ACCENT)));
        hint_spans.push(Span::styled(" Add ", Style::default().fg(DIM)));
    }
    hint_spans.push(Span::styled(" Esc ", Style::default().fg(DIM).bg(SUBTLE)));
    hint_spans.push(Span::styled(" Back ", Style::default().fg(DIM)));

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if input_focused { CYAN } else { SUBTLE }))
        .title(Span::styled(" Add Token ", Style::default().fg(ACCENT)))
        .title(Title::from(Line::from(hint_spans)).alignment(Alignment::Right).position(ratatui::widgets::block::Position::Bottom))
        .padding(Padding::horizontal(1));
    let input_inner = input_block.inner(chunks[2]);
    f.render_widget(input_block, chunks[2]);

    let input_text = if input_focused {
        format!("{}_", app.token_input)
    } else {
        app.token_input.clone()
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            input_text,
            Style::default().fg(if input_focused { TEXT } else { DIM }),
        )),
        input_inner,
    );

    // ── Status bar ──
    let line = if let Some(msg) = &app.status_message {
        let color = if msg.is_error { RED } else { GREEN };
        Line::from(Span::styled(&msg.text, Style::default().fg(color)))
    } else {
        Line::from(Span::styled(
            " K: back  ↑↓: navigate tokens",
            Style::default().fg(DIM),
        ))
    };
    f.render_widget(Paragraph::new(line), chunks[3]);
}

fn mask_token(token: &str) -> String {
    let len = token.len();
    if len <= 12 {
        return token.to_string();
    }
    let prefix = &token[..6];
    let suffix = &token[len - 4..];
    let masked = ".".repeat(std::cmp::min(len - 10, 16));
    format!("{}{}{}", prefix, masked, suffix)
}

// ── Log Fullscreen ─────────────────────────────────────────────

fn draw_log_fullscreen(f: &mut Frame, app: &App) {
    let area = f.area();

    let title = if app.log_scroll_offset > 0 {
        format!(" Logs  +{} ", app.log_scroll_offset)
    } else {
        " Logs ".to_string()
    };

    let hints = Line::from(vec![
        Span::styled(" ↑↓ ", Style::default().fg(Color::Black).bg(ACCENT)),
        Span::styled(" Scroll ", Style::default().fg(DIM)),
        Span::styled(" PgUp/Dn ", Style::default().fg(Color::Black).bg(ACCENT)),
        Span::styled(" Page ", Style::default().fg(DIM)),
        Span::styled(" Home/End ", Style::default().fg(Color::Black).bg(ACCENT)),
        Span::styled(" Jump ", Style::default().fg(DIM)),
        Span::styled(" Esc ", Style::default().fg(DIM).bg(SUBTLE)),
        Span::styled(" Back ", Style::default().fg(DIM)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SUBTLE))
        .title(Span::styled(title, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
        .title(Title::from(hints).alignment(Alignment::Center).position(ratatui::widgets::block::Position::Bottom));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.log_lines.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            " No log entries",
            Style::default().fg(DIM),
        )));
        f.render_widget(msg, inner);
    } else {
        let visible = inner.height as usize;
        let total = app.log_lines.len();
        let end = total.saturating_sub(app.log_scroll_offset);
        let start = end.saturating_sub(visible);
        let lines: Vec<Line> = app.log_lines[start..end]
            .iter()
            .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(TEXT))))
            .collect();
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(paragraph, inner);
    }
}
