//! Minimal async HTTP/1.1 server for the dashboard.
//!
//! Hand-rolled so we can avoid pulling in axum/hyper as a direct dependency.
//! Two modes:
//!  * **Loopback** (`--dashboard`) — bound to 127.0.0.1, plain HTTP. Host
//!    allowlist blocks DNS-rebinding; a per-session bearer secret adds a
//!    defense-in-depth layer against co-resident local processes that might
//!    know the port but not the token.
//!  * **Inbound**  (`--inbound`)   — bound to all interfaces, **HTTPS only**
//!    via a per-host self-signed cert under `~/.cokacdir/dashboard/`, plus a
//!    256-bit per-session bearer secret required on every `/api/*` call.
//!
//! Connection handling is generic over `AsyncRead + AsyncWrite` so both a
//! plain `TcpStream` and a `tokio_rustls::server::TlsStream<TcpStream>` flow
//! through the same parser/router/writer.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

use super::api;
use super::assets;
use super::state::{generate_secret, SharedState};
use super::tls;

/// Hard cap on time spent reading a single request. Prevents Slowloris-style
/// attacks where a peer dribbles bytes to keep tokio tasks alive forever.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(15);
/// Hard cap on the TLS handshake itself. A peer that opens a socket and
/// never speaks would otherwise pin a task for the full read timeout's worth
/// of nothing.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest tolerated request header section.
const MAX_HEADER_BYTES: usize = 65_536;
/// Largest tolerated request body.
const MAX_BODY_BYTES: usize = 1_048_576;

pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
    pub host: Option<String>,
    pub origin: Option<String>,
    pub bearer: Option<String>,
    pub peer: SocketAddr,
}

pub struct Response {
    pub status: u16,
    pub status_text: &'static str,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok_json(body: String) -> Self {
        Response {
            status: 200,
            status_text: "OK",
            content_type: "application/json; charset=utf-8".into(),
            body: body.into_bytes(),
        }
    }
    pub fn err_json(status: u16, text: &'static str, message: String) -> Self {
        let body = serde_json::json!({ "error": message }).to_string();
        Response {
            status,
            status_text: text,
            content_type: "application/json; charset=utf-8".into(),
            body: body.into_bytes(),
        }
    }
    pub fn static_str(content_type: &str, body: &'static str) -> Self {
        Response {
            status: 200,
            status_text: "OK",
            content_type: content_type.into(),
            body: body.as_bytes().to_vec(),
        }
    }
    pub fn not_found() -> Self {
        Response {
            status: 404,
            status_text: "Not Found",
            content_type: "text/plain; charset=utf-8".into(),
            body: b"Not Found".to_vec(),
        }
    }
    pub fn method_not_allowed() -> Self {
        Response {
            status: 405,
            status_text: "Method Not Allowed",
            content_type: "text/plain; charset=utf-8".into(),
            body: b"Method Not Allowed".to_vec(),
        }
    }
    pub fn unauthorized() -> Self {
        Self::err_json(401, "Unauthorized", "Missing or invalid auth token".into())
    }
}

pub async fn serve(port: u16, inbound: bool) -> Result<(), String> {
    let bind_ip = if inbound {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    let (listener, port) = bind_with_fallback(bind_ip, port).await?;

    // Mint per-session bearer secret for both modes. In inbound mode it's the
    // primary auth boundary; in loopback it's a second layer on top of the
    // Host allowlist so a co-resident local process can't hit /api/* just by
    // guessing the port. TLS material is inbound-only — loopback stays plain
    // HTTP since the traffic never leaves the kernel's loopback interface.
    let auth_token = Some(generate_secret()?);
    let tls_material = if inbound {
        Some(tls::load_or_create()?)
    } else {
        None
    };
    let state = SharedState::new(auth_token.clone(), inbound, port);
    let acceptor = tls_material.as_ref().map(|m| TlsAcceptor::from(m.server_config.clone()));

    print_banner(port, inbound, auth_token.as_deref(), tls_material.as_ref());

    if !inbound {
        let secret = auth_token.as_deref().unwrap_or("");
        try_open_browser(&format!("http://127.0.0.1:{}/#access={}", port, secret));
    }

    dlog!(
        "dashboard",
        "Listening on {}:{} (inbound={}, tls={}, auth={})",
        bind_ip,
        port,
        inbound,
        acceptor.is_some(),
        auth_token.is_some()
    );

    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                dlog!("dashboard", "accept failed: {}", e);
                continue;
            }
        };
        // Defense in depth: when the user asked for loopback only, refuse
        // any non-loopback peer even if a misconfiguration somehow routes in.
        if !inbound && !peer.ip().is_loopback() {
            dlog!("dashboard", "Rejecting non-loopback peer {}", peer);
            drop(socket);
            continue;
        }
        let st = state.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let res = match acceptor {
                Some(acc) => serve_tls(acc, socket, peer, st).await,
                None      => serve_plain(socket, peer, st).await,
            };
            if let Err(e) = res {
                dlog!("dashboard", "conn error from {}: {}", peer, e);
            }
        });
    }
}

