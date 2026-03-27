pub mod install;
pub mod update;
pub mod service;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cokacctl")]
#[command(version)]
#[command(about = "cokacdir installation and service manager")]
#[command(long_about = "\
cokacctl is the CLI/TUI tool for installing, updating, and managing cokacdir \
as a background service. cokacdir runs as a Telegram bot.

Running without any command launches an interactive TUI dashboard. \
All TUI features are also available as CLI commands listed below.

Supported platforms:
  - macOS (Apple Silicon & Intel) via launchd
  - Linux (x86_64 & ARM64) via systemd
  - Windows (x86_64 & ARM64) via Task Scheduler")]
#[command(after_help = "\
Quick Start:
  cokacctl install                  Download and install cokacdir
  cokacctl token <TOKEN>            Register a Telegram bot token
  cokacctl start                    Start the background service

Service Management:
  cokacctl stop                     Stop the running service
  cokacctl restart                  Restart with current tokens
  cokacctl remove                   Unregister the service entirely
  cokacctl log                      Tail service output in real time

Monitoring:
  cokacctl status                   Show versions, service state, token count
  cokacctl update                   Update cokacdir (auto-restarts if running)

Token Management:
  cokacctl token <T1> <T2> ...      Register one or more bot tokens
                                    (overwrites previously registered tokens)

Interactive Mode:
  cokacctl                          Launch TUI dashboard (no arguments)

Notes:
  - Tokens are persisted in ~/.cokacdir/config.json
  - 'start' requires tokens to be registered beforehand via 'token'
  - 'update' automatically stops and restarts the service if it was running
  - 'restart' reuses the previously registered tokens")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Download and install the cokacdir binary
    #[command(long_about = "\
Download the cokacdir binary for the current platform and install it. \
On Unix, installs to /usr/local/bin (may prompt for sudo). \
On Windows, installs to %LOCALAPPDATA%/cokacctl/.")]
    Install,

    /// Update cokacdir to the latest version
    #[command(long_about = "\
Check for a newer version of cokacdir and update in-place. \
If the service is running, it is automatically stopped before \
the update and restarted afterward with the same tokens.")]
    Update,

    /// Show version, service status, and system info
    #[command(long_about = "\
Display platform info, cokacctl version, cokacdir version and path, \
service status (Running/Stopped/Not installed), registered token count, \
and log file location.")]
    Status,

    /// Start the background service
    #[command(long_about = "\
Start cokacdir as a background service using previously registered tokens. \
Tokens must be registered first via 'cokacctl token <TOKEN>'. \
On macOS uses launchd, on Linux uses systemd, on Windows uses Task Scheduler. \
The service is configured to auto-start on login/boot.")]
    Start,

    /// Stop the background service
    #[command(long_about = "\
Stop the running cokacdir service. The service registration remains \
so it can be started again with 'cokacctl start'.")]
    Stop,

    /// Restart the background service
    #[command(long_about = "\
Stop and start the service using the currently registered tokens. \
Equivalent to 'cokacctl stop' followed by 'cokacctl start'.")]
    Restart,

    /// Remove the service registration entirely
    #[command(long_about = "\
Stop the service and remove its registration (launchd plist, \
systemd unit, or scheduled task). After removal, 'cokacctl start' \
will re-register the service from scratch.")]
    Remove,

    /// Tail the service log in real time
    #[command(long_about = "\
Show the last 20 lines of the cokacdir service log, then follow \
new output in real time (like 'tail -f'). Press Ctrl+C to stop.")]
    Log,

    /// Register Telegram bot token(s)
    #[command(long_about = "\
Save one or more Telegram bot tokens to the config file. \
These tokens are used when starting or restarting the service. \
Overwrites any previously registered tokens. \
Get tokens from @BotFather on Telegram (/newbot).")]
    Token {
        /// Telegram bot tokens (space-separated)
        #[arg(required = true)]
        tokens: Vec<String>,
    },
}
