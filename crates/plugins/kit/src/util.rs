//! Small encodings and randomness the service APIs need.

/// Standard base64 (with padding).
pub fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n =
            (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Lower-case hex.
pub fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex string → bytes (`None` on odd length or non-hex).
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    fn digit(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    s.as_bytes().chunks_exact(2).map(|p| Some(digit(p[0])? * 16 + digit(p[1])?)).collect()
}

/// Match a share link's exact HTTP(S) authority, never a host substring in its path/query.
/// Userinfo and non-default ports are deliberately not accepted for service share links.
pub fn has_web_host(url: &str, hosts: &[&str]) -> bool {
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    hosts.iter().any(|host| authority.eq_ignore_ascii_case(host))
}

/// Raw query value (no percent decoding); also works with SPA fragment routes containing '?'.
pub fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let (_, query) = url.split_once('?')?;
    query.split('#').next()?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

/// `application/x-www-form-urlencoded` for one value.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `a=1&b=2` from pairs (values encoded).
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v))).collect::<Vec<_>>().join("&")
}

#[allow(
    clippy::expect_used,
    reason = "a plugin that cannot read the system's randomness cannot sign in safely: it exits, and the host shows it as crashed"
)]
pub fn rand_bytes(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    getrandom::getrandom(&mut b).expect("operating system randomness unavailable");
    b
}

/// Random hex string, 2·n chars.
pub fn rand_hex(n: usize) -> String {
    hex(&rand_bytes(n))
}

const BASE62: &[u8; 62] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Random base62 string (NetEase's weapi secret key).
pub fn rand_base62(n: usize) -> String {
    rand_bytes(n).iter().map(|b| BASE62[*b as usize % 62] as char).collect()
}

/// Milliseconds since 1970-01-01 (UTC).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Seconds since 1970-01-01 (UTC).
pub fn now_s() -> u64 {
    now_ms() / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_works() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"foob"), "Zm9vYg==");
        assert_eq!(b64(b"hello world"), "aGVsbG8gd29ybGQ=");
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(unhex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(unhex("0"), None);
        assert_eq!(unhex("zz"), None);
        assert_eq!(unhex("中a"), None);
        assert_eq!(unhex("é"), None);
        assert_eq!(unhex("+1"), None);
        assert_eq!(unhex("aF"), Some(vec![175]));
    }

    #[test]
    fn urlencode_works() {
        assert_eq!(urlencode("a b&=中"), "a%20b%26%3D%E4%B8%AD");
        assert!(has_web_host("https://music.163.com/#/playlist?id=1", &["music.163.com"]));
        assert!(!has_web_host("https://evil.test/music.163.com?id=1", &["music.163.com"]));
        assert!(!has_web_host("https://music.163.com.evil.test/?id=1", &["music.163.com"]));
        assert_eq!(query_param("https://music.163.com/#/playlist?id=1&x=2", "id"), Some("1"));
        assert_eq!(query_param("https://music.163.com/?notid=1", "id"), None);
    }
}
