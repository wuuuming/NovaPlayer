//! The queue rows' words and thumbnails, kept on disk by the song's path (`disk_cache`): a hit
//! never stats or opens the song's file or address. A row is a record in its song's folder's
//! group (forgotten together when a cover is chosen for the folder) naming its thumbnail, kept
//! once by its content (an album's songs share one). Explicit rescans and cover edits forget
//! rows; a row expires after seven days.
use crate::sync::Locked;
use crate::{
    library::disk_cache::{self, DiskCache},
    library::store::{self, List},
    library::{self, Song, Tags},
};
use novaui::ImageHandle;
use std::{
    fs,
    io::{self, Read},
    path::Path,
    sync::{Mutex, OnceLock},
};

pub type Row = (Option<Tags>, Option<ImageHandle>);
#[cfg(any(test, feature = "test-support"))]
#[derive(Default, Clone, Copy, Debug)]
pub struct Stats {
    pub hits: u64,
    pub source_loads: u64,
    pub skipped: u64,
    pub write_errors: u64,
}

/// A row record's first line (what follows: the song as a one-song list).
const HEAD: &str = "NP-ROW-2\n";
const ROW: &str = "row";
const THUMB: &str = "rgba";
/// A row's thumbnail is this many pixels square.
const THUMB_SIDE: i32 = 96;
const THUMB_BYTES: u64 = 8 + THUMB_SIDE as u64 * THUMB_SIDE as u64 * 4;

struct Cache {
    disk: DiskCache,
    /// Moves with every row forgotten: a load that began before is not kept.
    version: u64,
    #[cfg(any(test, feature = "test-support"))]
    stats: Stats,
}

fn read_limit(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::other("row cache record too large"));
    }
    Ok(bytes)
}

/// The group a song's row is kept in: its folder's.
fn group(source: &Path) -> String {
    folder_group(source.parent().unwrap_or(Path::new("")))
}

/// The group a folder's songs' rows are kept in.
fn folder_group(folder: &Path) -> String {
    disk_cache::path_key(folder)
}

impl Cache {
    fn open(root: std::path::PathBuf, limit: u64, entries: usize) -> Self {
        Cache {
            disk: DiskCache::open(root, limit, entries),
            version: 0,
            #[cfg(any(test, feature = "test-support"))]
            stats: Stats::default(),
        }
    }

    fn get(&mut self, source: &Path) -> Option<Row> {
        let key = disk_cache::path_key(source);
        let path = self.disk.find(&key, ROW)?;
        let age = fs::metadata(&path).ok()?.modified().ok()?.elapsed().unwrap_or_default();
        if age.as_secs() > 7 * 24 * 3600 {
            self.disk.forget(&key);
            return None;
        }
        let bytes = read_limit(&path, 128 * 1024).ok()?;
        let text = std::str::from_utf8(&bytes).ok()?.strip_prefix(HEAD)?;
        let mut list = store::decode_list(text)?;
        if list.songs.len() != 1 {
            return None;
        }
        let song = std::sync::Arc::unwrap_or_clone(list.songs_mut().pop()?);
        if song.path != source {
            return None;
        }
        let thumb = match self.disk.blob_of(&key, THUMB) {
            None => None,
            Some((blob, bpath)) => {
                let raw = read_limit(&bpath, THUMB_BYTES).ok()?;
                if raw.len() as u64 != THUMB_BYTES || disk_cache::key(&raw) != blob {
                    return None;
                }
                if u32::from_le_bytes(raw[..4].try_into().ok()?) != THUMB_SIDE as u32
                    || u32::from_le_bytes(raw[4..8].try_into().ok()?) != THUMB_SIDE as u32
                {
                    return None;
                }
                Some(ImageHandle::rgba8(THUMB_SIDE, THUMB_SIDE, raw[8..].to_vec()))
            }
        };
        #[cfg(any(test, feature = "test-support"))]
        {
            self.stats.hits += 1;
        }
        Some((song.probed.then_some(song.tags), thumb))
    }

    fn put(&mut self, source: &Path, value: &Row) -> io::Result<()> {
        // An unreadable/offline file isn't a permanent "no artwork" result.
        if value.0.is_none() && value.1.is_none() {
            return Ok(());
        }
        let blob = match &value.1 {
            Some(thumb) => {
                let (side, pixels) = (thumb.width() as u32, thumb.pixels());
                if thumb.size() != (THUMB_SIDE, THUMB_SIDE) {
                    return Err(io::Error::other("invalid thumbnail representation"));
                }
                let mut bytes = Vec::with_capacity(8 + pixels.len());
                bytes.extend_from_slice(&side.to_le_bytes());
                bytes.extend_from_slice(&side.to_le_bytes());
                bytes.extend_from_slice(pixels);
                let key = disk_cache::key(&bytes);
                self.disk.put_blob(&key, THUMB, &bytes)?;
                Some(key)
            }
            None => None,
        };
        let mut song = Song::new(source.to_path_buf());
        song.probed = value.0.is_some();
        song.tags = value.0.clone().unwrap_or_default();
        let list =
            List { songs: std::sync::Arc::new(vec![std::sync::Arc::new(song)]), order: vec![0], ..Default::default() };
        let record = format!("{HEAD}{}", store::encode_list(&list));
        if record.len() > 128 * 1024 {
            return Err(io::Error::other("metadata exceeds cache record budget"));
        }
        self.disk.put(&group(source), &disk_cache::path_key(source), ROW, record.as_bytes(), blob.as_deref())
    }

