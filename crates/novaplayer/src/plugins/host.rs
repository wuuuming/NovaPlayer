//! Running enabled plugins: one resident child process each, spoken to in JSON lines over its
//! stdin/stdout (`docs/plugins.md`).
//!
//! A request is one line `{"id":N,"call":"<name>",...params}`; the answer is one line
//! `{"id":N,"ok":true,...}` or `{"id":N,"ok":false,"error":"..."}`. An answer may announce raw
//! bytes after it (`"bytes":K`): exactly K bytes follow the line's `\n` (PCM for `read`).
//!
//! Answers are matched to their calls by `id`: a reader thread hands each to the caller waiting
//! for it, so a plugin that declares `concurrent` has many calls in flight at once. Any other
//! plugin is served one call at a time (`Gate`).
//!
//! A call that gets no answer in its time is dropped; if nothing else waits on the plugin it is
//! hung, so it is killed and the next call starts it again. One that exits is started again on
//! its next call, at most three times a minute. A plugin's stderr goes to the player's log,
//! prefixed with its id.

use crate::sync::Locked;
use novaplayer_plugin_protocol::json::{self, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One answer: its fields, and the raw bytes that followed it (if it announced any).
#[derive(Debug)]
pub struct Reply {
    pub value: Value,
    pub bytes: Vec<u8>,
}

impl Reply {
    pub fn str(&self, k: &str) -> Option<&str> {
        self.value.get(k).and_then(Value::as_str)
    }
    pub fn num(&self, k: &str) -> Option<f64> {
        self.value.get(k).and_then(Value::as_f64)
    }

    /// A word of the answer: as given, none when it is missing, empty or only spaces.
    pub fn text(&self, k: &str) -> Option<String> {
        self.str(k).filter(|v| !v.trim().is_empty()).map(str::to_owned)
    }

    /// The song's words an answer gives (`open`'s, `metadata`'s): the keys `title`, `artist`,
    /// `album`, `album_artist`, `track`, `date`, `genre` (plugins.md), each a `text`.
    pub fn tags(&self) -> crate::library::Tags {
        crate::library::Tags {
            title: self.text("title"),
            artist: self.text("artist"),
            album: self.text("album"),
            album_artist: self.text("album_artist"),
            track: self.text("track"),
            date: self.text("date"),
            genre: self.text("genre"),
            duration: None,
        }
    }
}

enum Msg {
    Reply(Reply),
    Gone(String),
}

/// The calls waiting for their answer, by request id. Shared with the reader thread, which
/// hands each answer to its caller and, when the plugin's output ends, fails them all.
#[derive(Default)]
struct Pending {
    calls: HashMap<u64, Sender<Msg>>,
    /// Why the plugin's output ended (no call can be made on this process any more).
    gone: Option<String>,
}

struct Proc {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<Pending>>,
    next: AtomicU64,
}

impl Proc {
    fn alive(&self) -> bool {
        self.pending.locked().gone.is_none() && self.child.locked().try_wait().ok().flatten().is_none()
    }

    fn kill(&self) {
        let _ = self.child.locked().kill();
    }

    /// Killed and gone: its program's file is no longer in use when this returns.
    fn stop(&self) {
        let mut child = self.child.locked();
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

struct Slot {
    exec: PathBuf,
    dir: PathBuf,
    proc: Option<Arc<Proc>>,
    /// Start times in the last minute (restart limit).
    starts: VecDeque<Instant>,
}

/// The protocol's "one call at a time" makes a plugin's calls a queue. Playback's calls
/// (decode reads, resolving the stream) go ahead of the queue; the rest wait while one of them
/// waits. A call already running still runs to its end.
#[derive(Default)]
struct Gate {
    state: Mutex<(bool, usize)>, // (a call is running, playback calls waiting)
    turn: std::sync::Condvar,
}

fn urgent(call: &str) -> bool {
    matches!(call, "open" | "read" | "resolve" | "stream")
}

impl Gate {
    fn enter(&self, urgent: bool) {
        let mut g = self.state.locked();
        if urgent {
            g.1 += 1;
            while g.0 {
                g = crate::sync::wait(&self.turn, g);
            }
            g.1 -= 1;
        } else {
            while g.0 || g.1 > 0 {
                g = crate::sync::wait(&self.turn, g);
            }
        }
        g.0 = true;
    }
    fn leave(&self) {
        self.state.locked().0 = false;
        self.turn.notify_all();
    }
}

struct Turn<'a>(&'a Gate);
impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.0.leave();
    }
}

