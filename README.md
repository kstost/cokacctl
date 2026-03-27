# COKACCTL

**Installation & Service Manager for COKACDIR.**

Install, update, and manage cokacdir as a background service — all from a single command or interactive TUI. Supports systemd (Linux), launchd (macOS), and Task Scheduler (Windows).

cokacdir runs as a Telegram bot. You register your bot token(s) with cokacctl, and it handles running cokacdir in the background as a system service that persists across reboots.

## Quick Start

Copy and paste one line — downloads cokacctl, installs it, and launches automatically:

**macOS / Linux:**

```
curl -fsSL https://raw.githubusercontent.com/kstost/cokacctl/refs/heads/main/manage.sh | bash && cokacctl
```

**Windows (PowerShell):**

```
irm https://raw.githubusercontent.com/kstost/cokacctl/refs/heads/main/manage.ps1 | iex; cokacctl
```

After the installer runs, the interactive TUI launches. From there you can install cokacdir, register tokens, and start the service — all with keyboard shortcuts.

## Usage

### Interactive TUI

Run without arguments to launch the interactive TUI dashboard:

```bash
cokacctl
```

The TUI provides a full dashboard with:
- Version info and update availability
- Service status monitoring (Running / Stopped / Not installed)
- Log viewer with real-time streaming
- Keyboard shortcuts for all operations

**TUI Keyboard Shortcuts:**

| Key | Action |
|-----|--------|
| `I` | Install cokacdir |
| `U` | Update cokacdir |
| `S` | Start service |
| `T` | Stop service |
| `R` | Restart service |
| `D` | Remove service |
| `K` | Manage bot tokens |
| `L` | Full-screen log viewer |
| `Q` | Quit |

### CLI Commands

All TUI features are also available as CLI commands:

**Setup:**

```bash
cokacctl install                  # Download and install cokacdir binary
cokacctl update                   # Update cokacdir to latest version
```

**Token Management:**

```bash
cokacctl token <TOKEN>            # Register a Telegram bot token
cokacctl token <TOKEN1> <TOKEN2>  # Register multiple bot tokens (space-separated)
```

Tokens are saved to `~/.cokacdir/config.json` and reused across start/restart. Registering new tokens overwrites previously saved ones. Get tokens from [@BotFather](https://t.me/BotFather) on Telegram (`/newbot`).

**Service Management:**

```bash
cokacctl start                    # Start the background service
cokacctl stop                     # Stop the service
cokacctl restart                  # Restart with currently registered tokens
cokacctl remove                   # Remove the service registration entirely
```

- `start` requires tokens to be registered first via `cokacctl token`.
- `start` registers the service for auto-start on login/boot.
- `restart` reuses the currently registered tokens.
- `remove` stops the service and removes its registration. `start` will re-register from scratch.

**Monitoring:**

```bash
cokacctl status                   # Show versions, service state, and token count
cokacctl log                      # Tail service log in real time (Ctrl+C to stop)
```

**Typical workflow:**

```bash
cokacctl install                  # 1. Install cokacdir
cokacctl token 123456:ABC-xyz     # 2. Register your Telegram bot token
cokacctl start                    # 3. Start the service
cokacctl status                   # 4. Verify it's running
cokacctl log                      # 5. Watch the log
```

### How `update` Works

`cokacctl update` checks for a newer version and updates the cokacdir binary in-place. If the service is currently running, it automatically stops the service before updating and restarts it afterward with the same tokens.

## Supported Platforms

| Platform | Architecture | Service Backend |
|----------|-------------|-----------------|
| macOS | Apple Silicon (aarch64) | launchd |
| macOS | Intel (x86_64) | launchd |
| Linux | x86_64 | systemd |
| Linux | ARM64 (aarch64) | systemd |
| Windows | x86_64 | Task Scheduler |
| Windows | ARM64 (aarch64) | Task Scheduler |

## File Locations

| File | Path |
|------|------|
| Config | `~/.cokacdir/config.json` |
| Service log | `~/.cokacdir/logs/cokacdir.log` (Windows) |
| | `~/Library/Logs/cokacdir/cokacdir.log` (macOS) |
| | `~/.local/state/cokacdir/cokacdir.log` (Linux) |
| Error log | Same directory as service log, `cokacdir.error.log` |
| Debug log | `~/.cokacdir/debug/cokacctl.log` |

## Community

[Telegram Group](https://t.me/cokacvibe)

## License

MIT License

## Author

cokac <monogatree@gmail.com>

## Disclaimer

THIS SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.

IN NO EVENT SHALL THE AUTHORS, COPYRIGHT HOLDERS, OR CONTRIBUTORS BE LIABLE FOR ANY CLAIM, DAMAGES, OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

This includes, without limitation:

- Data loss or corruption
- System damage or malfunction
- Security breaches or vulnerabilities
- Financial losses
- Any direct, indirect, incidental, special, exemplary, or consequential damages

The user assumes full responsibility for all consequences arising from the use of this software, regardless of whether such use was intended, authorized, or anticipated.

**USE AT YOUR OWN RISK.**
