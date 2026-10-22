//! The stdin/stdout loop (docs/plugins.md §The pipe): one request line in, one answer line
//! out; nothing else on stdout (diagnostics go to stderr, the host logs them under the
//! plugin's id).

use crate::json::{self, Value};
use novaplayer_plugin_protocol::MAX_LINE_BYTES;
use std::collections::BTreeMap;
use std::io::{self, BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

/// The request being answered is in a Chinese locale (`locale`, docs/plugins.md §The pipe).
static ZH: AtomicBool = AtomicBool::new(false);

/// The text in the player's language for the request being answered: `zh` for a Chinese
/// locale, else `en` (English when the request names none).
pub fn tr<'a>(zh: &'a str, en: &'a str) -> &'a str {
    if ZH.load(Ordering::Relaxed) {
        zh
    } else {
        en
    }
}

/// What a call handler returns: ok with extra answer fields, or an error message.
pub type Answer = Result<Vec<(String, Value)>, String>;

pub fn ok(fields: Vec<(String, Value)>) -> Answer {
    Ok(fields)
}

pub fn fail(why: impl Into<String>) -> Answer {
    Err(why.into())
}

/// Runs the protocol loop. `hello` is answered here (`name`, `api 3`); everything else goes
/// to `handler`, which gets the whole request object (use `req.get("call")`, …).
pub fn run(name: &str, mut handler: impl FnMut(&Value) -> Answer) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    let mut input = stdin.lock();
    loop {
        let line = match read_line(&mut input) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                eprintln!("bad request line: {e}");
                break;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req = match json::parse(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("bad request line: {e}");
                continue;
            }
        };
        let Some(id) = req.get("id").and_then(Value::as_u64) else {
            eprintln!("request missing integer id");
            continue;
        };
        let zh = req.get("locale").and_then(Value::as_str).is_some_and(|l| l.starts_with("zh"));
        ZH.store(zh, Ordering::Relaxed);
        let answer = if req.get("call").and_then(Value::as_str) == Some("hello") {
            ok(vec![
                ("name".into(), Value::str(name)),
                ("api".into(), Value::Num(novaplayer_plugin_protocol::API.into())),
            ])
        } else {
            handler(&req)
        };
        let mut m: BTreeMap<String, Value> = BTreeMap::new();
        m.insert("id".into(), Value::Num(id as f64));
        match answer {
            Ok(fields) => {
                m.insert("ok".into(), Value::Bool(true));
                for (k, v) in fields {
                    if k != "id" && k != "ok" {
                        m.insert(k, v);
                    }
                }
            }
            Err(e) => {
                m.insert("ok".into(), Value::Bool(false));
                m.insert("error".into(), Value::str(e));
            }
        }
        let mut line = json::write(&Value::Obj(m));
        if line.len() > MAX_LINE_BYTES {
            line = json::write(&json::obj(vec![
                ("id", Value::Num(id as f64)),
                ("ok", Value::Bool(false)),
                ("error", Value::str("answer exceeds protocol line limit")),
            ]));
        }
        if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}

/// Bounded even when a peer never sends a newline. Oversized input terminates the pipe.
fn read_line(input: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    input.take((MAX_LINE_BYTES + 2) as u64).read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.len() > MAX_LINE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "request exceeds protocol line limit"));
    }
    String::from_utf8(bytes).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// A call's string parameter.
pub fn str_param<'a>(req: &'a Value, k: &str) -> Option<&'a str> {
    req.get(k).and_then(Value::as_str)
}