#[derive(Clone)]
struct Entry {
    gate: Arc<Gate>,
    slot: Arc<Mutex<Slot>>,
    /// The manifest's `concurrent`: the plugin answers overlapping calls by `id`, in any order.
    concurrent: bool,
}

/// The running plugins. Calls are synchronous; the player makes them from worker threads.
#[derive(Clone, Default)]
pub struct Host {
    slots: Arc<Mutex<BTreeMap<String, Entry>>>,
}

const RESTARTS_PER_MINUTE: usize = 3;

impl Host {
    /// Makes `id` callable (it starts on its first call). Re-registering after a change stops the
    /// running instance.
    pub fn register(&self, id: &str, dir: PathBuf, exec: PathBuf, concurrent: bool) {
        let slot = Slot { exec, dir, proc: None, starts: VecDeque::new() };
        let entry = Entry { gate: Arc::default(), slot: Arc::new(Mutex::new(slot)), concurrent };
        self.slots.locked().insert(id.to_owned(), entry);
    }

    /// Makes `id` uncallable and stops its process now, not when the last call on it ends: its
    /// folder may be about to change (`Registry::install`, `remove`). Calls in flight fail.
    pub fn unregister(&self, id: &str) {
        let entry = self.slots.locked().remove(id);
        let running = entry.and_then(|e| e.slot.locked().proc.take());
        if let Some(p) = running {
            p.stop();
        }
    }

    pub fn registered(&self, id: &str) -> bool {
        self.slots.locked().contains_key(id)
    }

    /// The plugin's running process, started if it is not (at most three starts a minute).
    fn process(id: &str, slot: &Mutex<Slot>) -> Result<Arc<Proc>, String> {
        let mut s = slot.locked();
        if s.proc.as_ref().is_some_and(|p| !p.alive()) {
            s.proc = None;
        }
        if let Some(p) = &s.proc {
            return Ok(p.clone());
        }
        let now = Instant::now();
        while s.starts.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60)) {
            s.starts.pop_front();
        }
        if s.starts.len() >= RESTARTS_PER_MINUTE {
            return Err(novaui::i18n::t("err.plugin-crashing", &[("id", id)]));
        }
        s.starts.push_back(now);
        let p = Arc::new(spawn(id, &s.exec, &s.dir)?);
        s.proc = Some(p.clone());
        Ok(p)
    }

    /// Forgets `p` if it is still the slot's process (the next call starts a fresh one).
    fn retire(slot: &Mutex<Slot>, p: &Arc<Proc>) {
        let mut s = slot.locked();
        if s.proc.as_ref().is_some_and(|c| Arc::ptr_eq(c, p)) {
            s.proc = None;
        }
    }

    /// Calls `call` with `params` (an object's fields) and waits up to `timeout` for the answer.
    /// `call`, waited for as long as the protocol gives it (`novaplayer_plugin_protocol::timeout`).
    pub fn ask(&self, id: &str, call: &str, params: &[(&str, Value)]) -> Result<Reply, String> {
        self.call(id, call, params, novaplayer_plugin_protocol::timeout(call))
    }

    pub fn call(&self, id: &str, call: &str, params: &[(&str, Value)], timeout: Duration) -> Result<Reply, String> {
        let entry =
            self.slots.locked().get(id).cloned().ok_or_else(|| novaui::i18n::t("err.plugin-off", &[("id", id)]))?;
        // A plugin takes its calls one at a time; a concurrent one is not queued.
        let _turn = (!entry.concurrent).then(|| {
            entry.gate.enter(urgent(call));
            Turn(&entry.gate)
        });
        let p = Self::process(id, &entry.slot)?;
        let n = p.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = p.pending.locked();
            if let Some(why) = &pending.gone {
                return Err(novaui::i18n::t("err.plugin-exited-why", &[("id", id), ("why", why)]));
            }
            pending.calls.insert(n, tx);
        }
        let mut fields: BTreeMap<String, Value> = params.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect();
        fields.insert("id".into(), Value::Num(n as f64));
        fields.insert("call".into(), Value::Str(call.into()));
        // The player's language, on every request: it may change between two (`plugins.md`).
        fields.insert("locale".into(), Value::Str(novaui::i18n::global().locale()));
        let line = json::write(&Value::Obj(fields));
        let wrote = {
            let mut w = p.stdin.locked();
            writeln!(w, "{line}").and_then(|_| w.flush())
        };
        if wrote.is_err() {
            p.pending.locked().calls.remove(&n);
            Self::retire(&entry.slot, &p);
            return Err(novaui::i18n::t("err.plugin-exited", &[("id", id)]));
        }
        match rx.recv_timeout(timeout) {
            Ok(Msg::Reply(r)) => {
                if r.value.get("ok").and_then(Value::as_bool) == Some(true) {
                    Ok(r)
                } else {
                    Err(r.str("error").map_or_else(|| novaui::i18n::t("err.plugin-error", &[]), str::to_owned))
                }
            }
            Ok(Msg::Gone(why)) => {
                Self::retire(&entry.slot, &p);
                Err(novaui::i18n::t("err.plugin-exited-why", &[("id", id), ("why", &why)]))
            }
            Err(RecvTimeoutError::Timeout) => {
                // Dropped. With nothing else waiting on it the plugin is hung: it answers nothing
                // again, so it is killed. With other calls in flight it may only be busy.
                let idle = {
                    let mut pending = p.pending.locked();
                    pending.calls.remove(&n);
                    pending.calls.is_empty()
                };
                if idle {
                    p.kill();
                    Self::retire(&entry.slot, &p);
                }
                Err(novaui::i18n::t("err.plugin-timeout", &[("id", id), ("s", &timeout.as_secs().to_string())]))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Self::retire(&entry.slot, &p);
                Err(novaui::i18n::t("err.plugin-exited", &[("id", id)]))
            }
        }
    }
}

