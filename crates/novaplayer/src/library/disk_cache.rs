//! Files kept on disk by a source's key, under a budget: the queue rows' words and thumbnails
//! (`row_cache`) and the plugins' faces (`plugins::client`). An entry is a few files of one
//! key in its group's folder (a song's folder, a plugin), and may name one shared file, a blob
//! kept once by its content (a thumbnail many songs of an album show).
//!
//! What is kept, by key, with each file's size, the blob it names and when it was last used, is
//! held in memory, read from the folder's listing once (`open`): a file's name says its key and
//! its blob, so no record is read to know it. Eviction (the least recently used entries first,
//! once the budget or the count is passed, down to nine tenths of them) and forgetting a group
//! work from it alone. Reads and writes of the files are the callers', outside the lock.

use crate::sync::Locked;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

/// The folder shared files are kept in.
const BLOBS: &str = "blobs";

/// One entry's files.
struct Entry {
    group: String,
    /// The blob it names (its files' names carry it).
    blob: Option<String>,
    /// Its files: extension, size.
    files: Vec<(String, u64)>,
    /// When it was last used (a count; a file's time at `open`).
    used: u64,
}

/// A shared file: its extension, size, and how many entries name it.
#[derive(Default)]
struct Blob {
    ext: String,
    len: u64,
    refs: usize,
}

#[derive(Default)]
struct Index {
    entries: HashMap<String, Entry>,
    /// Shared files.
    blobs: HashMap<String, Blob>,
    /// Every file's size, together.
    total: u64,
    clock: u64,
}

pub struct DiskCache {
    root: PathBuf,
    budget: u64,
    max_entries: usize,
    index: Mutex<Index>,
}