    /// A row forgotten moves the version, so a load begun before is not kept.
    fn invalidate(&mut self, source: &Path) {
        self.version += 1;
        self.disk.forget(&disk_cache::path_key(source));
    }

    /// The rows of a folder forgotten, as `invalidate`'s.
    fn invalidate_folder(&mut self, folder: &Path) {
        self.version += 1;
        self.disk.forget_group(&folder_group(folder));
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let root = crate::data_root().unwrap_or_else(std::env::temp_dir).join(crate::DATA_DIR).join("row-cache-v2");
        Mutex::new(Cache::open(root, 128 * 1024 * 1024, 8192))
    })
}
pub fn load(path: &Path) -> Row {
    // An online song's row: its cached face, else its plugin asked now (the list's words stay;
    // only the thumbnail is new). Never through the file probe below.
    if crate::plugins::client::parse(path).is_some() {
        // The prefetched thumbnail first (no plugin call), else the face.
        if let Some(t) = crate::plugins::client::kept_thumb(path).and_then(|b| library::thumb(&b, THUMB_SIDE as u32)) {
            return (None, Some(t));
        }
        let song = crate::library::Song::new(path.to_path_buf());
        let thumb = crate::plugins::client::track_info(&song, false)
            .and_then(|f| f.cover)
            .and_then(|b| library::thumb(&b, THUMB_SIDE as u32));
        return (None, thumb);
    }
    let version = {
        let mut cache = cache().locked();
        if let Some(value) = cache.get(path) {
            return value;
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            cache.stats.source_loads += 1;
        }
        cache.version
    };
    let mut t = crate::playback::engine::probe(path);
    let bytes = crate::library::covers::resolve(path, t.as_mut().and_then(|t| t.cover.take()));
    let thumb = bytes.as_deref().and_then(|b| library::thumb(b, THUMB_SIDE as u32));
    let tags = t.as_ref().map(Tags::from);
    let value = (tags, thumb);
    let mut cache = cache().locked();
    if cache.version == version {
        let _stored = cache.put(path, &value);
        #[cfg(any(test, feature = "test-support"))]
        if _stored.is_err() {
            cache.stats.write_errors += 1;
        }
    }
    value
}
pub fn invalidate(path: &Path) {
    cache().locked().invalidate(path);
}
/// The rows of the songs in `folder` (a cover chosen or restored for it): forgotten with their
/// group, no row read.
pub fn invalidate_folder(folder: &Path) {
    cache().locked().invalidate_folder(folder);
}
#[cfg(any(test, feature = "test-support"))]
pub fn skipped() {
    cache().locked().stats.skipped += 1;
}
#[cfg(all(feature = "test-support", windows))]
pub fn stats() -> Stats {
    cache().locked().stats
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn warm_rows_need_no_source_and_shared_blobs_survive_restart_and_budget() {
        let root = std::env::temp_dir().join(format!("np-row-cache-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut cache = Cache::open(root.clone(), 2 * 1024 * 1024, 2048);
        let value = (
            Some(Tags { title: Some("cached song".into()), ..Default::default() }),
            Some(ImageHandle::rgba8(96, 96, vec![127; 96 * 96 * 4])),
        );
        // Nonexistent network source paths: a get that checks source metadata cannot pass.
        let folder = Path::new(if cfg!(windows) { r"Z:\offline-cache-fixture" } else { "/offline-cache-fixture" });
        let sources: Vec<_> = (0..1000).map(|i| folder.join(format!("{i}.flac"))).collect();
        for path in &sources {
            cache.put(path, &value).unwrap();
        }
        assert_eq!(cache.disk.size().2, 1, "one shared thumbnail");
        drop(cache);
        let mut cache = Cache::open(root.clone(), 2 * 1024 * 1024, 2048);
        for _ in 0..3 {
            for path in sources.iter().rev() {
                let row = cache.get(path).expect("warm scrolling must not need its source file");
                assert_eq!(row.0.unwrap().title.as_deref(), Some("cached song"));
                assert_eq!(row.1.unwrap().pixels(), value.1.as_ref().unwrap().pixels());
            }
        }
        assert_eq!(cache.stats.hits, 3000);
        let (_, blob) = cache.disk.blob_of(&disk_cache::path_key(&sources[1]), THUMB).unwrap();
        fs::write(&blob, b"damaged").unwrap();
        assert!(cache.get(&sources[1]).is_none());
        cache.put(&sources[1], &value).unwrap();
        assert!(cache.get(&sources[1]).is_some(), "refetch must repair a damaged blob, not miss forever");
        cache.invalidate(&sources[0]);
        assert!(cache.get(&sources[0]).is_none());
        assert!(cache.get(&sources[1]).is_some());
        // The folder's rows go together; the thumbnail they shared with it.
        invalidate_folder_in(&mut cache, folder);
        assert!(cache.get(&sources[2]).is_none());
        assert_eq!(cache.disk.size(), (0, 0, 0));
        for path in &sources[..20] {
            cache.put(path, &value).unwrap();
        }
        drop(cache);
        let cache = Cache::open(root.clone(), 40 * 1024, 8);
        let (bytes, rows, _) = cache.disk.size();
        assert!(bytes <= 40 * 1024 && rows <= 8, "{bytes} bytes, {rows} rows");
        fs::remove_dir_all(root).unwrap();
    }

    fn invalidate_folder_in(cache: &mut Cache, folder: &Path) {
        cache.invalidate_folder(folder);
    }
}
