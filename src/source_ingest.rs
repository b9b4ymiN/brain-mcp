//! Source ingestion: SSRF-guarded fetch + deterministic chunking (Task D3).
//!
//! Feeds `brain_ingest_source` (URL/file/text → quarantine). The URL path is
//! the security-critical one (GOAL-vNext threat model TM-005: "SSRF,
//! private-IP pivot... during ingest"):
//!
//! - Scheme allowlist (`http`/`https` only — no `file:`, `ftp:`, `javascript:`).
//! - DNS resolution is validated BEFORE connecting, and the client is pinned
//!   to the exact validated address (`ClientBuilder::resolve`) — this closes
//!   the DNS-rebinding TOCTOU gap where a hostname could resolve to a safe IP
//!   during our check and a private one when the connection is actually made.
//! - Redirects are followed manually (`redirect::Policy::none()` + our own
//!   loop), re-validating scheme/DNS/IP on every hop — reqwest's built-in
//!   redirect handling does NOT re-run a caller's SSRF policy per hop, so an
//!   external URL that 302s to `http://169.254.169.254/` (cloud metadata) or
//!   `http://localhost/admin` would otherwise sail through unchecked.
//! - Response body is read through a hard byte cap regardless of what
//!   `Content-Length` claims (a malicious/misconfigured server can lie about
//!   it).
//! - `Content-Type` must match an allowlist before the body is even read.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

/// Every way a `fetch_url` call can be refused or fail. Each variant names
/// the specific guard that tripped, so a caller (and a test) can assert on
/// the exact denial reason rather than "it failed somehow".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestError {
    DisallowedScheme(String),
    UnresolvableHost(String),
    PrivateOrReservedIp(String),
    TooManyRedirects,
    ResponseTooLarge,
    DisallowedContentType(String),
    Http(String),
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngestError::DisallowedScheme(s) => write!(f, "disallowed URL scheme: {s}"),
            IngestError::UnresolvableHost(h) => write!(f, "could not resolve host: {h}"),
            IngestError::PrivateOrReservedIp(ip) => {
                write!(f, "refusing to fetch a private/reserved IP: {ip}")
            }
            IngestError::TooManyRedirects => write!(f, "too many redirects"),
            IngestError::ResponseTooLarge => write!(f, "response exceeded the size limit"),
            IngestError::DisallowedContentType(ct) => write!(f, "disallowed content type: {ct}"),
            IngestError::Http(msg) => write!(f, "http error: {msg}"),
        }
    }
}

impl std::error::Error for IngestError {}

/// Bounds for a single `fetch_url` call. `max_redirects` bounds the manual
/// redirect loop (no unbounded following); `max_body_bytes` is enforced by
/// actually counting bytes read, not by trusting `Content-Length`.
#[derive(Clone, Debug)]
pub struct IngestPolicy {
    pub max_redirects: u8,
    pub max_body_bytes: u64,
    pub allowed_content_types: Vec<String>,
    pub timeout: Duration,
}

impl Default for IngestPolicy {
    fn default() -> Self {
        Self {
            max_redirects: 5,
            max_body_bytes: 10 * 1024 * 1024,
            allowed_content_types: vec![
                "text/plain".to_owned(),
                "text/html".to_owned(),
                "text/markdown".to_owned(),
                "application/json".to_owned(),
            ],
            timeout: Duration::from_secs(30),
        }
    }
}

/// True if `ip` is a private, loopback, link-local, unspecified, or otherwise
/// non-routable-from-the-public-internet address — including the IPv4-mapped
/// IPv6 form (`::ffff:10.0.0.1`), which would otherwise sail past an
/// IPv6-only check. Conservative: over-blocking is the safe direction for an
/// egress filter (matches `provider::detect_secret`'s stated bias).
pub fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_ipv4(&mapped);
            }
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 — unique local addresses.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10 — link-local.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn is_blocked_ipv4(v4: &Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_documentation()
        // 100.64.0.0/10 — carrier-grade NAT (RFC 6598); not covered by
        // `is_private`, but not publicly routable either.
        || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64)
}

fn validate_scheme(url: &reqwest::Url) -> Result<(), IngestError> {
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(IngestError::DisallowedScheme(other.to_owned())),
    }
}

