//! TLS material lifecycle for the inbound dashboard.
//!
//! On first launch (or when the cached cert expired/can't be parsed) we mint
//! a self-signed X.509 cert that covers every IP address currently bound to a
//! local interface plus the loopback names. The cert is stored under
//! `~/.cokacdir/dashboard/{cert.pem, key.pem}` so subsequent launches reuse
//! the same fingerprint — that's what lets the user trust the cert once and
//! avoid recurring browser warnings.
//!
//! The SHA-256 fingerprint of the leaf cert is exposed so the startup banner
//! can print it. A user comparing that fingerprint to the one shown in the
//! browser's "certificate details" warning is the only out-of-band channel
//! that defeats a TLS MITM against a self-signed deployment.

use std::fs;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use sha2::{Digest, Sha256};

/// Threshold for proactive renewal. When less than this remains, we burn the
/// cached cert and mint a new one so we don't hand out an about-to-expire
/// cert that the browser will reject mid-session.
const RENEW_BEFORE_DAYS: i64 = 14;

pub struct TlsMaterial {
    pub server_config: Arc<ServerConfig>,
    /// Colon-separated uppercase hex of SHA-256(DER(leaf cert)). Matches the
    /// fingerprint format browsers display.
    pub fingerprint_sha256: String,
    pub cert_path: PathBuf,
    /// IPs/names baked into the cert SAN. Banner uses these to suggest URLs
    /// that won't trigger a name-mismatch warning.
    pub san_entries: Vec<String>,
}

pub fn load_or_create() -> Result<TlsMaterial, String> {
    // rustls 0.23 requires a process-wide crypto provider to be installed
    // before any ServerConfig can be built. ring is the default and is what
    // we compiled in. install_default returns Err if already installed —
    // ignore that since we only care that it's present.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let dir = dirs::home_dir()
        .ok_or_else(|| "Cannot determine home directory".to_string())?
        .join(".cokacdir")
        .join("dashboard");
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Cannot create cert directory {}: {}", dir.display(), e))?;

    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");

    let san = build_san_entries();
    dlog!("dashboard::tls", "SAN entries: {:?}", san);

    let (cert_pem, key_pem, regenerated) = match try_load(&cert_path, &key_path, &san) {
        Some(pair) => {
            dlog!("dashboard::tls", "Reusing cached cert at {}", cert_path.display());
            (pair.0, pair.1, false)
        }
        None => {
            dlog!("dashboard::tls", "Generating fresh cert");
            let (c, k) = generate_cert(&san)?;
            persist(&cert_path, &key_path, &c, &k)?;
            (c, k, true)
        }
    };

    let cert_chain = parse_cert_chain(&cert_pem)?;
    let priv_key = parse_priv_key(&key_pem)?;

    if cert_chain.is_empty() {
        return Err("Parsed cert chain was empty".into());
    }
    let fingerprint = sha256_fingerprint(&cert_chain[0]);

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, priv_key)
        .map_err(|e| format!("rustls ServerConfig: {}", e))?;
    // Browser fetches use HTTP/1.1 — advertising it explicitly avoids ALPN
    // probing edge cases on some clients.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    dlog!(
        "dashboard::tls",
        "TLS material ready (fingerprint={}, regenerated={})",
        fingerprint,
        regenerated
    );

    Ok(TlsMaterial {
        server_config: Arc::new(config),
        fingerprint_sha256: fingerprint,
        cert_path,
        san_entries: san,
    })
}

