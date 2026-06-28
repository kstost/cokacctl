#[macro_use]
pub mod debug;
pub mod platform;
pub mod version;
pub mod download;
pub mod config;
pub mod telegram;

/// Progress message for background operations displayed in TUI.
pub enum ProgressMsg {
    Log(String),
    Done(Result<(), String>),
}

pub type ProgressTx = std::sync::mpsc::Sender<ProgressMsg>;
