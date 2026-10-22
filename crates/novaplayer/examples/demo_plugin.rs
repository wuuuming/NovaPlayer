//! The demo plugin: every extension point of `docs/plugins.md`, with no network and no account.
//! The tests run it as a real child process; plugin authors can read it as a template.
//!
//! - decode `.tone`: a text file `<hz> <seconds>` played as a 48 kHz stereo sine (PCM over the
//!   pipe, any position);
//! - links `demo:<n>`: a list of n tone songs, as an M3U8 text of `demo:tone:<hz>` URIs;
//! - resolve `demo:` URIs to a local WAV it writes, at an option `quality` (标准 / 高);
//! - metadata: fills in an album for the songs it resolves;
//! - connect: a QR sign-in (`connect.*`) that succeeds after two polls, or a `form` that succeeds at once;
//! - `slow` / `hang` / `crash`: misbehaviours for the host's pipelining, timeout and restart.
//!
//! It declares `"concurrent": true`: each request runs on its own thread and answers when it is
//! done, in any order, so a slow call never holds up the others. Answers share one locked
//! stdout, so a line (and the raw bytes after it) is never torn.

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

type Out = Arc<Mutex<std::io::Stdout>>;

fn main() {
    let stdin = std::io::stdin();
    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    let polls = Arc::new(AtomicU32::new(0));
    for line in stdin.lock().lines().map_while(Result::ok) {
        let (out, polls) = (out.clone(), polls.clone());
        std::thread::spawn(move || handle(&line, &out, &polls));
    }
}