/// Resolve `host:port` and require EVERY returned address to pass
/// [`is_blocked_ip`] (conservative: a hostname with mixed public/private
/// answers is refused, not partially trusted). Returns the first validated
/// address so the caller can pin the HTTP client to it.
fn resolve_and_validate(host: &str, port: u16) -> Result<SocketAddr, IngestError> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|_| IngestError::UnresolvableHost(host.to_owned()))?
        .collect();
    let Some(first) = addrs.first().copied() else {
        return Err(IngestError::UnresolvableHost(host.to_owned()));
    };
    for addr in &addrs {
        if is_blocked_ip(&addr.ip()) {
            return Err(IngestError::PrivateOrReservedIp(addr.ip().to_string()));
        }
    }
    Ok(first)
}

/// Fetch `url_str` under `policy`, following redirects manually (each hop
/// re-validated) up to `policy.max_redirects`. Returns `(body_text,
/// content_type)` on success.
pub fn fetch_url(url_str: &str, policy: &IngestPolicy) -> Result<(String, String), IngestError> {
    let mut current = reqwest::Url::parse(url_str).map_err(|e| IngestError::Http(e.to_string()))?;

    for _hop in 0..=policy.max_redirects {
        validate_scheme(&current)?;
        let host = current
            .host_str()
            .ok_or_else(|| IngestError::UnresolvableHost(url_str.to_owned()))?
            .to_owned();
        let port = current
            .port_or_known_default()
            .ok_or_else(|| IngestError::DisallowedScheme(current.scheme().to_owned()))?;
        let pinned = resolve_and_validate(&host, port)?;

        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .resolve(&host, pinned)
            .timeout(policy.timeout)
            .build()
            .map_err(|e| IngestError::Http(e.to_string()))?;

        let response = client
            .get(current.clone())
            .send()
            .map_err(|e| IngestError::Http(e.to_string()))?;
        let status = response.status();

        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| IngestError::Http("redirect with no Location header".to_owned()))?;
            current = current
                .join(location)
                .map_err(|e| IngestError::Http(format!("invalid redirect location: {e}")))?;
            continue;
        }
        if !status.is_success() {
            return Err(IngestError::Http(format!("unexpected status {status}")));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !policy
            .allowed_content_types
            .iter()
            .any(|allowed| allowed == &content_type)
        {
            return Err(IngestError::DisallowedContentType(content_type));
        }

        // Read through a hard cap — never trust Content-Length alone.
        let mut limited = response.take(policy.max_body_bytes + 1);
        let mut buf = Vec::new();
        limited
            .read_to_end(&mut buf)
            .map_err(|e| IngestError::Http(e.to_string()))?;
        if buf.len() as u64 > policy.max_body_bytes {
            return Err(IngestError::ResponseTooLarge);
        }
        return Ok((String::from_utf8_lossy(&buf).into_owned(), content_type));
    }
    Err(IngestError::TooManyRedirects)
}

// ── Deterministic chunking ───────────────────────────────────────────────────