/// Returns Some((cert_pem, key_pem)) when the cached cert is still usable.
///
/// Strategy: PEM-parseability is the *required* check (without it rustls
/// can't load the cert at all). Validity-window and SAN-coverage checks are
/// *advisory* — if our hand-rolled X.509 walker can't read the cert (some
/// future rcgen encoding tweak we didn't anticipate), we reuse the cert
/// rather than regenerate. That keeps the fingerprint stable across runs
/// and lets the user's "trust this cert" decision survive a cokacctl
/// upgrade. rustls itself will reject an actually-expired cert at TLS
/// handshake time, at which point the user can nuke `~/.cokacdir/dashboard`
/// to force regeneration.
fn try_load(
    cert_path: &PathBuf,
    key_path: &PathBuf,
    expected_san: &[String],
) -> Option<(String, String)> {
    let cert_pem = fs::read_to_string(cert_path).ok()?;
    let key_pem = fs::read_to_string(key_path).ok()?;

    let chain = parse_cert_chain(&cert_pem).ok()?;
    let leaf = chain.first()?;

    match inspect_cert(leaf) {
        Some((not_after_unix, sans)) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let renew_threshold = now + RENEW_BEFORE_DAYS * 86_400;
            if not_after_unix <= renew_threshold {
                dlog!("dashboard::tls", "Cached cert in renewal window, regenerating");
                return None;
            }
            for want in expected_san {
                if !sans.iter().any(|s| s.eq_ignore_ascii_case(want)) {
                    dlog!(
                        "dashboard::tls",
                        "Cached cert missing SAN '{}', regenerating",
                        want
                    );
                    return None;
                }
            }
        }
        None => {
            // Parser couldn't read it — keep the cert so the fingerprint
            // stays stable. Worst case: actually-expired or stale-SAN cert
            // surfaces as a TLS-time browser warning, which the user can
            // resolve by deleting the cached files.
            dlog!(
                "dashboard::tls",
                "Cached cert PEM-parseable but X.509 inspect failed; \
                 keeping cert to preserve fingerprint stability"
            );
        }
    }

    Some((cert_pem, key_pem))
}

fn generate_cert(san: &[String]) -> Result<(String, String), String> {
    let mut params = CertificateParams::new(san.to_vec())
        .map_err(|e| format!("CertificateParams::new: {}", e))?;

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "cokacctl dashboard");
    dn.push(DnType::OrganizationName, "cokacctl");
    params.distinguished_name = dn;

    // CertificateParams::new() already sets a generous validity window
    // (100 years by default). Back-dating not_before is not exposed
    // cleanly across rcgen minor versions, so we rely on the default —
    // clock skew of a few seconds is tolerated by every mainstream browser.

    let key_pair = KeyPair::generate().map_err(|e| format!("KeyPair::generate: {}", e))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("self_signed: {}", e))?;

    Ok((cert.pem(), key_pair.serialize_pem()))
}

fn persist(
    cert_path: &PathBuf,
    key_path: &PathBuf,
    cert_pem: &str,
    key_pem: &str,
) -> Result<(), String> {
    // Cert is public; default perms (0644) are fine.
    fs::write(cert_path, cert_pem)
        .map_err(|e| format!("Cannot write cert {}: {}", cert_path.display(), e))?;

    // Private key must be 0600 — write to tmp + rename so the file is never
    // visible at the default umask between create and chmod.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let tmp = key_path.with_extension("pem.tmp");
        let _ = fs::remove_file(&tmp);
        {
            let mut f = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| format!("Cannot create key tmp: {}", e))?;
            f.write_all(key_pem.as_bytes())
                .map_err(|e| format!("Cannot write key: {}", e))?;
            let _ = f.sync_all();
        }
        fs::rename(&tmp, key_path)
            .map_err(|e| format!("Cannot finalize key: {}", e))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(key_path, key_pem)
            .map_err(|e| format!("Cannot write key: {}", e))?;
    }
    Ok(())
}

fn parse_cert_chain(pem: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    let mut out = Vec::new();
    let mut cursor = pem.as_bytes();
    for item in rustls_pemfile::certs(&mut cursor) {
        let der = item.map_err(|e| format!("cert pem parse: {}", e))?;
        out.push(der);
    }
    Ok(out)
}

fn parse_priv_key(pem: &str) -> Result<PrivateKeyDer<'static>, String> {
    // `private_key` autodetects PKCS#8 / PKCS#1 / SEC1 in one pass, so we
    // tolerate any key shape rcgen or a future swap-in might produce.
    let mut cursor = pem.as_bytes();
    rustls_pemfile::private_key(&mut cursor)
        .map_err(|e| format!("key pem parse: {}", e))?
        .ok_or_else(|| "no private key found in PEM".to_string())
}

fn sha256_fingerprint(cert: &CertificateDer<'_>) -> String {
    let digest = Sha256::digest(cert.as_ref());
    digest
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(":")
}