async fn serve_plain(
    socket: TcpStream,
    peer: SocketAddr,
    state: SharedState,
) -> Result<(), String> {
    handle_connection(socket, peer, state).await
}

async fn serve_tls(
    acceptor: TlsAcceptor,
    socket: TcpStream,
    peer: SocketAddr,
    state: SharedState,
) -> Result<(), String> {
    let tls_stream = match timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(socket)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            dlog!("dashboard", "TLS handshake error from {}: {}", peer, e);
            return Ok(());
        }
        Err(_) => {
            dlog!("dashboard", "TLS handshake timeout from {}", peer);
            return Ok(());
        }
    };
    handle_connection(tls_stream, peer, state).await
}

async fn handle_connection<S>(
    mut socket: S,
    peer: SocketAddr,
    state: SharedState,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let req = match timeout(REQUEST_READ_TIMEOUT, read_request(&mut socket, peer)).await {
        Ok(Ok(req)) => req,
        Ok(Err(e)) => {
            dlog!("dashboard", "read_request error from {}: {}", peer, e);
            return Ok(());
        }
        Err(_) => {
            dlog!("dashboard", "read timeout from {}", peer);
            let _ = write_response(&mut socket, &Response {
                status: 408, status_text: "Request Timeout",
                content_type: "text/plain; charset=utf-8".into(),
                body: b"Request Timeout".to_vec(),
            }).await;
            return Ok(());
        }
    };

    dlog!("dashboard", "{} {} from {}", req.method, req.path, peer);
    let resp = route(&req, &state).await;
    write_response(&mut socket, &resp).await?;
    Ok(())
}

async fn read_request<S>(socket: &mut S, peer: SocketAddr) -> Result<Request, String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end;
    loop {
        let n = socket.read(&mut tmp).await
            .map_err(|e| format!("read: {}", e))?;
        if n == 0 {
            return Err("client closed before headers".into());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_header_end(&buf) {
            header_end = pos;
            break;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err("headers too large".into());
        }
    }

    let head = std::str::from_utf8(&buf[..header_end])
        .map_err(|e| format!("non-utf8 headers: {}", e))?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length: usize = 0;
    let mut host: Option<String> = None;
    let mut origin: Option<String> = None;
    let mut bearer: Option<String> = None;
    for line in lines {
        if line.is_empty() { break; }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("content-length") {
                content_length = val.parse().unwrap_or(0);
                if content_length > MAX_BODY_BYTES {
                    return Err("body too large".into());
                }
            } else if key.eq_ignore_ascii_case("host") {
                host = Some(val.to_string());
            } else if key.eq_ignore_ascii_case("origin") {
                origin = Some(val.to_string());
            } else if key.eq_ignore_ascii_case("authorization") {
                if let Some(rest) = val.strip_prefix("Bearer ") {
                    bearer = Some(rest.trim().to_string());
                } else if let Some(rest) = val.strip_prefix("bearer ") {
                    bearer = Some(rest.trim().to_string());
                }
            }
        }
    }

    let already_have = buf.len().saturating_sub(header_end + 4);
    let mut body = Vec::with_capacity(content_length);
    if already_have > 0 {
        body.extend_from_slice(&buf[header_end + 4..header_end + 4 + already_have.min(content_length)]);
    }
    while body.len() < content_length {
        let n = socket.read(&mut tmp).await
            .map_err(|e| format!("read body: {}", e))?;
        if n == 0 { break; }
        let need = content_length - body.len();
        body.extend_from_slice(&tmp[..n.min(need)]);
    }

    Ok(Request { method, path, body, host, origin, bearer, peer })
}

