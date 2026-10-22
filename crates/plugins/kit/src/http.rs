//! Blocking HTTPS for the service APIs: per-call headers and cookies, redirect control, and
//! Set-Cookie capture (the login flows live on cookies). One instance per plugin process.

use crate::json::{self, Value};
use std::cell::Cell;
use std::time::{Duration, Instant};

pub struct Http {
    follow: ureq::Agent,
    nofollow: ureq::Agent,
    timeout: Duration,
    deadline: Cell<Option<Instant>>,
}

/// One response: status, body, the Set-Cookie pairs it carried, and its headers.
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub cookies: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
}

impl Response {
    pub fn text(&self) -> Result<String, String> {
        String::from_utf8(self.body.clone()).map_err(|_| "response is not UTF-8".to_string())
    }
    pub fn json(&self) -> Result<Value, String> {
        json::parse(&self.text()?)
    }
    /// The value of the first `Set-Cookie: name=…`.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.cookies.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
    }
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }
}

fn err(e: ureq::Error) -> String {
    format!("network: {e}")
}

impl Http {
    pub fn new(timeout: Duration) -> Self {
        let agent = |redirects: u32| {
            let config = ureq::Agent::config_builder()
                .timeout_global(Some(timeout))
                .http_status_as_error(false)
                .max_redirects(redirects)
                .build();
            ureq::Agent::new_with_config(config)
        };
        Http { follow: agent(5), nofollow: agent(0), timeout, deadline: Cell::new(None) }
    }

    /// One end-to-end budget, including authentication, pagination and fallbacks.
    /// Leave time for JSON serialization and pipe writes before the host kills the process.
    pub fn begin_call(&self, call: &str) {
        self.deadline.set(Some(Instant::now() + novaplayer_plugin_protocol::budget(call)));
    }

    pub fn remaining(&self) -> Result<Duration, String> {
        let remaining =
            self.deadline.get().map(|d| d.saturating_duration_since(Instant::now())).unwrap_or(self.timeout);
        if remaining.is_zero() {
            return Err("request budget exhausted".into());
        }
        Ok(remaining.min(self.timeout))
    }

    /// GET `url`; `redirects`: follow them (default) or return the 3xx itself (login flows
    /// read its cookies and Location).
    pub fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        cookies: &[(&str, &str)],
        redirects: bool,
    ) -> Result<Response, String> {
        let agent = if redirects { &self.follow } else { &self.nofollow };
        let mut req = agent.get(url).config().timeout_global(Some(self.remaining()?)).build();
        for &(k, v) in headers {
            req = req.header(k, v);
        }
        if !cookies.is_empty() {
            req = req.header("Cookie", cookie_header(cookies));
        }
        finish(req.call().map_err(err)?)
    }

    /// POST with an `application/x-www-form-urlencoded` body.
    pub fn post_form(
        &self,
        url: &str,
        form: &str,
        headers: &[(&str, &str)],
        cookies: &[(&str, &str)],
        redirects: bool,
    ) -> Result<Response, String> {
        self.post(url, form.as_bytes(), "application/x-www-form-urlencoded", headers, cookies, redirects)
    }

    /// POST with an `application/json` body.
    pub fn post_json(
        &self,
        url: &str,
        body: &str,
        headers: &[(&str, &str)],
        cookies: &[(&str, &str)],
    ) -> Result<Response, String> {
        self.post(url, body.as_bytes(), "application/json", headers, cookies, false)
    }

    fn post(
        &self,
        url: &str,
        body: &[u8],
        content_type: &str,
        headers: &[(&str, &str)],
        cookies: &[(&str, &str)],
        redirects: bool,
    ) -> Result<Response, String> {
        let agent = if redirects { &self.follow } else { &self.nofollow };
        let mut req =
            agent.post(url).config().timeout_global(Some(self.remaining()?)).build().content_type(content_type);
        for &(k, v) in headers {
            req = req.header(k, v);
        }
        if !cookies.is_empty() {
            req = req.header("Cookie", cookie_header(cookies));
        }
        finish(req.send(body).map_err(err)?)
    }
}

pub fn cookie_header(cookies: &[(&str, &str)]) -> String {
    cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
}

fn finish(res: ureq::http::Response<ureq::Body>) -> Result<Response, String> {
    let status = res.status().as_u16();
    if status >= 400 {
        return Err(format!("HTTP {status}"));
    }
    let headers = res.headers();
    let mut cookies = Vec::new();
    for v in headers.get_all("set-cookie") {
        let Ok(v) = v.to_str() else { continue };
        // "name=value; Path=/; …" — the pair before the first ';'.
        let pair = v.split(';').next().unwrap_or("");
        if let Some((n, val)) = pair.split_once('=') {
            cookies.push((n.trim().to_string(), val.trim().to_string()));
        }
    }
    let mut hs = Vec::new();
    for (n, v) in headers.iter() {
        if let Ok(v) = v.to_str() {
            hs.push((n.as_str().to_string(), v.to_string()));
        }
    }
    let mut body = res.into_body();
    let bytes = body.with_config().limit(16 * 1024 * 1024).read_to_vec().map_err(|e| format!("network: {e}"))?;
    Ok(Response { status, body: bytes, cookies, headers: hs })
}