fn spawn(id: &str, exec: &std::path::Path, dir: &std::path::Path) -> Result<Proc, String> {
    let mut cmd = Command::new(exec);
    cmd.current_dir(dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child =
        cmd.spawn().map_err(|e| novaui::i18n::t("err.plugin-start", &[("id", id), ("e", &e.to_string())]))?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let pending: Arc<Mutex<Pending>> = Arc::default();
    let reader = pending.clone();
    let tag = id.to_owned();
    std::thread::Builder::new()
        .name(format!("plugin-{id}"))
        .spawn(move || read_loop(stdout, &reader))
        .map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name(format!("plugin-{id}-err"))
        .spawn(move || log_lines(stderr, |line| eprintln!("[plugin {tag}] {line}")))
        .map_err(|e| e.to_string())?;
    Ok(Proc { child: Mutex::new(child), stdin: Mutex::new(stdin), pending, next: AtomicU64::new(0) })
}

/// The most raw bytes one answer may carry.
const MAX_BYTES: usize = 64 << 20;
/// The longest stderr line logged whole; a longer one is logged in pieces this long.
const MAX_LOG_LINE: usize = 64 << 10;

/// A plugin's stderr, line by line to `log` (the player's log). Never holds more than
/// `MAX_LOG_LINE` of it, whatever the plugin writes.
fn log_lines(stderr: impl Read, mut log: impl FnMut(&str)) {
    let mut r = BufReader::new(stderr);
    let mut line = Vec::new();
    loop {
        line.clear();
        match r.by_ref().take(MAX_LOG_LINE as u64).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => log(String::from_utf8_lossy(&line).trim_end()),
        }
    }
}

/// An answer's `bytes` block, read as it arrives: a plugin that announces more than it sends
/// makes the host hold only what it sent.
fn read_block(r: &mut impl Read, k: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    r.take(k as u64).read_to_end(&mut bytes).ok()?;
    (bytes.len() == k).then_some(bytes)
}