/// Split `text` into chunks on blank-line paragraph boundaries, packing
/// consecutive paragraphs up to `max_chunk_bytes` (a single paragraph longer
/// than that is its own oversized chunk rather than being split mid-sentence
/// — evidence exactness cares about not truncating a paragraph, not about a
/// hard byte ceiling). Each chunk is quarantined as one captured object, so
/// an evidence span that names "this whole capture" is exact by
/// construction — no partial byte-range plumbing needed in the store layer.
pub fn chunk_text(text: &str, max_chunk_bytes: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if paragraphs.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for paragraph in paragraphs {
        if !current.is_empty() && current.len() + 2 + paragraph.len() > max_chunk_bytes {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(paragraph);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_loopback_v4() {
        assert!(is_blocked_ip(&"127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_rfc1918_ranges() {
        for ip in ["10.0.0.1", "172.16.0.1", "172.31.255.255", "192.168.1.1"] {
            assert!(
                is_blocked_ip(&ip.parse().unwrap()),
                "{ip} should be blocked"
            );
        }
    }

    #[test]
    fn blocks_link_local_and_cloud_metadata() {
        // 169.254.169.254 is the AWS/GCP/Azure instance-metadata endpoint —
        // a canonical real-world SSRF target, and it's link-local range.
        assert!(is_blocked_ip(&"169.254.169.254".parse().unwrap()));
        assert!(is_blocked_ip(&"169.254.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_carrier_grade_nat() {
        assert!(is_blocked_ip(&"100.64.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"100.127.255.255".parse().unwrap()));
        assert!(!is_blocked_ip(&"100.128.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_ipv6_loopback_and_unique_local_and_link_local() {
        assert!(is_blocked_ip(&"::1".parse().unwrap()));
        assert!(is_blocked_ip(&"fc00::1".parse().unwrap()));
        assert!(is_blocked_ip(&"fd12:3456:789a::1".parse().unwrap()));
        assert!(is_blocked_ip(&"fe80::1".parse().unwrap()));
    }

    #[test]
    fn blocks_ipv4_mapped_ipv6_private_address() {
        // ::ffff:10.0.0.1 — an IPv6-only check would miss this.
        assert!(is_blocked_ip(&"::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"::ffff:127.0.0.1".parse().unwrap()));
    }

    // NOTE on what ISN'T unit-tested here: the manual redirect-revalidation
    // loop in `fetch_url` (re-running scheme+DNS+IP checks on every hop, the
    // property that actually stops "public URL 302s to an internal address")
    // can't be exercised against a local test server in-process — any such
    // server necessarily binds to loopback, which `is_blocked_ip` correctly
    // refuses at hop 0, before the redirect logic is ever reached. That's the
    // guard doing its job, not a test gap to route around with a workaround
    // server. The redirect-to-internal case is proven for real in the Task
    // D3 adversarial corpus (D3.4/D3.5), which fetches from a real reachable
    // host. What IS covered here is the actual security primitive
    // (`is_blocked_ip`, exhaustively) and that the loop calls it before
    // trusting the initial URL.

    #[test]
    fn allows_public_addresses() {
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34"] {
            assert!(
                !is_blocked_ip(&ip.parse().unwrap()),
                "{ip} should be allowed"
            );
        }
        assert!(!is_blocked_ip(&"2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn fetch_url_rejects_disallowed_scheme() {
        let err = fetch_url("file:///etc/passwd", &IngestPolicy::default()).unwrap_err();
        assert!(matches!(err, IngestError::DisallowedScheme(_)));
    }

    #[test]
    fn fetch_url_rejects_ftp_scheme() {
        let err = fetch_url("ftp://example.com/x", &IngestPolicy::default()).unwrap_err();
        assert!(matches!(err, IngestError::DisallowedScheme(_)));
    }

    #[test]
    fn fetch_url_rejects_direct_private_ip_literal() {
        let err = fetch_url("http://127.0.0.1:1/x", &IngestPolicy::default()).unwrap_err();
        assert!(matches!(err, IngestError::PrivateOrReservedIp(_)));
    }

    #[test]
    fn fetch_url_rejects_cloud_metadata_ip_literal() {
        let err = fetch_url(
            "http://169.254.169.254/latest/meta-data/",
            &IngestPolicy::default(),
        )
        .unwrap_err();
        assert!(matches!(err, IngestError::PrivateOrReservedIp(_)));
    }

    #[test]
    fn chunk_text_splits_on_paragraph_boundaries() {
        let text = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let chunks = chunk_text(text, 1000);
        assert_eq!(
            chunks,
            vec!["First paragraph.\n\nSecond paragraph.\n\nThird paragraph."]
        );
    }

    #[test]
    fn chunk_text_packs_up_to_the_byte_limit() {
        let text = "aaaa\n\nbbbb\n\ncccc\n\ndddd";
        let chunks = chunk_text(text, 10);
        // Each pair packs to exactly 10 bytes ("aaaa\n\nbbbb", "cccc\n\ndddd").
        assert_eq!(chunks, vec!["aaaa\n\nbbbb", "cccc\n\ndddd"]);
    }

    #[test]
    fn chunk_text_keeps_an_oversized_paragraph_whole() {
        let long = "x".repeat(50);
        let chunks = chunk_text(&long, 10);
        assert_eq!(chunks, vec![long]);
    }

    #[test]
    fn chunk_text_of_empty_input_is_empty() {
        assert!(chunk_text("   \n\n  ", 100).is_empty());
    }
}