async fn route(req: &Request, state: &SharedState) -> Response {
    // DNS-rebinding defense in loopback mode: reject any Host that doesn't
    // name a loopback authority, so an attacker-controlled hostname resolving
    // to 127.0.0.1 can't slip past the Origin-vs-Host cross-compare (both
    // headers would carry the attacker's name and match each other).
    // In inbound mode `host_allowed` returns true unconditionally — the
    // client's authority legitimately varies with port forwarding / reverse
    // proxies, and bearer-token auth on `/api/*` is what actually blocks
    // rebinding there. See `SharedState::host_allowed` for details.
    if !state.host_allowed(req.host.as_deref()) {
        dlog!(
            "dashboard",
            "rejecting Host={:?} (not in allowlist) from {}",
            req.host, req.peer
        );
        return Response::err_json(
            421, "Misdirected Request",
            "Host header does not name this dashboard".into(),
        );
    }

    if req.method == "GET" {
        if let Some((ct, body)) = assets::lookup(&req.path) {
            return Response::static_str(ct, body);
        }
    }

    if req.path.starts_with("/api/") {
        if req.method != "GET" {
            if let Some(ref origin) = req.origin {
                if !origin_matches_host(origin, req.host.as_deref()) {
                    return Response::err_json(
                        403, "Forbidden",
                        "Cross-origin request blocked".into(),
                    );
                }
            }
        }
        if !state.check_auth(req.bearer.as_deref()) {
            dlog!("dashboard", "auth failed: {} {} from {}",
                  req.method, req.path, req.peer);
            return Response::unauthorized();
        }
        return api::handle(req, state).await;
    }

    if req.method != "GET" {
        return Response::method_not_allowed();
    }
    Response::not_found()
}

async fn write_response<S>(socket: &mut S, resp: &Response) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let head = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         Referrer-Policy: no-referrer\r\n\
         Connection: close\r\n\
         \r\n",
        resp.status, resp.status_text, resp.content_type, resp.body.len()
    );
    socket.write_all(head.as_bytes()).await.map_err(|e| format!("write head: {}", e))?;
    socket.write_all(&resp.body).await.map_err(|e| format!("write body: {}", e))?;
    socket.shutdown().await.ok();
    Ok(())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Number of sequential ports to try after the requested one. 20 covers the
/// common "I have a couple of dashboards already up" case without scanning an
/// arbitrary stretch of the address space.
const BIND_FALLBACK_ATTEMPTS: u16 = 20;

/// Binds the requested port; on "address already in use" errors, walks
/// forward through adjacent ports up to `BIND_FALLBACK_ATTEMPTS`. Only
/// AddrInUse is considered retryable — other errors (EACCES on privileged
/// ports, interface unreachable, etc.) won't be cured by picking a different
/// port and surface immediately.
async fn bind_with_fallback(
    bind_ip: IpAddr,
    requested_port: u16,
) -> Result<(TcpListener, u16), String> {
    let mut last_err: Option<String> = None;
    let mut attempted: u16 = 0;
    let mut last_candidate = requested_port;
    for offset in 0..BIND_FALLBACK_ATTEMPTS {
        // `requested_port + offset` must stay within u16. When the user picks
        // a high starting port we may exhaust the range before BIND_FALLBACK
        // _ATTEMPTS iterations — stop cleanly in that case.
        let candidate = match requested_port.checked_add(offset) {
            Some(p) => p,
            None => break,
        };
        attempted += 1;
        last_candidate = candidate;
        let addr = SocketAddr::new(bind_ip, candidate);
        match TcpListener::bind(addr).await {
            Ok(listener) => {
                // `--port 0` is the "let the OS pick" convention. The bound
                // socket's actual port lives in `local_addr`; `candidate` (0)
                // would produce a broken URL in the banner. Resolving the
                // real port here also guards against any future change that
                // might rebind mid-call.
                let actual = listener
                    .local_addr()
                    .map(|a| a.port())
                    .unwrap_or(candidate);
                if requested_port == 0 {
                    dlog!("dashboard", "OS-assigned port {}", actual);
                } else if actual != requested_port {
                    eprintln!(
                        "\x1b[33m  Port {} is in use; using port {} instead.\x1b[0m",
                        requested_port, actual
                    );
                    dlog!(
                        "dashboard",
                        "Requested port {} unavailable; bound {} after {} attempt(s)",
                        requested_port, actual, attempted
                    );
                }
                return Ok((listener, actual));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                dlog!("dashboard", "Port {} in use, trying next", candidate);
                last_err = Some(format!("{}: {}", addr, e));
                continue;
            }
            Err(e) => {
                return Err(format!("Failed to bind {}: {}", addr, e));
            }
        }
    }
    Err(format!(
        "No free port found in range {}..={} after {} attempt(s). \
         Last error: {}. \
         Stop the process holding the port (e.g. `lsof -i :{}` on Unix, \
         `netstat -ano | findstr :{}` on Windows) or rerun with a different \
         starting port via `--port <PORT>`.",
        requested_port,
        last_candidate,
        attempted,
        last_err.unwrap_or_else(|| "none".into()),
        requested_port,
        requested_port,
    ))
}