fn read_loop(stdout: impl Read, pending: &Mutex<Pending>) {
    // The output ended: every call waiting is failed, and no new one is accepted.
    let end = |why: String| {
        let mut p = pending.locked();
        for (_, tx) in p.calls.drain() {
            let _ = tx.send(Msg::Gone(why.clone()));
        }
        p.gone = Some(why);
    };
    let mut r = BufReader::with_capacity(1 << 16, stdout);
    loop {
        let mut line = Vec::new();
        // The line and its `\r\n`.
        let longest = novaplayer_plugin_protocol::MAX_LINE_BYTES + 2;
        match r.by_ref().take(longest as u64).read_until(b'\n', &mut line) {
            Ok(0) => return end("output ended".into()),
            Err(e) => return end(e.to_string()),
            Ok(_) => {}
        }
        if line.last() != Some(&b'\n') {
            return end("output line too long".into());
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let Ok(value) = json::parse(text) else {
            return end(format!("output is not JSON: {}", text.chars().take(80).collect::<String>()));
        };
        let k = value.get("bytes").and_then(Value::as_f64).map_or(0, |n| n.max(0.0) as usize);
        if k > MAX_BYTES {
            return end("data block too large".into());
        }
        let Some(bytes) = read_block(&mut r, k) else {
            return end("data block incomplete".into());
        };
        // An answer nobody waits for (its call timed out) is dropped; so is a line with no id.
        let Some(id) = value.get("id").and_then(Value::as_u64) else { continue };
        let waiting = pending.locked().calls.remove(&id);
        if let Some(tx) = waiting {
            let _ = tx.send(Msg::Reply(Reply { value, bytes }));
        }
    }
}

/// The demo plugin (`examples/demo_plugin.rs`), compiled once per process with the toolchain's
/// `rustc` beside the test binary (`<target>/<profile>/demo-plugin/`), against the protocol crate
/// this binary was built with: the tests here and the scenarios, which reach it from another
/// workspace, where Cargo cannot build this package's example.
#[cfg(any(test, feature = "test-support"))]
pub fn demo_plugin() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let fail = |why: String| -> ! { panic!("the demo plugin did not build: {why}") };
            let exe = std::env::current_exe().unwrap_or_else(|e| fail(e.to_string()));
            // <target>/<profile>/deps/<test>.exe
            let Some(profile) = exe.parent().and_then(|deps| deps.parent()) else { fail(exe.display().to_string()) };
            let dir = profile.join("demo-plugin");
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| fail(e.to_string()));
            let out = dir.join(format!("demo_plugin{}", std::env::consts::EXE_SUFFIX));
            let rustc =
                std::path::Path::new(env!("CARGO")).with_file_name(format!("rustc{}", std::env::consts::EXE_SUFFIX));
            let source = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/demo_plugin.rs");
            let deps = profile.join("deps");
            let protocol = std::fs::read_dir(&deps)
                .unwrap_or_else(|e| fail(e.to_string()))
                .flatten()
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    name.starts_with("libnovaplayer_plugin_protocol-") && name.ends_with(".rlib")
                })
                .max_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok())
                .unwrap_or_else(|| fail(format!("no novaplayer_plugin_protocol rlib in {}", deps.display())))
                .path();
            let status = Command::new(if rustc.is_file() { rustc.as_os_str() } else { "rustc".as_ref() })
                // Abort, as the release profile builds the protocol crate (an unwinding one links too).
                .args(["--edition", "2021", "-O", "-C", "panic=abort", "--crate-name", "demo_plugin", source, "-L"])
                .arg(&deps)
                .arg("--extern")
                .arg(format!("novaplayer_plugin_protocol={}", protocol.display()))
                .arg("-o")
                .arg(&out)
                .status()
                .unwrap_or_else(|e| fail(e.to_string()));
            if !status.success() || !out.is_file() {
                fail(format!("rustc {status}"));
            }
            out
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One rule for an answer's words wherever they are read: one of spaces only is none.
    #[test]
    fn an_answer_word_of_spaces_is_none() {
        let reply = Reply {
            value: json::obj(vec![("title", Value::Str("  ".into())), ("artist", Value::Str("A b".into()))]),
            bytes: Vec::new(),
        };
        let tags = reply.tags();
        assert_eq!((tags.title, tags.artist.as_deref(), tags.album), (None, Some("A b"), None));
    }

    /// Regression: stderr was read with `lines()` (a plugin writing no newline grew
    /// the host without bound), and an answer's announced `bytes` were allocated before a byte
    /// arrived.
    #[test]
    fn stderr_and_blocks_hold_no_more_than_what_is_bounded_or_sent() {
        let endless = std::io::repeat(b'x').take(MAX_LOG_LINE as u64 * 3 + 5);
        let mut pieces = Vec::new();
        log_lines(endless.chain(&b"\nlast\r\n"[..]), |l| pieces.push(l.len()));
        assert_eq!(pieces, [MAX_LOG_LINE, MAX_LOG_LINE, MAX_LOG_LINE, 5, 4]);

        assert_eq!(read_block(&mut &b"abcdef"[..], 4).as_deref(), Some(&b"abcd"[..]));
        assert_eq!(read_block(&mut &b"ab"[..], MAX_BYTES), None, "announced more than sent");
    }

    fn host() -> Host {
        crate::app::speak_zh_cn();
        let h = Host::default();
        let p = demo_plugin();
        h.register("demo", p.parent().unwrap().to_path_buf(), p, true);
        h
    }

    #[test]
    fn calls_answers_and_raw_bytes() {
        let h = host();
        let r = h.call("demo", "hello", &[], Duration::from_secs(10)).unwrap();
        assert_eq!(r.str("name"), Some("Demo"));
        let tone = std::env::temp_dir().join(format!("np-host-{}.tone", std::process::id()));
        std::fs::write(&tone, "440 1.5").unwrap();
        let path = Value::Str(tone.to_string_lossy().into());
        let r = h.call("demo", "open", &[("path", path.clone())], Duration::from_secs(5)).unwrap();
        assert_eq!(r.num("frames"), Some(72000.0));
        let r = h
            .call(
                "demo",
                "read",
                &[("path", path), ("at", Value::Num(71000.0)), ("frames", Value::Num(4800.0))],
                Duration::from_secs(5),
            )
            .unwrap();
        assert_eq!(r.num("frames"), Some(1000.0));
        assert_eq!(r.bytes.len(), 1000 * 4);
        let _ = std::fs::remove_file(tone);
        let e = h.call("demo", "nope", &[], Duration::from_secs(5)).unwrap_err();
        assert!(e.contains("unknown call"), "{e}");
    }

    #[test]
    fn a_hung_plugin_times_out_and_a_crashed_one_restarts() {
        let h = host();
        let t = Instant::now();
        let e = h.call("demo", "hang", &[], Duration::from_millis(500)).unwrap_err();
        assert!(e.contains("无响应"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(3));
        // Killed; the next call starts it again.
        assert!(h.call("demo", "hello", &[], Duration::from_secs(10)).is_ok());
        let e = h.call("demo", "crash", &[], Duration::from_secs(5)).unwrap_err();
        assert!(e.contains("退出"), "{e}");
        assert!(h.call("demo", "hello", &[], Duration::from_secs(10)).is_ok());
        // Three starts a minute at most.
        let _ = h.call("demo", "crash", &[], Duration::from_secs(5));
        let e = h.call("demo", "hello", &[], Duration::from_secs(5)).unwrap_err();
        assert!(e.contains("反复退出"), "{e}");
    }

    /// A concurrent plugin answers a quick call while a slow one is still running.
    #[test]
    fn a_slow_call_does_not_hold_up_the_others() {
        let h = host();
        assert!(h.call("demo", "hello", &[], Duration::from_secs(10)).is_ok());
        let slow = {
            let h = h.clone();
            std::thread::spawn(move || h.call("demo", "slow", &[("ms", Value::Num(1500.0))], Duration::from_secs(10)))
        };
        std::thread::sleep(Duration::from_millis(300));
        let t = Instant::now();
        assert!(h.call("demo", "hello", &[], Duration::from_secs(5)).is_ok());
        assert!(t.elapsed() < Duration::from_millis(1000), "waited for the slow call: {:?}", t.elapsed());
        assert!(slow.join().unwrap().is_ok());
    }

    #[test]
    fn qr_signin_waits_then_returns_a_credential() {
        let h = host();
        let r = h.call("demo", "connect.begin", &[("flow", Value::Str("qr".into()))], Duration::from_secs(10)).unwrap();
        let session = r.str("session").unwrap().to_owned();
        assert!(r.value.at(&["show", "qr"]).and_then(Value::as_str).is_some_and(|q| !q.is_empty()));
        let poll = || {
            h.call("demo", "connect.poll", &[("session", Value::Str(session.clone()))], Duration::from_secs(5)).unwrap()
        };
        assert_eq!(poll().str("state"), Some("waiting"));
        let r = poll();
        assert_eq!(r.str("state"), Some("done"));
        assert_eq!(r.str("credential"), Some("demo-credential"));
    }

    /// Regression (plugin host bounds): stderr was read with unbounded `lines()`,
    /// and an answer's `bytes` block was allocated whole before any of it arrived.
    #[test]
    fn what_a_plugin_writes_is_held_only_as_far_as_the_bounds() {
        let mut pieces = Vec::new();
        let long = "x".repeat(MAX_LOG_LINE * 2 + 10);
        log_lines(format!("first\r\n{long}\nlast").as_bytes(), |l| pieces.push(l.len()));
        assert_eq!(pieces, [5, MAX_LOG_LINE, MAX_LOG_LINE, 10, 4]);

        // Announced at a terabyte, ten bytes sent: refused, with only what came in held.
        assert_eq!(read_block(&mut &b"0123456789"[..], 1 << 40), None);
        assert_eq!(read_block(&mut &b"0123456789tail"[..], 10).as_deref(), Some(&b"0123456789"[..]));
    }
}
