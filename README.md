# COKACCTL

**Installation & Service Manager for COKACDIR.**

Install, update, and manage cokacdir as a background service — all from a single command or interactive TUI. Supports systemd (Linux), launchd (macOS), and Task Scheduler (Windows).

## Quick Start

Copy and paste one line — downloads, installs, and launches automatically:

**macOS / Linux:**

```
curl -fsSL https://raw.githubusercontent.com/kstost/cokacctl/refs/heads/main/manage.sh | bash && cokacctl
```

**Windows (PowerShell):**

```
irm https://raw.githubusercontent.com/kstost/cokacctl/refs/heads/main/manage.ps1 | iex; cokacctl
```

## Usage

After installation, run directly:

```bash
cokacctl
```

### CLI Commands

```bash
cokacctl install                  # Install cokacdir
cokacctl update                   # Update cokacdir to latest version
cokacctl status                   # Show version, service status, and system info

cokacctl service start <TOKEN>    # Start service with Telegram bot token(s)
cokacctl service stop             # Stop the service
cokacctl service restart          # Restart the service
cokacctl service remove           # Remove the service
cokacctl service status           # Show service status
cokacctl service log              # Tail the service log
cokacctl service token <TOKEN>    # Update bot tokens and restart
```

## Supported Platforms

- macOS (Apple Silicon & Intel)
- Linux (x86_64 & ARM64)
- Windows (x86_64 & ARM64)

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
