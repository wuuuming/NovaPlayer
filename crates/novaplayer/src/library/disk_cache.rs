//! Files kept on disk by a key, under a budget: the library index's records and pictures
//! (`index`) and the plugins' faces (`plugins::client`). An entry is a few files of one key in
//! its group's folder (a song's folder, a plugin, the pictures).
//!
//! What is kept, by key, with each file's size and when it was last used, is held in memory,
//! read from the folder's listing once (`open`): a file's name says its key, so no record is read
//! to know it. Eviction (the least recently used entries first,
//! once the budget or the count is passed, down to nine tenths of them) and forgetting a group
//! work from it alone. Reads and writes of the files are the callers', outside the lock.

use crate::sync::Locked;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

/// One entry's files.
struct Entry {
    group: String,
    /// Its files: extension, size.
    files: Vec<(String, u64)>,
    /// When it was last used (a count; a file's time at `open`).
    used: u64,
}

#[derive(Default)]
struct Index {
    entries: HashMap<String, Entry>,
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
/// hex. Half keeps a kept file's path well inside Windows' 260 characters (a group and a key in
/// one name) with no collision in sight.
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

/// Where entry `key`'s `ext` file is kept in `group`.
fn entry_path(root: &Path, group: &str, key: &str, ext: &str) -> PathBuf {
    root.join(group).join(format!("{key}.{ext}"))
}

impl Index {
    /// Using an entry: the clock moves a step and gives the new time.
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
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
                let [k, ext] = parts[..] else { continue };
                if !hex(k) {
                    continue;
                }
                index.total += m.len();
                index.clock = index.clock.max(time(&m));
                let e = index.entries.entry(k.to_owned()).or_insert_with(|| Entry {
                    group: gname.clone(),
                    files: Vec::new(),
                    used: 0,
                });
                e.files.push((ext.to_owned(), m.len()));
                e.used = e.used.max(time(&m));
            }
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
        Some(entry_path(&self.root, &e.group, key, ext))
    }

    /// Keeps entry `key`'s `ext` file in `group`: written whole or not at all. An entry kept in
    /// another group moves (its old files go).
    pub fn put(&self, group: &str, key: &str, ext: &str, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(self.root.join(group))?;
        crate::files::write_atomic(&entry_path(&self.root, group, key, ext), bytes)?;
        let mut ix = self.index.locked();
        if ix.entries.get(key).is_some_and(|e| e.group != group) {
            self.drop_entry(&mut ix, key);
        }
        let clock = ix.tick();
        let e = ix.entries.entry(key.to_owned()).or_insert_with(|| Entry {
            group: group.to_owned(),
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
        self.trim(&mut ix);
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
        let path = entry_path(&self.root, &e.group, key, ext);
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
            let _ = std::fs::remove_file(entry_path(&self.root, &e.group, key, ext));
            ix.total = ix.total.saturating_sub(*len);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is kept is found again after a restart (from the listing alone); over the budget
    /// the least recently used goes first; a group goes together.
    #[test]
    fn kept_files_survive_a_restart_and_the_budget_takes_the_least_used() {
        let root = std::env::temp_dir().join(format!("np-disk-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let keys: Vec<String> = (0..4).map(|k| key(&[k])).collect();
        {
            let c = DiskCache::open(root.clone(), 1000, 100);
            for (k, g) in keys.iter().zip(["a", "a", "b", "b"]) {
                c.put(g, k, "rec", &[1; 300]).unwrap();
            }
            // Three fit; the fourth put trims to nine tenths: the least recently used goes.
            assert!(c.find(&keys[0], "rec").is_none(), "the oldest went");
            assert!(keys[1..].iter().all(|k| c.find(k, "rec").is_some()));
        }
        let c = DiskCache::open(root.clone(), 1000, 100);
        assert!(keys[1..].iter().all(|k| c.find(k, "rec").is_some()), "found again after a restart");
        assert_eq!(std::fs::read(c.find(&keys[2], "rec").unwrap()).unwrap(), vec![1; 300]);
        c.forget_group("b");
        assert!(c.find(&keys[2], "rec").is_none() && c.find(&keys[3], "rec").is_none());
        assert!(c.find(&keys[1], "rec").is_some(), "another group stays");
        let _ = std::fs::remove_dir_all(&root);
    }
}