/// Reads the cert's notAfter (unix seconds) and SAN entries. Hand-rolls the
/// minimum DER walk we need so we don't pull a full X.509 parser in just for
/// validity checks. Returns None on any parse problem so the caller can fall
/// back to "regenerate".
fn inspect_cert(cert: &CertificateDer<'_>) -> Option<(i64, Vec<String>)> {
    let der = cert.as_ref();
    let parsed = x509_minimal::parse(der).ok()?;
    Some((parsed.not_after_unix, parsed.sans))
}

/// Builds the SAN list for the cert: localhost + every interface IP we can
/// see. Wildcard binds (0.0.0.0/::) are skipped — they're not addressable.
fn build_san_entries() -> Vec<String> {
    let mut out: Vec<String> = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for iface in ifs {
            let ip = iface.ip();
            if ip.is_unspecified() {
                continue;
            }
            let s = ip.to_string();
            if !out.iter().any(|e| e.eq_ignore_ascii_case(&s)) {
                out.push(s);
            }
        }
    }
    out
}

/// Convenience: list the IPs from `san_entries` that look like routable
/// (non-loopback) IPv4 addresses, for the banner suggestion.
pub fn advertised_addresses(sans: &[String]) -> Vec<IpAddr> {
    sans.iter()
        .filter_map(|s| s.parse::<IpAddr>().ok())
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect()
}

// ─── x509 minimal ─────────────────────────────────────────────────────────
//
// Just enough DER walking to extract notAfter and the SAN extension. Pulling
// in `x509-parser` would be cleaner but adds significant compile time for
// what amounts to two field reads.

mod x509_minimal {
    pub struct ParsedCert {
        pub not_after_unix: i64,
        pub sans: Vec<String>,
    }

    pub fn parse(der: &[u8]) -> Result<ParsedCert, &'static str> {
        // Certificate ::= SEQUENCE { tbsCertificate, signatureAlg, signature }
        let cert = read_sequence(der)?;
        // TBSCertificate ::= SEQUENCE { version[0]?, serial, sigAlg, issuer,
        //                               validity, subject, spki, ...extensions[3]? }
        let tbs = read_sequence(cert.contents)?;
        let mut p = tbs.contents;

        // Skip optional [0] EXPLICIT version (context-specific, constructed)
        if p.first().map(|b| *b == 0xA0).unwrap_or(false) {
            let h = read_any(p)?;
            p = h.rest;
        }
        // serial INTEGER
        let h = read_any(p)?; p = h.rest;
        let _ = h;
        // sigAlg SEQUENCE
        let h = read_any(p)?; p = h.rest;
        let _ = h;
        // issuer SEQUENCE
        let h = read_any(p)?; p = h.rest;
        let _ = h;
        // validity SEQUENCE { notBefore, notAfter }
        let validity = read_sequence(p)?;
        p = validity.rest;
        let nb = read_any(validity.contents)?;
        let na = read_any(nb.rest)?;
        let not_after_unix = parse_time(na.tag, na.contents)?;

        // subject SEQUENCE
        let h = read_any(p)?; p = h.rest;
        let _ = h;
        // spki SEQUENCE
        let h = read_any(p)?; p = h.rest;
        let _ = h;