/// Returns true when the Origin header refers to the same host:port we are
/// being addressed at. Strips the scheme so "http://127.0.0.1:38573" or
/// "https://192.168.1.5:38573" both compare against Host "<addr>:<port>".
fn origin_matches_host(origin: &str, host: Option<&str>) -> bool {
    let host = match host {
        Some(h) => h,
        None => return false,
    };
    let stripped = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or(origin)
        .trim_end_matches('/');
    stripped.eq_ignore_ascii_case(host)
}

fn print_banner(port: u16, inbound: bool, auth: Option<&str>, tls: Option<&tls::TlsMaterial>) {
    println!();
    println!("  cokacctl dashboard");
    println!("  ────────────────────────────────────────");
    if inbound {
        let secret = auth.unwrap_or("");
        // Always advertise `localhost` as the primary URL. Interface IPs that
        // getifaddrs returns are often non-reachable for the user's actual
        // remote-access case (Docker bridges, VM host-only networks, VPN
        // tunnels, link-local) so suggesting them causes more failed clicks
        // than successful ones. A user who genuinely wants remote access
        // knows their own network and can substitute the hostname themselves
        // — the cert's SAN already covers every local interface IP so any
        // substitution that works network-wise will also pass TLS.
        println!("  Open: https://localhost:{}/#access={}", port, secret);
        if let Some(material) = tls {
            println!();
            println!("  Cert fingerprint (SHA-256):");
            for line in fingerprint_lines(&material.fingerprint_sha256) {
                println!("    {}", line);
            }
            println!("  Cert path: {}", material.cert_path.display());
        }
        println!();
        println!("  \x1b[36mFirst-visit browser warning\x1b[0m");
        println!("  ────────────────────────────────────────");
        println!("  On first visit you will see a \"Your connection is not private\"");
        println!("  warning in the browser.");
        println!();
        println!("    1) Click \x1b[1m[Advanced]\x1b[0m");
        println!("    2) Click \x1b[1m[Proceed to ... (unsafe)]\x1b[0m");
        println!();
        println!("  \x1b[32mThis is expected.\x1b[0m The warning appears because the certificate is");
        println!("  not signed by a public CA (Let's Encrypt etc.); cokacctl issued a");
        println!("  self-signed cert entirely inside this machine. The certificate and");
        println!("  its private key were generated here and never leave this machine.");
        println!("  Compare the fingerprint shown above with the one in the browser's");
        println!("  certificate-details dialog. If they match, traffic from that point");
        println!("  on is end-to-end TLS-encrypted and safe from eavesdropping/tampering.");
        println!();
        println!("  \x1b[33m! Inbound mode: bound to 0.0.0.0 — reachable from other hosts.\x1b[0m");
        println!("  \x1b[33m  Treat the URL like a password — anyone with it gets full control.\x1b[0m");
    } else {
        let secret = auth.unwrap_or("");
        println!("  Open: http://127.0.0.1:{}/#access={}", port, secret);
        println!("  Bound to loopback only — not reachable from other hosts.");
        println!("  Access token in the URL is required; other local processes");
        println!("  that don't have it cannot reach /api/*.");
    }
    println!("  Press Ctrl+C to stop.");
    println!();
}

/// Splits a "AA:BB:CC:..." fingerprint into ~16-byte lines for readability.
fn fingerprint_lines(fp: &str) -> Vec<String> {
    // 32 bytes -> 32 hex pairs separated by ':' = 95 chars; print as two
    // 16-byte rows so the user can eyeball it against a browser dialog.
    let bytes: Vec<&str> = fp.split(':').collect();
    bytes
        .chunks(16)
        .map(|c| c.join(":"))
        .collect()
}

fn try_open_browser(url: &str) {
    // Spawn fire-and-forget; failure is harmless — the URL is printed anyway.
    #[cfg(target_os = "macos")]
    let spawn = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let spawn = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let spawn = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let spawn: std::io::Result<std::process::Child> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no known browser launcher for this OS",
    ));
    match spawn {
        Ok(_) => dlog!("dashboard", "Browser open attempted"),
        Err(e) => dlog!("dashboard", "Browser open failed: {} (ok)", e),
    }
}

