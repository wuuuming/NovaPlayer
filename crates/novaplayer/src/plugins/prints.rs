//! Plugin fingerprints (`registry::fingerprint`): what the user confirmed when enabling, checked
//! against a plugin's files on every scan. Hashing a plugin reads all of its files, so it is
//! done once per change, on a worker (principles R4): a fingerprint is kept with each file's
//! length and modification time, saved in `fingerprints.txt` beside `enabled.txt`, and a scan
//! that finds the files as they were takes it from there without reading them.

use crate::sync::Locked;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

/// One file of a plugin's folder: its path inside the folder (`/`-separated), length and
/// modification time (nanoseconds since 1970).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    rel: String,
    len: u64,
    modified: u128,
}

#[derive(Default)]
struct Cache {
    /// Folder name → the files as they were when hashed, and their fingerprint.
    known: HashMap<String, (Vec<Seen>, String)>,
    /// Folders a worker is hashing now.
    busy: HashSet<PathBuf>,
}

/// The fingerprints of one plugins folder, shared with the workers that compute them.
#[derive(Clone)]
pub struct Prints {
    file: Option<PathBuf>,
    cache: Arc<Mutex<Cache>>,
    /// Called on a worker when it has stored a fingerprint (the app rescans).
    done: Arc<Mutex<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(any(test, feature = "test-support"))]
    workers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

impl Prints {
    /// The fingerprints saved under `root` (none when it has no file yet).
    pub fn open(root: Option<&Path>) -> Self {
        let file = root.map(|r| r.join("fingerprints.txt"));
        let known =
            file.as_deref().and_then(|f| std::fs::read_to_string(f).ok()).map(|t| parse(&t)).unwrap_or_default();
        Prints {
            file,
            cache: Arc::new(Mutex::new(Cache { known, busy: HashSet::new() })),
            done: Arc::new(Mutex::new(Arc::new(|| {}))),
            #[cfg(any(test, feature = "test-support"))]
            workers: Arc::default(),
        }
    }

    /// What a worker calls once it has stored a fingerprint.
    pub fn on_done(&self, done: impl Fn() + Send + Sync + 'static) {
        *self.done.locked() = Arc::new(done);
    }

    /// `dir`'s fingerprint, when its files are as they were when it was last hashed. Only
    /// reads the folder's listing and each file's length and time.
    pub fn known(&self, dir: &Path) -> Option<String> {
        let name = dir.file_name()?.to_string_lossy().into_owned();
        let seen = walk(dir).into_iter().map(|(s, _)| s).collect::<Vec<_>>();
        let cache = self.cache.locked();
        cache.known.get(&name).filter(|(was, _)| *was == seen).map(|(_, print)| print.clone())
    }

    /// `dir`'s fingerprint now: known, or hashed here (the caller's thread).
    pub fn now(&self, dir: &Path) -> String {
        self.known(dir).unwrap_or_else(|| self.hash(dir))
    }

    /// Hashes `dir` on a worker, unless one is already at it; `on_done` follows.
    pub fn check(&self, dir: &Path) {
        if !self.cache.locked().busy.insert(dir.to_path_buf()) {
            return;
        }
        let (this, folder) = (self.clone(), dir.to_path_buf());
        let spawned = std::thread::Builder::new().name("plugin-fingerprint".into()).spawn(move || {
            this.hash(&folder);
            this.cache.locked().busy.remove(&folder);
            let done = this.done.locked().clone();
            done();
        });
        match spawned {
            #[cfg(any(test, feature = "test-support"))]
            Ok(worker) => self.workers.locked().push(worker),
            #[cfg(not(any(test, feature = "test-support")))]
            Ok(_) => {}
            Err(_) => {
                self.cache.locked().busy.remove(dir);
            }
        }
    }

    /// Tests: waits for every worker started so far.
    #[cfg(any(test, feature = "test-support"))]
    pub fn settle(&self) {
        let workers = std::mem::take(&mut *self.workers.locked());
        for w in workers {
            let _ = w.join();
        }
    }