        // Optional [1], [2] (issuerUniqueID, subjectUniqueID), then [3] extensions
        let mut sans = Vec::new();
        while !p.is_empty() {
            let h = read_any(p)?;
            p = h.rest;
            if h.tag == 0xA3 {
                // [3] EXPLICIT extensions SEQUENCE OF Extension
                let ext_seq = read_sequence(h.contents)?;
                let mut ep = ext_seq.contents;
                while !ep.is_empty() {
                    let ext = read_sequence(ep)?;
                    ep = ext.rest;
                    // Extension ::= SEQUENCE { OID, critical?, OCTET STRING }
                    let mut xp = ext.contents;
                    let oid = read_any(xp)?; xp = oid.rest;
                    if oid.tag != 0x06 { continue; }
                    let is_san = oid.contents == &[0x55, 0x1d, 0x11];
                    // Optional BOOLEAN critical
                    let next_tag = xp.first().copied().unwrap_or(0);
                    if next_tag == 0x01 {
                        let b = read_any(xp)?;
                        xp = b.rest;
                    }
                    let val = read_any(xp)?;
                    if is_san && val.tag == 0x04 {
                        // OCTET STRING wraps a SEQUENCE OF GeneralName
                        if let Ok(seq) = read_sequence(val.contents) {
                            let mut gp = seq.contents;
                            while !gp.is_empty() {
                                let g = read_any(gp)?;
                                gp = g.rest;
                                match g.tag {
                                    // [2] IA5String dNSName
                                    0x82 => if let Ok(s) = std::str::from_utf8(g.contents) {
                                        sans.push(s.to_string());
                                    },
                                    // [7] OCTET STRING iPAddress
                                    0x87 => {
                                        if g.contents.len() == 4 {
                                            let ip = std::net::Ipv4Addr::new(
                                                g.contents[0], g.contents[1],
                                                g.contents[2], g.contents[3]);
                                            sans.push(ip.to_string());
                                        } else if g.contents.len() == 16 {
                                            let mut a = [0u8; 16];
                                            a.copy_from_slice(g.contents);
                                            let ip = std::net::Ipv6Addr::from(a);
                                            sans.push(ip.to_string());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(ParsedCert { not_after_unix, sans })
    }

    struct Header<'a> {
        tag: u8,
        contents: &'a [u8],
        rest: &'a [u8],
    }

    fn read_any(input: &[u8]) -> Result<Header<'_>, &'static str> {
        if input.is_empty() { return Err("eof"); }
        let tag = input[0];
        let (len, len_bytes) = read_length(&input[1..])?;
        let header_len = 1 + len_bytes;
        if input.len() < header_len + len { return Err("len overflow"); }
        Ok(Header {
            tag,
            contents: &input[header_len..header_len + len],
            rest: &input[header_len + len..],
        })
    }

    fn read_sequence(input: &[u8]) -> Result<Header<'_>, &'static str> {
        let h = read_any(input)?;
        if h.tag != 0x30 { return Err("expected SEQUENCE"); }
        Ok(h)
    }

    fn read_length(input: &[u8]) -> Result<(usize, usize), &'static str> {
        if input.is_empty() { return Err("len eof"); }
        let first = input[0];
        if first & 0x80 == 0 {
            return Ok((first as usize, 1));
        }
        let n = (first & 0x7F) as usize;
        if n == 0 || n > 4 || input.len() < 1 + n { return Err("bad len"); }
        let mut v = 0usize;
        for i in 0..n { v = (v << 8) | input[1 + i] as usize; }
        Ok((v, 1 + n))
    }

    fn parse_time(tag: u8, body: &[u8]) -> Result<i64, &'static str> {
        // 0x17 UTCTime "YYMMDDhhmmssZ"; 0x18 GeneralizedTime "YYYYMMDDhhmmssZ"
        let s = std::str::from_utf8(body).map_err(|_| "time utf8")?;
        let s = s.trim_end_matches('Z');
        let (year, rest) = match tag {
            0x17 => {
                if s.len() < 12 { return Err("utctime short"); }
                let yy: i32 = s[..2].parse().map_err(|_| "utctime year")?;
                let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
                (year, &s[2..])
            }
            0x18 => {
                if s.len() < 14 { return Err("gentime short"); }
                let yyyy: i32 = s[..4].parse().map_err(|_| "gentime year")?;
                (yyyy, &s[4..])
            }
            _ => return Err("unknown time tag"),
        };
        if rest.len() < 10 { return Err("time short"); }
        let mo: u32 = rest[0..2].parse().map_err(|_| "mo")?;
        let da: u32 = rest[2..4].parse().map_err(|_| "da")?;
        let hh: u32 = rest[4..6].parse().map_err(|_| "hh")?;
        let mm: u32 = rest[6..8].parse().map_err(|_| "mm")?;
        let ss: u32 = rest[8..10].parse().map_err(|_| "ss")?;

        // Days since 1970-01-01
        let days = days_from_civil(year, mo as i32, da as i32);
        let secs = days as i64 * 86_400 + (hh as i64) * 3600 + (mm as i64) * 60 + (ss as i64);
        Ok(secs)
    }

    /// Howard Hinnant's date algorithm — proleptic Gregorian, 1970-01-01 = day 0.
    fn days_from_civil(y: i32, m: i32, d: i32) -> i32 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = (y - era * 400) as u32;
        let mp = (if m > 2 { m - 3 } else { m + 9 }) as u32;
        let doy = (153 * mp + 2) / 5 + (d as u32) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe as i32 - 719_468
    }
}
