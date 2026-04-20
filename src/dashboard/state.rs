//! Shared runtime state for the dashboard — things that aren't already
//! persisted by the core cokacctl modules (activity feed, session start time,
//! cached version check).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ActivityItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub meta: String,
    pub tone: String,
    pub when: String,
}

#[derive(Debug, Default)]
pub struct Inner {
    pub activity: VecDeque<ActivityItem>,
    pub started_at: Option<SystemTime>,
    pub latest_version: Option<String>,
    pub last_check: Option<SystemTime>,
    pub checking_update: bool,
    pub next_id: u64,
}

#[derive(Clone)]
pub struct SharedState {
    inner: Arc<Mutex<Inner>>,
    /// Per-session secret required as `Authorization: Bearer <token>` when
    /// the dashboard is reachable from the network. `None` means auth is
    /// disabled (loopback mode). The initial HTML fetch doesn't go through
    /// this check — it lands on the static asset handler, which is fine
    /// because the HTML has no embedded secrets and the bearer only needs
    /// to reach the browser so the JSX bundle can attach it on `/api/*`.
    auth_token: Arc<Option<String>>,
    /// Tracks whether the current bind is reachable from the network.
    inbound: bool,
    /// Lowercased `host:port` authorities this server will answer to. Every
    /// request's `Host` header is checked against this list to neutralize DNS
    /// rebinding: an attacker can set an external hostname's DNS to resolve
    /// to 127.0.0.1 and trick a victim's browser into issuing requests to our
    /// loopback port, but the browser still sends the attacker's hostname in
    /// the `Host` header. Rejecting unknown Host cuts that path off.
    allowed_hosts: Arc<Vec<String>>,
    /// Serializes config load→mutate→save cycles so concurrent dashboard
    /// requests can't lose each other's edits. `Config::save()` already does
    /// atomic rename, which prevents *file* corruption, but not logical
    /// lost-updates: two handlers racing on the load side will each compute
    /// a mutation off the pre-write snapshot and the second save wipes the
    /// first one's change. Acquired inside spawn_blocking so the critical
    /// section runs on the blocking pool.
    pub config_lock: Arc<Mutex<()>>,
    /// Prevents concurrent install / update runs from clobbering each other
    /// on the cokacdir binary. `try_lock` is used so a second request fails
    /// fast with 409 instead of queueing up behind a multi-minute download.
    pub binary_op_lock: Arc<tokio::sync::Mutex<()>>,
}