    /// Reads and hashes every file of `dir`, keeps the result with the lengths and times seen
    /// before reading (a file that changes meanwhile is hashed again on the next scan), saves.
    fn hash(&self, dir: &Path) -> String {
        let files = walk(dir);
        let mut h = Sha256::new();
        for (seen, path) in &files {
            h.update(seen.rel.as_bytes());
            h.update([0]);
            h.update(std::fs::read(path).unwrap_or_default());
            h.update([0]);
        }
        let print: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let Some(name) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else { return print };
        let text = {
            let mut cache = self.cache.locked();
            cache.known.insert(name, (files.into_iter().map(|(s, _)| s).collect(), print.clone()));
            write(&cache.known)
        };
        if let Some(f) = &self.file {
            let _ = crate::files::write_atomic(f, text.as_bytes());
        }
        print
    }
}

/// Every file under `dir`, sorted by its path inside it.
fn walk(dir: &Path) -> Vec<(Seen, PathBuf)> {
    fn go(base: &Path, d: &Path, out: &mut Vec<(Seen, PathBuf)>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                go(base, &p, out);
            } else if let Ok(rel) = p.strip_prefix(base) {
                let meta = std::fs::metadata(&p).ok();
                let modified = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                let rel = rel.to_string_lossy().replace('\\', "/");
                out.push((Seen { rel, len: meta.map_or(0, |m| m.len()), modified }, p));
            }
        }
    }
    let mut out = Vec::new();
    go(dir, dir, &mut out);
    out.sort_by(|a, b| a.0.rel.cmp(&b.0.rel));
    out
}

/// `fingerprints.txt`: per plugin, `<folder>\t<fingerprint>`, then one line per file,
/// ` <length> <time> <path>`. A folder or path holding a tab or a line break is not kept.
fn write(known: &HashMap<String, (Vec<Seen>, String)>) -> String {
    let plain = |s: &str| !s.contains(['\t', '\n', '\r']);
    let mut names: Vec<_> = known.keys().filter(|n| plain(n)).collect();
    names.sort();
    let mut out = String::new();
    for name in names {
        let (files, print) = &known[name];
        if !files.iter().all(|f| plain(&f.rel)) {
            continue;
        }
        out.push_str(&format!("{name}\t{print}\n"));
        for f in files {
            out.push_str(&format!(" {} {} {}\n", f.len, f.modified, f.rel));
        }
    }
    out
}

fn parse(text: &str) -> HashMap<String, (Vec<Seen>, String)> {
    let mut known = HashMap::new();
    let mut current: Option<(String, Vec<Seen>, String)> = None;
    for line in text.lines() {
        if let Some(file) = line.strip_prefix(' ') {
            let mut parts = file.splitn(3, ' ');
            let (Some(len), Some(modified), Some(rel)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let (Ok(len), Ok(modified)) = (len.parse(), modified.parse()) else { continue };
            if let Some((_, files, _)) = &mut current {
                files.push(Seen { rel: rel.to_owned(), len, modified });
            }
        } else if let Some((name, print)) = line.split_once('\t') {
            if let Some((n, files, p)) = current.take() {
                known.insert(n, (files, p));
            }
            current = Some((name.to_owned(), Vec::new(), print.to_owned()));
        }
    }
    if let Some((n, files, p)) = current {
        known.insert(n, (files, p));
    }
    known
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (principles R4): every enabled plugin's files were hashed in full on the UI
    /// thread at start and on each enable or reload.
    #[test]
    fn a_fingerprint_is_kept_by_length_and_time_and_worked_out_on_a_worker() {
        let root = std::env::temp_dir().join(format!("np-prints-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("demo");
        std::fs::create_dir_all(dir.join("face")).unwrap();
        std::fs::write(dir.join("plugin.json"), b"{}").unwrap();
        std::fs::write(dir.join("face").join("a b.nui"), b"row").unwrap();

        let prints = Prints::open(Some(&root));
        assert_eq!(prints.known(&dir), None);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        prints.on_done(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
        prints.check(&dir);
        prints.settle();
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        let print = prints.known(&dir).expect("hashed by the worker");

        // Saved: another opening knows it without hashing.
        assert_eq!(Prints::open(Some(&root)).known(&dir).as_deref(), Some(print.as_str()));
        // A file of another length is not the file that was hashed.
        std::fs::write(dir.join("plugin.json"), b"{ }").unwrap();
        assert_eq!(prints.known(&dir), None);
        assert_ne!(prints.now(&dir), print);
        let _ = std::fs::remove_dir_all(&root);
    }
}
