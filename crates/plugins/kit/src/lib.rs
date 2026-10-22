//! Shared plumbing for NovaPlayer service plugins (docs/plugins.md, protocol v2).
//!
//! A plugin is a resident child process spoken to in JSON lines over stdin/stdout. This crate
//! carries the boring parts so a service plugin only implements its calls:
//!
//! - [`json`]: a small JSON reader/writer (the same grammar the host uses);
//! - [`m3u8`]: the extended M3U8 text a `playlist` answers with;
//! - [`pipe`]: the stdin/stdout loop: parse a request, run the handler, write the answer;
//! - [`http`]: blocking HTTPS with per-call headers/cookies, redirect control, Set-Cookie
//!   capture (login flows live on cookies);
//! - [`util`]: base64, hex, URL-encoding, random strings.

pub mod http;
pub mod json;
pub mod layers;
pub mod m3u8;
pub mod pipe;
pub mod responses;
pub mod util;