impl SharedState {
    pub fn new(
        auth_token: Option<String>,
        inbound: bool,
        port: u16,
        sans: &[String],
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            auth_token: Arc::new(auth_token),
            inbound,
            allowed_hosts: Arc::new(build_allowed_hosts(inbound, port, sans)),
            config_lock: Arc::new(Mutex::new(())),
            binary_op_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn inbound(&self) -> bool {
        self.inbound
    }

    /// Returns true when the presented `Host` header names one of the
    /// authorities this server actually serves. Case-insensitive. Missing
    /// Host is treated as not-allowed (HTTP/1.1 requires it anyway).
    pub fn host_allowed(&self, host: Option<&str>) -> bool {
        let got = match host {
            Some(h) => h.trim().to_ascii_lowercase(),
            None => return false,
        };
        self.allowed_hosts.iter().any(|a| a == &got)
    }

    /// Constant-time comparison of a presented token against the configured
    /// secret. Returns true when no auth is required.
    pub fn check_auth(&self, presented: Option<&str>) -> bool {
        let expected = match self.auth_token.as_ref() {
            Some(t) => t.as_bytes(),
            None => return true, // auth disabled
        };
        let got = match presented {
            Some(p) => p.as_bytes(),
            None => return false,
        };
        if expected.len() != got.len() {
            return false;
        }
        let mut diff: u8 = 0;
        for (a, b) in expected.iter().zip(got.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }

    pub fn push_activity(&self, kind: &str, title: &str, meta: &str, tone: &str) {
        let mut g = self.inner.lock().unwrap();
        g.next_id += 1;
        let id = format!("a-{}", g.next_id);
        let item = ActivityItem {
            id,
            kind: kind.to_string(),
            title: title.to_string(),
            meta: meta.to_string(),
            tone: tone.to_string(),
            when: rfc3339_now(),
        };
        g.activity.push_front(item);
        while g.activity.len() > 200 {
            g.activity.pop_back();
        }
    }

    pub fn activity(&self) -> Vec<ActivityItem> {
        self.inner.lock().unwrap().activity.iter().cloned().collect()
    }

    pub fn mark_started(&self) {
        self.inner.lock().unwrap().started_at = Some(SystemTime::now());
    }

    pub fn mark_stopped(&self) {
        self.inner.lock().unwrap().started_at = None;
    }

    pub fn started_at(&self) -> Option<SystemTime> {
        self.inner.lock().unwrap().started_at
    }

    pub fn latest_version(&self) -> Option<String> {
        self.inner.lock().unwrap().latest_version.clone()
    }

    pub fn last_check(&self) -> Option<SystemTime> {
        self.inner.lock().unwrap().last_check
    }

    pub fn set_latest_version(&self, v: Option<String>) {
        let mut g = self.inner.lock().unwrap();
        g.latest_version = v;
        g.last_check = Some(SystemTime::now());
    }

    pub fn set_checking(&self, v: bool) {
        self.inner.lock().unwrap().checking_update = v;
    }

    pub fn checking(&self) -> bool {
        self.inner.lock().unwrap().checking_update
    }
}

/// Builds the set of `host:port` authorities the dashboard should answer to.
///
/// Loopback mode: only the three loopback names (no network-reachable IPs
/// exist for this bind, and accepting interface IPs would re-open the DNS
/// rebinding hole we're trying to close).
///
/// Inbound mode: the SAN list, which already enumerates localhost + every
/// local interface IP. That's the exact set of authorities the cert is valid
/// for, so any browser that reached us via TLS without a name-mismatch
/// warning is sending one of these in Host.
///
/// IPv6 literals get wrapped in brackets because HTTP Host headers use
/// `[addr]:port` form for IPv6.
fn build_allowed_hosts(inbound: bool, port: u16, sans: &[String]) -> Vec<String> {
    let entries: Vec<String> = if inbound {
        sans.to_vec()
    } else {
        vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()]
    };
    entries
        .into_iter()
        .map(|e| format_host(&e, port).to_ascii_lowercase())
        .collect()
}

fn format_host(entry: &str, port: u16) -> String {
    // IPv6 literal (`::1`, `fe80::...`) — needs brackets in Host header.
    // DNS names and IPv4 literals never contain ':'.
    if entry.contains(':') && !entry.starts_with('[') {
        format!("[{}]:{}", entry, port)
    } else {
        format!("{}:{}", entry, port)
    }
}

pub fn rfc3339_now() -> String {
    use chrono::SecondsFormat;
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn rfc3339_systime(t: SystemTime) -> String {
    use chrono::{DateTime, SecondsFormat, Utc};
    let dt: DateTime<Utc> = t.into();
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Mints a per-session secret using the OS RNG. 32 bytes -> 64 hex chars
/// gives ~256 bits of entropy, which puts brute force out of reach even for
/// an attacker with unlimited inbound connections.
///
/// Returns Err when the OS RNG is unreachable. There is deliberately no
/// weaker fallback: a guessable secret on an inbound-bound socket is worse
/// than refusing to start, because the banner invites the user to treat the
/// URL like a password. The caller is expected to propagate the error so
/// `--inbound` fails loudly instead of silently opening a weak door.
pub fn generate_secret() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| format!(
            "OS RNG unavailable ({}). Refusing to start the inbound dashboard \
             with a degraded auth secret — investigate /dev/urandom or the \
             platform RNG source.",
            e
        ))?;
    Ok(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}
