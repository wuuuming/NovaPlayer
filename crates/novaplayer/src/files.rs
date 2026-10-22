//! Saving files so a crash never leaves half of one (principles B3).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `bytes` to `file` whole or not at all: they go to a temporary file beside it, which
/// is then renamed over it. The folder is made if it is missing. On an error the old file is as
/// it was and the temporary one is gone.
///
/// The temporary name is this process's and this call's own, so two threads saving the same
/// file (or two files differing only in extension) never write into each other's.
pub fn write_atomic(file: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = temporary(file);
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, file));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Reads `file` and `decode`s it. A file that is there but does not decode (damaged, or written
/// by a newer version) is treated as missing: it is renamed aside to `<name>.bad-<seconds>`, so
/// the next save cannot overwrite what may still be recovered, and a line is logged.
pub fn read_kept<T>(file: &Path, decode: impl FnOnce(&str) -> Option<T>) -> Option<T> {
    let bytes = std::fs::read(file).ok()?;
    let decoded = std::str::from_utf8(&bytes).ok().and_then(decode);
    if decoded.is_none() {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let name = file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        let aside = file.with_file_name(format!("{name}.bad-{at}"));
        let moved = std::fs::rename(file, &aside);
        eprintln!("novaplayer: {} does not read; kept as {} ({moved:?})", file.display(), aside.display());
    }
    decoded
}

fn temporary(file: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let call = NEXT.fetch_add(1, Ordering::Relaxed);
    file.with_file_name(format!(".{name}.{}-{call}.tmp", std::process::id()))
}

/// A text is an http(s) web address (case-sensitive, as the lists and plugins write them).
pub fn is_web_address(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::write_atomic;

    #[test]
    fn replaces_the_file_whole_and_leaves_no_temporary() {
        let dir = std::env::temp_dir().join(format!("novaplayer-write-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("nested").join("settings.txt");
        write_atomic(&file, b"volume 0.5\n").expect("a new file in a new folder");
        write_atomic(&file, b"volume 0.7\n").expect("over the old one");
        assert_eq!(std::fs::read(&file).expect("saved"), b"volume 0.7\n");
        let left: Vec<_> = std::fs::read_dir(file.parent().expect("nested"))
            .expect("the folder")
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["settings.txt"]);
        // A file that cannot be replaced (a folder in its place) leaves nothing behind either.
        let blocked = dir.join("blocked");
        std::fs::create_dir_all(blocked.join("inside")).expect("a folder");
        assert!(write_atomic(&blocked, b"x").is_err());
        assert_eq!(std::fs::read_dir(&dir).expect("the folder").count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression (principles B3): a playlist index that did not decode was read as empty, and the next
    /// save wrote the empty index over it.
    #[test]
    fn a_file_that_does_not_decode_is_set_aside_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("novaplayer-read-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        let file = dir.join("index.txt");
        let number = |t: &str| t.trim().parse::<u32>().ok();

        assert_eq!(super::read_kept(&file, number), None, "missing");
        std::fs::write(&file, "7\n").expect("written");
        assert_eq!(super::read_kept(&file, number), Some(7));
        std::fs::write(&file, "version 9\n").expect("written");
        assert_eq!(super::read_kept(&file, number), None, "damaged");
        assert!(!file.exists());
        let aside: Vec<_> = std::fs::read_dir(&dir).expect("the folder").flatten().map(|e| e.path()).collect();
        assert_eq!(aside.len(), 1);
        assert!(aside[0].file_name().is_some_and(|n| n.to_string_lossy().starts_with("index.txt.bad-")));
        assert_eq!(std::fs::read_to_string(&aside[0]).expect("kept"), "version 9\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
