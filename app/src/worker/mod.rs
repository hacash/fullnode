//! Standalone mining clients. They use the full node HTTP API and do not own
//! chain state, P2P, or server lifecycle.

mod json;

pub mod diamond;
pub mod pow;

use sys::{Ret, errf};

/// Host:port (or host:port/prefix) for plaintext `http://` only.
/// `https://` is rejected; a leading `http://` is stripped.
fn plain_http_host(raw: &str) -> Ret<String> {
    let s = raw.trim();
    if s.is_empty() {
        return errf!("connect address is empty");
    }
    if s.len() >= 8 && s[..8].eq_ignore_ascii_case("https://") {
        return errf!("worker HTTP is plaintext only; https is not supported: {s}");
    }
    let s = if s.len() >= 7 && s[..7].eq_ignore_ascii_case("http://") {
        &s[7..]
    } else {
        s
    };
    let s = s.trim().trim_end_matches('/');
    if s.is_empty() {
        return errf!("connect address is empty");
    }
    Ok(s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::plain_http_host;

    #[test]
    fn plain_http_host_strips_scheme_and_rejects_tls() {
        assert_eq!(plain_http_host("127.0.0.1:8082").unwrap(), "127.0.0.1:8082");
        assert_eq!(
            plain_http_host("http://127.0.0.1:8082/").unwrap(),
            "127.0.0.1:8082"
        );
        assert!(plain_http_host("https://127.0.0.1:8082").is_err());
        assert!(plain_http_host("").is_err());
    }
}