/// The key of `bytes` (a path's, a folder's, a picture's): the first half of their SHA-256, in
/// hex. Half keeps a kept file's path well inside Windows' 260 characters (a group, a key and a
/// blob in one name) with no collision in sight.
pub fn key(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut hex = String::with_capacity(32);
    for byte in &Sha256::digest(bytes)[..16] {
        // Writing to a String does not fail.
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// The key of a path, as the system spells it.
pub fn path_key(path: &Path) -> String {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        key(&path.as_os_str().encode_wide().flat_map(u16::to_le_bytes).collect::<Vec<_>>())
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::ffi::OsStrExt;
        key(path.as_os_str().as_bytes())
    }
}

/// Where shared file `blob` with extension `ext` is kept.
fn blob_path(root: &Path, blob: &str, ext: &str) -> PathBuf {
    root.join(BLOBS).join(format!("{blob}.{ext}"))
}

/// Where entry `key`'s `ext` file (naming `blob`) is kept in `group`.
fn entry_path(root: &Path, group: &str, key: &str, blob: Option<&str>, ext: &str) -> PathBuf {
    root.join(group).join(file_name(key, blob, ext))
}

impl Index {
    /// Using an entry: the clock moves a step and gives the new time.
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// A shared file no entry names goes: out of the map, its size out of the total, its file
    /// deleted.
    fn drop_blob(&mut self, root: &Path, b: &str) {
        if let Some(blob) = self.blobs.remove(b) {
            let _ = std::fs::remove_file(blob_path(root, b, &blob.ext));
            self.total = self.total.saturating_sub(blob.len);
        }
    }
}

fn file_name(key: &str, blob: Option<&str>, ext: &str) -> String {
    match blob {
        Some(b) => format!("{key}.{b}.{ext}"),
        None => format!("{key}.{ext}"),
    }
}

fn hex(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl DiskCache {
    /// The cache kept in `root`: its listing read now (a worker's: a few thousand names).
    pub fn open(root: PathBuf, budget: u64, max_entries: usize) -> Self {
        let mut index = Index::default();
        let time = |m: &std::fs::Metadata| {
            m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |t| t.as_secs())
        };
        for group in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let gname = group.file_name().to_string_lossy().into_owned();
            for f in std::fs::read_dir(group.path()).into_iter().flatten().flatten() {
                let Ok(m) = f.metadata() else { continue };
                let name = f.file_name().to_string_lossy().into_owned();
                let parts: Vec<&str> = name.split('.').collect();
                index.total += m.len();
                index.clock = index.clock.max(time(&m));
                if gname == BLOBS {
                    if let [b, ext] = parts[..] {
                        let blob = index.blobs.entry(b.to_owned()).or_default();
                        (blob.ext, blob.len) = (ext.to_owned(), m.len());
                    }
                    continue;
                }
                let (k, blob, ext) = match parts[..] {
                    [k, ext] if hex(k) => (k, None, ext),
                    [k, b, ext] if hex(k) && hex(b) => (k, Some(b), ext),
                    _ => continue,
                };
                let e = index.entries.entry(k.to_owned()).or_insert_with(|| Entry {
                    group: gname.clone(),
                    blob: blob.map(str::to_owned),
                    files: Vec::new(),
                    used: 0,
                });
                e.files.push((ext.to_owned(), m.len()));
                e.used = e.used.max(time(&m));
            }
        }
        let names: Vec<String> = index.entries.values().filter_map(|e| e.blob.clone()).collect();
        for b in names {
            index.blobs.entry(b).or_default().refs += 1;
        }
        // A shared file no entry names (a crash between its write and its entry's) goes.
        let orphans: Vec<String> = index.blobs.iter().filter(|(_, b)| b.refs == 0).map(|(k, _)| k.clone()).collect();
        for b in orphans {
            index.drop_blob(&root, &b);
        }
        let cache = DiskCache { root, budget, max_entries, index: Mutex::new(index) };
        cache.trim(&mut cache.index.locked());
        cache
    }

    /// Where entry `key`'s `ext` file is, if it is kept; reading it is use.
    pub fn find(&self, key: &str, ext: &str) -> Option<PathBuf> {
        let mut ix = self.index.locked();
        let clock = ix.tick();
        let e = ix.entries.get_mut(key)?;
        e.files.iter().any(|(x, _)| x == ext).then_some(())?;
        e.used = clock;
        Some(entry_path(&self.root, &e.group, key, e.blob.as_deref(), ext))
    }

    /// The blob entry `key` names, and where its `ext` file is.
    pub fn blob_of(&self, key: &str, ext: &str) -> Option<(String, PathBuf)> {
        let b = self.index.locked().entries.get(key)?.blob.clone()?;
        let path = blob_path(&self.root, &b, ext);
        Some((b, path))
    }

    /// Keeps entry `key`'s `ext` file in `group`, naming `blob` (written before by `put_blob`):
    /// written whole or not at all. Its other files go when the blob it names changes.
    pub fn put(&self, group: &str, key: &str, ext: &str, bytes: &[u8], blob: Option<&str>) -> std::io::Result<()> {
        std::fs::create_dir_all(self.root.join(group))?;
        crate::files::write_atomic(&entry_path(&self.root, group, key, blob, ext), bytes)?;
        let mut ix = self.index.locked();
        let moved = ix.entries.get(key).is_some_and(|e| e.group != group || e.blob.as_deref() != blob);
        if moved {
            // Another group or blob: the entry's old files go (their names say the old ones).
            self.drop_entry(&mut ix, key);
        }
        let clock = ix.tick();
        let added = blob.is_some() && !ix.entries.contains_key(key);
        let e = ix.entries.entry(key.to_owned()).or_insert_with(|| Entry {
            group: group.to_owned(),
            blob: blob.map(str::to_owned),
            files: Vec::new(),
            used: clock,
        });
        e.used = clock;
        let len = bytes.len() as u64;
        let was = match e.files.iter_mut().find(|(x, _)| x == ext) {
            Some(f) => std::mem::replace(&mut f.1, len),
            None => {
                e.files.push((ext.to_owned(), len));
                0
            }
        };
        ix.total = ix.total + len - was;
        if let (true, Some(b)) = (added, blob) {
            ix.blobs.entry(b.to_owned()).or_default().refs += 1;
        }
        self.trim(&mut ix);
        Ok(())
    }

    /// Keeps a shared file by its content's key, unless it is kept already with these bytes.
    pub fn put_blob(&self, blob: &str, ext: &str, bytes: &[u8]) -> std::io::Result<()> {
        let path = blob_path(&self.root, blob, ext);
        // Kept already, whole (a damaged file is written again).
        if std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64)
            && self.index.locked().blobs.contains_key(blob)
        {
            return Ok(());
        }
        std::fs::create_dir_all(self.root.join(BLOBS))?;
        crate::files::write_atomic(&path, bytes)?;
        let mut ix = self.index.locked();
        let old = ix.blobs.entry(blob.to_owned()).or_default();
        old.ext = ext.to_owned();
        let was = std::mem::replace(&mut old.len, bytes.len() as u64);
        ix.total = ix.total + bytes.len() as u64 - was;
        Ok(())
    }

    /// Entry `key`'s files go.
    pub fn forget(&self, key: &str) {
        self.drop_entry(&mut self.index.locked(), key);
    }

    /// One file of entry `key` goes (the others stay).
    pub fn forget_file(&self, key: &str, ext: &str) {
        let mut ix = self.index.locked();
        let Some(e) = ix.entries.get_mut(key) else { return };
        let Some(k) = e.files.iter().position(|(x, _)| x == ext) else { return };
        let (_, len) = e.files.remove(k);
        let path = entry_path(&self.root, &e.group, key, e.blob.as_deref(), ext);
        let _ = std::fs::remove_file(path);
        ix.total = ix.total.saturating_sub(len);
        if ix.entries.get(key).is_some_and(|e| e.files.is_empty()) {
            self.drop_entry(&mut ix, key);
        }
    }

    /// Every entry of `group` goes.
    pub fn forget_group(&self, group: &str) {
        let mut ix = self.index.locked();
        let keys: Vec<String> = ix.entries.iter().filter(|(_, e)| e.group == group).map(|(k, _)| k.clone()).collect();
        for k in keys {
            self.drop_entry(&mut ix, &k);
        }
    }

    /// Everything goes.
    pub fn clear(&self) {
        let mut ix = self.index.locked();
        let _ = std::fs::remove_dir_all(&self.root);
        *ix = Index::default();
    }

    fn drop_entry(&self, ix: &mut Index, key: &str) {
        let Some(e) = ix.entries.remove(key) else { return };
        for (ext, len) in &e.files {
            let _ = std::fs::remove_file(entry_path(&self.root, &e.group, key, e.blob.as_deref(), ext));
            ix.total = ix.total.saturating_sub(*len);
        }
        let Some(b) = e.blob else { return };
        let gone = ix.blobs.get_mut(&b).is_some_and(|r| {
            r.refs = r.refs.saturating_sub(1);
            r.refs == 0
        });
        if gone {
            ix.drop_blob(&self.root, &b);
        }
    }

    /// Over the budget or the count: the least recently used entries go, down to nine tenths
    /// of both (a cache at its edge does not trim at every write).
    fn trim(&self, ix: &mut Index) {
        if ix.total <= self.budget && ix.entries.len() <= self.max_entries {
            return;
        }
        let (budget, count) = (self.budget / 10 * 9, self.max_entries / 10 * 9);
        let mut order: Vec<(u64, String)> = ix.entries.iter().map(|(k, e)| (e.used, k.clone())).collect();
        order.sort_unstable();
        for (_, k) in order {
            if ix.total <= budget && ix.entries.len() <= count {
                break;
            }
            self.drop_entry(ix, &k);
        }
    }

    /// What is kept, together, and how many entries (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn size(&self) -> (u64, usize, usize) {
        let ix = self.index.locked();
        (ix.total, ix.entries.len(), ix.blobs.len())
    }
}