fn handle(line: &str, out: &Out, polls: &AtomicU32) {
    let id = field_num(line, "id").unwrap_or(0.0) as u64;
    let call = field_str(line, "call").unwrap_or_default();
    match call.as_str() {
        "hello" => reply(out, id, &format!(r#""name":"Demo","api":{}"#, novaplayer_plugin_protocol::API)),
        "open" => {
            // A `.tone` file: "<hz> <seconds>".
            let path = field_str(line, "path").unwrap_or_default();
            match std::fs::read_to_string(&path).ok().and_then(|t| parse_tone(&t)) {
                Some((_, secs)) => {
                    reply(out, id, &format!(r#""rate":48000,"channels":2,"frames":{}"#, (secs * 48000.0) as u64))
                }
                None => fail(out, id, "not a tone file"),
            }
        }
        "read" => {
            // PCM s16le interleaved: `frames` frames from frame `at`.
            let path = field_str(line, "path").unwrap_or_default();
            let at = field_num(line, "at").unwrap_or(0.0) as u64;
            let want = field_num(line, "frames").unwrap_or(4800.0) as u64;
            let Some((hz, secs)) = std::fs::read_to_string(&path).ok().and_then(|t| parse_tone(&t)) else {
                return fail(out, id, "not a tone file");
            };
            let total = (secs * 48000.0) as u64;
            let n = want.min(total.saturating_sub(at));
            let mut pcm = Vec::with_capacity(n as usize * 4);
            for k in at..at + n {
                let v = ((k as f64 / 48000.0 * hz * std::f64::consts::TAU).sin() * 8000.0) as i16;
                pcm.extend_from_slice(&v.to_le_bytes());
                pcm.extend_from_slice(&v.to_le_bytes());
            }
            let mut o = out.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = writeln!(o, r#"{{"id":{id},"ok":true,"frames":{n},"bytes":{}}}"#, pcm.len());
            let _ = o.write_all(&pcm);
            let _ = o.flush();
        }
        "playlist" => {
            let url = field_str(line, "url").unwrap_or_default();
            let Some(n) = url.strip_prefix("demo:").and_then(|n| n.parse::<u32>().ok()) else {
                return fail(out, id, "not a demo link");
            };
            // An ordinary extended M3U8; its entries are URIs only this plugin resolves.
            let mut m3u = format!("#EXTM3U\n#PLAYLIST:Demo list {n}\n");
            for k in 1..=n.min(500) {
                m3u.push_str(&format!("#EXTINF:2,Demo - Tone {k}\ndemo:tone:{}\n", 220 + k * 20));
            }
            reply(out, id, &format!(r#""m3u8":{}"#, quote(&m3u)));
        }
        "options" => reply(
            out,
            id,
            r#""values":[{"id":"high","name":"高","label":"48 kHz / 16-bit PCM"},{"id":"std","name":"标准","label":"48 kHz / 16-bit PCM"}]"#,
        ),
        "resolve" => {
            // A playable address for one of its URIs. The demo hands back a local file it
            // writes (a real plugin returns the platform's URL).
            let uri = field_str(line, "uri").unwrap_or_default();
            let opt = field_str(line, "option").unwrap_or_else(|| "std".into());
            let name = if opt == "high" { "高" } else { "标准" };
            let hz: f64 = uri.strip_prefix("demo:tone:").and_then(|h| h.parse().ok()).unwrap_or(440.0);
            let dir = std::env::temp_dir().join("novaplayer-demo-plugin");
            let _ = std::fs::create_dir_all(&dir);
            let p = dir.join(format!("{hz}.wav"));
            if !p.exists() {
                let _ = std::fs::write(&p, wav(hz, 2.0));
            }
            reply(
                out,
                id,
                &format!(
                    r#""url":{},"option":{},"option_name":{}"#,
                    quote(&p.to_string_lossy()),
                    quote(&opt),
                    quote(name)
                ),
            );
        }
        "metadata" => {
            let uri = field_str(line, "uri").unwrap_or_default();
            if uri.starts_with("demo:") {
                reply(out, id, r#""album":"Demo Album","genre":"Test""#);
            } else {
                reply(out, id, "");
            }
        }
        "connect.begin" => {
            // A form's values came with the call: its first poll is the answer.
            let form = field_str(line, "flow").as_deref() == Some("form");
            polls.store(u32::from(form), Ordering::Relaxed);
            if form {
                reply(out, id, r#""session":"demo-1","poll_ms":500,"expires_s":60"#);
            } else {
                reply(out, id, r#""session":"demo-1","show":{"qr":"demo://signin/123"},"poll_ms":500,"expires_s":60"#);
            }
        }
        "connect.poll" => {
            if field_str(line, "session").as_deref() != Some("demo-1") {
                return fail(out, id, "no such session");
            }
            if polls.fetch_add(1, Ordering::Relaxed) + 1 >= 2 {
                reply(out, id, r#""state":"done","credential":"demo-credential","display_name":"demo-user""#);
            } else {
                reply(out, id, r#""state":"waiting""#);
            }
        }
        "connect.cancel" | "connect.end" => reply(out, id, ""),
        "play_event" => reply(out, id, ""),
        // Takes `ms` to answer: the host's other calls must not wait for it.
        "slow" => {
            std::thread::sleep(std::time::Duration::from_millis(field_num(line, "ms").unwrap_or(1000.0) as u64));
            reply(out, id, "");
        }
        "hang" => std::thread::sleep(std::time::Duration::from_secs(3600)),
        "crash" => std::process::exit(3),
        _ => fail(out, id, "unknown call"),
    }
}

fn reply(out: &Out, id: u64, fields: &str) {
    let sep = if fields.is_empty() { "" } else { "," };
    let mut o = out.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = writeln!(o, r#"{{"id":{id},"ok":true{sep}{fields}}}"#);
    let _ = o.flush();
}

fn fail(out: &Out, id: u64, why: &str) {
    let mut o = out.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = writeln!(o, r#"{{"id":{id},"ok":false,"error":{}}}"#, quote(why));
    let _ = o.flush();
}

fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn parse_tone(t: &str) -> Option<(f64, f64)> {
    let mut it = t.split_whitespace();
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

fn wav(hz: f64, secs: f64) -> Vec<u8> {
    let n = (secs * 48000.0) as u32;
    let data = n * 4;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&48000u32.to_le_bytes());
    b.extend_from_slice(&(48000u32 * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for k in 0..n {
        let v = ((k as f64 / 48000.0 * hz * std::f64::consts::TAU).sin() * 8000.0) as i16;
        b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

/// Minimal field readers: the host writes flat objects with string / number values.
fn field_str(line: &str, k: &str) -> Option<String> {
    let key = format!("\"{k}\":\"");
    let i = line.find(&key)? + key.len();
    let mut s = String::new();
    let mut esc = false;
    let mut chars = line[i..].chars();
    while let Some(c) = chars.next() {
        if esc {
            match c {
                'n' => s.push('\n'),
                't' => s.push('\t'),
                'r' => s.push('\r'),
                'u' => {
                    let h: String = chars.by_ref().take(4).collect();
                    s.push(u32::from_str_radix(&h, 16).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}'));
                }
                c => s.push(c),
            }
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '"' {
            return Some(s);
        } else {
            s.push(c);
        }
    }
    None
}

fn field_num(line: &str, k: &str) -> Option<f64> {
    let key = format!("\"{k}\":");
    let i = line.find(&key)? + key.len();
    let t: String =
        line[i..].chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == 'e').collect();
    t.parse().ok()
}
