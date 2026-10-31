//! The library index: what is known of local songs without opening their files again. Kept on
//! disk (`disk_cache`, `index-v1` in the data folder), it holds three kinds of record:
//!
//! - a song's (by its path, in its folder's group): its file's size and time when it was read,
//!   its words, the picture it shows and whether it has lyrics;
//! - an album's (by its folder and its album's name, in the folder's group): the picture of the
//!   first of its songs read, and which file that was. The album's other songs show it before
//!   their own files are read: an album's files carry the same picture, often thousands of pixels
//!   in every one of them. It holds while that file's own record does (a re-tagged album is read
//!   again); a song whose own picture differs shows its own once its file is read (when it is
//!   played). One reader an album at a time: the rows and reading ahead do not both read it;
//! - each picture once, by its content: its row thumbnail (`THUMB_SIDE` square, raw RGBA) and its
//!   face (at most `FACE_SIDE`: the picture as it is when it is that small or carries a gain map,
//!   else a JPEG of it at that size), which a song's face decodes instead of the picture in the
//!   song's file.
//!
//! A record holds while its file's size and time are what its folder's listing says
//! (`covers::stamp`: one listing a folder a run); nothing expires by age. A song whose file
//! changed is read again before anything stands for it. A cover chosen or given
//! back for a folder forgets the folder's records (its pictures stay: they are kept by content).
//!
//! Read ahead (`ahead`, `warm`), on a thread of its own that holds back while a song opens: the
//! songs either side of the one that plays (record, face, thumbnail), then one song of each album
//! of the shown list with no picture that holds (record and thumbnail: a face is made only for a
//! song about to play).
use crate::library::{self, covers, disk_cache, disk_cache::DiskCache, store, Cover, Lyrics, Song, Tags};
use crate::player_core::MediaTags;
use crate::sync::Locked;
use novaui::ImageHandle;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A queue row's words (when its file was read) and thumbnail.
pub type Row = (Option<Tags>, Option<ImageHandle>);

#[cfg(any(test, feature = "test-support"))]
#[derive(Default, Clone, Copy, Debug)]
pub struct Stats {
    /// Rows answered from a record (the song's or its album's): no file read.
    pub hits: u64,
    /// Songs' files read.
    pub source_loads: u64,
    pub skipped: u64,
    pub write_errors: u64,
}

const SONG: &str = "song";
const ALBUM: &str = "album";
const THUMB: &str = "thumb";
const FACE: &str = "face";
/// The group pictures are kept in (records are in their folders').
const PICTURES: &str = "pictures";
const HEAD: &str = "NP-SONG-1\n";
/// A row's thumbnail is this many pixels square.
pub const THUMB_SIDE: i32 = 96;
const THUMB_BYTES: u64 = 8 + THUMB_SIDE as u64 * THUMB_SIDE as u64 * 4;
/// A face is at most this many pixels a side (`library::decode_cover`'s).
pub const FACE_SIDE: u32 = 1040;
const FACE_LIMIT: u64 = 32 << 20;
const RECORD_LIMIT: u64 = 128 * 1024;
/// Local songs' and albums' records and every picture, online songs' too (`plugins::client`
/// keeps their words and names their pictures here).
const BUDGET: u64 = 384 << 20;
const ENTRIES: usize = 64 * 1024;
/// How long reading ahead holds back when a song begins to open.
const HOLD: Duration = Duration::from_secs(2);
/// Threads reading ahead: one. A cold file on a share is bound by the share's disk (about half a
/// second an album's file on a network share); more readers only make that disk seek between them, and
/// take its time from the song that plays from it.
const READERS: usize = 1;

/// What a read of a song's file found.
#[derive(Clone, Debug, Default, PartialEq)]
struct Record {
    /// Its file's size and time when it was read.
    stamp: (u64, u64),
    tags: Tags,
    /// The picture it shows (`covers::resolve`'s, by content).
    picture: Option<String>,
    lyrics: bool,
}

/// A song's record, as far as its file goes.
enum Kept {
    Good(Box<Record>),
    /// Its file changed since it was read: it is read again (nothing stands for it).
    Stale,
    None,
}

/// What this run knows of a song (the UI's questions are answered from memory).
#[derive(Clone, Debug, Default)]
struct Known {
    lyrics: bool,
}

struct Index {
    disk: DiskCache,
    /// The albums being read (`album_turn`): another reader of one waits for its picture.
    reading: (Mutex<std::collections::HashSet<String>>, Condvar),
    /// Moves with every record forgotten: a read begun before is not kept.
    version: AtomicU64,
    known: Mutex<HashMap<PathBuf, Known>>,
    #[cfg(any(test, feature = "test-support"))]
    stats: Mutex<Stats>,
}

fn read_limit(path: &Path, max: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(max + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

/// The group a song's records are kept in: its folder's.
fn group(path: &Path) -> String {
    disk_cache::path_key(path.parent().unwrap_or(Path::new("")))
}

/// The key of a local song's album: its folder and its album's name. None for an online song
/// (its plugin keeps each song's picture) or a song with no album.
fn album_key(path: &Path, album: &str) -> Option<String> {
    let album = album.trim();
    if album.is_empty() || crate::plugins::client::parse(path).is_some() {
        return None;
    }
    // A folder's name in any case on Windows (its file system's), as it is elsewhere.
    let folder = path.parent()?.to_string_lossy();
    let folder = if cfg!(windows) { folder.to_lowercase() } else { folder.into_owned() };
    Some(disk_cache::key(format!("album\n{folder}\n{}", album.to_lowercase()).as_bytes()))
}

fn encode(path: &Path, r: &Record) -> String {
    let mut song = Song::new(path.to_path_buf());
    song.probed = true;
    song.tags = r.tags.clone();
    let list = store::List {
        songs: std::sync::Arc::new(vec![std::sync::Arc::new(song)]),
        order: vec![0],
        ..Default::default()
    };
    format!(
        "{HEAD}stamp {} {}\npicture {}\nlyrics {}\n{}",
        r.stamp.0,
        r.stamp.1,
        r.picture.as_deref().unwrap_or("-"),
        u8::from(r.lyrics),
        store::encode_list(&list)
    )
}

fn decode(bytes: &[u8], path: &Path) -> Option<Record> {
    let text = std::str::from_utf8(bytes).ok()?.strip_prefix(HEAD)?;
    let mut lines = text.splitn(4, '\n');
    let mut stamp = lines.next()?.strip_prefix("stamp ")?.split(' ');
    let stamp = (stamp.next()?.parse().ok()?, stamp.next()?.parse().ok()?);
    let picture = match lines.next()?.strip_prefix("picture ")? {
        "-" => None,
        id if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) => Some(id.to_owned()),
        _ => return None,
    };
    let lyrics = lines.next()?.strip_prefix("lyrics ")? == "1";
    let mut list = store::decode_list(lines.next()?)?;
    let song = std::sync::Arc::unwrap_or_clone(list.songs_mut().pop()?);
    (song.path == path).then_some(Record { stamp, tags: song.tags, picture, lyrics })
}

/// A picture's face as kept: the picture itself when it is at most `FACE_SIDE` a side (as it
/// came) or carries a gain map (a re-encoding would lose what an HDR display shows of it); else
/// a JPEG of it at that size (a PNG when it has transparency). One rule for every song of an
/// album, so each one's face is the same pixels and nothing changes between them.
fn render_face(source: &[u8]) -> Option<Vec<u8>> {
    if novaui::image::has_gain_map(source) {
        return Some(source.to_vec());
    }
    let (w, h) =
        image::ImageReader::new(std::io::Cursor::new(source)).with_guessed_format().ok()?.into_dimensions().ok()?;
    if w.max(h) <= FACE_SIDE {
        return Some(source.to_vec());
    }
    let img = novaui::image::load(source)?.resize(FACE_SIDE, FACE_SIDE, image::imageops::FilterType::Triangle);
    let mut out = Vec::new();
    if img.color().has_alpha() {
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&img.into_rgb8()).ok()?;
    }
    Some(out)
}

/// What one read of a song's file gave.
struct FileRead {
    tags: Option<MediaTags>,
    /// Its picture: by content, and the picture's bytes.
    picture: Option<(String, Vec<u8>)>,
    /// Its lyrics, when they were asked for.
    lyrics: Option<Lyrics>,
}

impl Index {
    fn open(root: PathBuf) -> Self {
        Index {
            disk: DiskCache::open(root, BUDGET, ENTRIES),
            reading: Default::default(),
            version: AtomicU64::new(0),
            known: Mutex::new(HashMap::new()),
            #[cfg(any(test, feature = "test-support"))]
            stats: Mutex::new(Stats::default()),
        }
    }

    /// Song `path`'s record, while its file is as its folder lists it (an unreachable folder
    /// keeps what was read).
    fn kept(&self, path: &Path) -> Kept {
        let key = disk_cache::path_key(path);
        let Some(r) =
            self.disk.find(&key, SONG).and_then(|f| read_limit(&f, RECORD_LIMIT)).and_then(|b| decode(&b, path))
        else {
            return Kept::None;
        };
        if covers::stamp(path).is_some_and(|s| s != r.stamp) {
            self.forget(path);
            return Kept::Stale;
        }
        self.known.locked().insert(path.to_owned(), Known { lyrics: r.lyrics });
        Kept::Good(Box::new(r))
    }

    /// Album `key`'s picture, and the name of the file (in the album's folder) it came from.
    fn album_entry(&self, key: &str) -> Option<(String, String)> {
        let file = self.disk.find(key, ALBUM)?;
        let text = String::from_utf8(read_limit(&file, 4096)?).ok()?;
        let (id, from) = text.split_once('\n')?;
        (id.len() == 32 && !from.is_empty() && !from.contains(['/', '\\'])).then(|| (id.to_owned(), from.to_owned()))
    }

    /// Album `key`'s picture, while the file it came from (`folder`'s) still shows it as read:
    /// re-tagged, the album is read again.
    fn album(&self, key: &str, folder: &Path) -> Option<String> {
        let (id, from) = self.album_entry(key)?;
        match self.kept(&folder.join(from)) {
            Kept::Good(r) if r.picture.as_deref() == Some(&id) => Some(id),
            _ => None,
        }
    }

    /// Album `key`'s turn to be read: while another reader has it, this one waits (at most a
    /// few seconds: a share that hangs does not hold every row of its album).
    fn album_turn(&self, key: &str) -> AlbumTurn<'_> {
        let (m, cv) = &self.reading;
        let end = Instant::now() + Duration::from_secs(5);
        let mut set = m.locked();
        while set.contains(key) && Instant::now() < end {
            set = cv.wait_timeout(set, Duration::from_millis(100)).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
        }
        set.insert(key.to_owned());
        AlbumTurn { ix: self, key: key.to_owned() }
    }

    fn thumb(&self, id: &str) -> Option<ImageHandle> {
        let raw = read_limit(&self.disk.find(id, THUMB)?, THUMB_BYTES)?;
        let side = THUMB_SIDE as u32;
        if raw.len() as u64 != THUMB_BYTES
            || u32::from_le_bytes(raw[..4].try_into().ok()?) != side
            || u32::from_le_bytes(raw[4..8].try_into().ok()?) != side
        {
            return None;
        }
        Some(ImageHandle::rgba8(THUMB_SIDE, THUMB_SIDE, raw[8..].to_vec()))
    }

    fn face(&self, id: &str) -> Option<Vec<u8>> {
        read_limit(&self.disk.find(id, FACE)?, FACE_LIMIT).filter(|b| !b.is_empty())
    }

    fn put(&self, group: &str, key: &str, ext: &str, bytes: &[u8]) {
        let _written = self.disk.put(group, key, ext, bytes);
        #[cfg(any(test, feature = "test-support"))]
        if _written.is_err() {
            self.stats.locked().write_errors += 1;
        }
    }

    /// Picture `id`'s thumbnail: kept, else made from `source` (or from its face) and kept.
    fn thumb_or_make(&self, id: &str, source: Option<&[u8]>) -> Option<ImageHandle> {
        if let Some(t) = self.thumb(id) {
            return Some(t);
        }
        let face;
        let bytes = match source {
            Some(b) => b,
            None => {
                face = self.face(id)?;
                &face
            }
        };
        let t = library::thumb(bytes, THUMB_SIDE as u32)?;
        if t.size() == (THUMB_SIDE, THUMB_SIDE) {
            let mut raw = Vec::with_capacity(THUMB_BYTES as usize);
            raw.extend_from_slice(&(THUMB_SIDE as u32).to_le_bytes());
            raw.extend_from_slice(&(THUMB_SIDE as u32).to_le_bytes());
            raw.extend_from_slice(t.pixels());
            self.put(PICTURES, id, THUMB, &raw);
        }
        Some(t)
    }

    /// Picture `id`'s face: kept, else made from `source` (`render_face`) and kept.
    fn face_or_make(&self, id: &str, source: &[u8]) -> Option<Vec<u8>> {
        if let Some(f) = self.face(id) {
            return Some(f);
        }
        let f = render_face(source)?;
        self.put(PICTURES, id, FACE, &f);
        Some(f)
    }

    /// Reads song `path`'s file: its words, its picture (`covers::resolve`), whether it has
    /// lyrics (its lyrics too, with `lyrics`), and keeps its record, and its album's when the
    /// album has none. A file that does not read keeps nothing (a share that is away is not
    /// taken for a song with nothing).
    fn read(&self, path: &Path, lyrics: bool) -> FileRead {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.stats.locked().source_loads += 1;
        }
        let version = self.version.load(Ordering::Acquire);
        let stamp = covers::stamp(path).or_else(|| std::fs::metadata(path).ok().map(|m| covers::stamp_of(&m)));
        let mut tags = crate::playback::engine::probe(path);
        let bytes = covers::resolve(path, tags.as_mut().and_then(|t| t.cover.take()));
        let embedded = tags.as_ref().and_then(|t| t.lyrics.as_deref()).and_then(Lyrics::parse);
        let (lyrics, has) = if lyrics {
            let l = library::choose_lyrics(embedded, || library::sidecar(path))
                .map(|l| l.with_voices(library::duet_sidecar(path).as_ref()));
            let has = l.is_some();
            (l, has)
        } else {
            let has = embedded.is_some() || covers::lyrics_beside(path);
            (None, has)
        };
        let picture = bytes.map(|b| (disk_cache::key(&b), b));
        if let (Some(t), Some(stamp)) = (&tags, stamp) {
            if self.version.load(Ordering::Acquire) == version {
                let r =
                    Record { stamp, tags: Tags::from(t), picture: picture.as_ref().map(|p| p.0.clone()), lyrics: has };
                let group = group(path);
                self.put(&group, &disk_cache::path_key(path), SONG, encode(path, &r).as_bytes());
                // The album's picture: this song's when the album has none that holds (none
                // yet, or the file it came from changed), or when it came from this song.
                if let (Some((id, _)), Some(key), Some(name), Some(folder)) = (
                    &picture,
                    t.album.as_deref().and_then(|a| album_key(path, a)),
                    path.file_name().map(|n| n.to_string_lossy().into_owned()),
                    path.parent(),
                ) {
                    let mine = match self.album_entry(&key) {
                        Some((was, from)) if from.eq_ignore_ascii_case(&name) => was != *id,
                        Some(_) => self.album(&key, folder).is_none(),
                        None => true,
                    };
                    if mine {
                        self.put(&group, &key, ALBUM, format!("{id}\n{name}").as_bytes());
                    }
                }
                self.known.locked().insert(path.to_owned(), Known { lyrics: has });
            }
        }
        FileRead { tags, picture, lyrics }
    }

    fn row(&self, path: &Path, album: Option<&str>) -> Row {
        match self.kept(path) {
            Kept::Good(r) => {
                let r = *r;
                let thumb = r.picture.as_deref().and_then(|id| self.thumb_or_make(id, None));
                // A picture named but no longer kept (the budget let it go): read again.
                if r.picture.is_none() || thumb.is_some() {
                    self.hit();
                    return (Some(r.tags), thumb);
                }
            }
            Kept::Stale => {}
            // Its album's picture, its own file unread (`album_key`); another reader of the
            // album is waited for (its picture is this one's).
            Kept::None => {
                if let (Some(key), Some(folder)) = (album.and_then(|a| album_key(path, a)), path.parent()) {
                    let turn = self.album_turn(&key);
                    if let Some(t) = self.album(&key, folder).and_then(|id| self.thumb_or_make(&id, None)) {
                        self.hit();
                        return (None, Some(t));
                    }
                    let read = self.read(path, false);
                    drop(turn);
                    let thumb = read.picture.as_ref().and_then(|(id, b)| self.thumb_or_make(id, Some(b)));
                    return (read.tags.as_ref().map(Tags::from), thumb);
                }
            }
        }
        let read = self.read(path, false);
        let thumb = read.picture.as_ref().and_then(|(id, b)| self.thumb_or_make(id, Some(b)));
        (read.tags.as_ref().map(Tags::from), thumb)
    }

    fn opened(&self, path: &Path) -> Opened {
        let read = self.read(path, true);
        let face = read.picture.as_ref().and_then(|(id, b)| Some((id, self.face_or_make(id, b)?)));
        // Its rows' thumbnail from its face, while it is at hand.
        if let Some((id, f)) = &face {
            let _ = self.thumb_or_make(id, Some(f));
        }
        let cover = face.and_then(|(_, f)| library::decode_cover(&f, FACE_SIDE));
        Opened { tags: read.tags, cover, lyrics: read.lyrics, editable: path.exists() }
    }

    fn known_cover(&self, path: &Path, album: Option<&str>) -> Option<Cover> {
        let id = match self.kept(path) {
            Kept::Good(r) => r.picture?,
            Kept::Stale => return None,
            Kept::None => self.album(&album_key(path, album?)?, path.parent()?)?,
        };
        library::decode_cover(&self.face(&id)?, FACE_SIDE)
    }

    /// Song `path` read ahead: its record and its picture's thumbnail, and with `face` its face
    /// (a song about to play; an album's rows need only the thumbnail). Nothing to do when what
    /// is asked is known without it (its record's picture, else its album's: `known_cover`).
    fn prefetch(&self, path: &Path, album: Option<&str>, face: bool) {
        if crate::plugins::client::parse(path).is_some() {
            return;
        }
        let known = match self.kept(path) {
            Kept::Good(r) => r.picture.map(Some),
            Kept::Stale => Some(None),
            Kept::None => {
                album.and_then(|a| album_key(path, a)).zip(path.parent()).and_then(|(k, f)| self.album(&k, f)).map(Some)
            }
        };
        let ext = if face { FACE } else { THUMB };
        match known {
            // No picture, read already.
            None if matches!(self.kept(path), Kept::Good(_)) => return,
            Some(Some(id)) if self.disk.find(&id, ext).is_some() => return,
            _ => {}
        }
        let read = self.read(path, false);
        if let Some((id, b)) = &read.picture {
            if face {
                // The thumbnail from the face (a 1040 px picture decodes in a fraction of the
                // time of a 3000 px one).
                let f = self.face_or_make(id, b);
                let _ = self.thumb_or_make(id, Some(f.as_deref().unwrap_or(b)));
            } else {
                let _ = self.thumb_or_make(id, Some(b));
            }
        }
    }

    fn hit(&self) {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.stats.locked().hits += 1;
        }
    }

    /// Album `album`'s picture read ahead through `path` (one of its songs), unless the album
    /// has one that holds; waits for another reader of it.
    fn prefetch_album(&self, path: &Path, album: &str) {
        let (Some(key), Some(folder)) = (album_key(path, album), path.parent()) else { return };
        let _turn = self.album_turn(&key);
        if self.album(&key, folder).is_none() {
            self.prefetch(path, None, false);
        }
    }

    fn forget(&self, path: &Path) {
        self.version.fetch_add(1, Ordering::AcqRel);
        self.disk.forget(&disk_cache::path_key(path));
        self.known.locked().remove(path);
    }

    fn forget_folder(&self, folder: &Path) {
        self.version.fetch_add(1, Ordering::AcqRel);
        // The folder's songs' records and its albums', together.
        self.disk.forget_group(&disk_cache::path_key(folder));
        self.known.locked().retain(|p, _| p.parent() != Some(folder));
    }
}

/// An album's turn to be read (`Index::album_turn`): gone, the next reader goes on.
struct AlbumTurn<'a> {
    ix: &'a Index,
    key: String,
}

impl Drop for AlbumTurn<'_> {
    fn drop(&mut self) {
        let (m, cv) = &self.ix.reading;
        m.locked().remove(&self.key);
        cv.notify_all();
    }
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let data = crate::data_root().unwrap_or_else(std::env::temp_dir).join(crate::DATA_DIR);
        // The rows' cache this index took over: what it kept is read again, once.
        let _ = std::fs::remove_dir_all(data.join("row-cache-v2"));
        Index::open(data.join("index-v1"))
    })
}

/// A song's queue row (a worker's): from its record, else its album's picture (`album`: the
/// album its row's words name, when they are read already), else its file read. An online
/// song's: its kept thumbnail (prefetched, or its face's), else its face asked of its plugin.
pub fn row(path: &Path, album: Option<&str>) -> Row {
    if crate::plugins::client::parse(path).is_some() {
        if let Some(t) = crate::plugins::client::kept_thumb(path) {
            return (None, Some(t));
        }
        let song = Song::new(path.to_path_buf());
        let thumb = crate::plugins::client::track_info(&song, false)
            .and_then(|f| f.cover)
            .and_then(|b| library::thumb(&b, THUMB_SIDE as u32));
        return (None, thumb);
    }
    index().row(path, album)
}

// ---- pictures for others (online songs') --------------------------------------------------

/// Keeps `bytes` as a picture's face as they are (an online cover, made small already): its
/// key, by its content. A song of the same album asking again finds it kept.
pub fn keep_face(bytes: &[u8]) -> Option<String> {
    let ix = index();
    let id = disk_cache::key(bytes);
    if ix.disk.find(&id, FACE).is_none() {
        ix.disk.put(PICTURES, &id, FACE, bytes).ok()?;
    }
    Some(id)
}

/// Picture `id`'s face, while it is kept.
pub fn face_of(id: &str) -> Option<Vec<u8>> {
    index().face(id)
}

/// Keeps a small picture (an online row's thumbnail as fetched) as a thumbnail: its key.
pub fn keep_thumb(bytes: &[u8]) -> Option<String> {
    let id = disk_cache::key(bytes);
    index().thumb_or_make(&id, Some(bytes)).map(|_| id)
}

/// Picture `id`'s thumbnail: kept, else made from its face (and kept).
pub fn thumb_of(id: &str) -> Option<ImageHandle> {
    index().thumb_or_make(id, None)
}

/// Picture `id` forgotten (tests: the store's budget let it go).
#[cfg(test)]
pub fn forget_picture_for_test(id: &str) {
    index().disk.forget(id);
}

/// Whether `id` names a picture of the store (a name read from a file).
pub fn is_picture_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A local song as it opens to play (a worker's): its file read for its words, picture and
/// lyrics; its face from the picture's kept face.
pub struct Opened {
    pub tags: Option<MediaTags>,
    pub cover: Option<Cover>,
    pub lyrics: Option<Lyrics>,
    /// A cover can be chosen for it (a file of this computer's or a share's).
    pub editable: bool,
}

pub fn open(path: &Path) -> Opened {
    index().opened(path)
}

/// The face a local song is known to show (its record's picture, else its album's), with no
/// read of its file: what stands while the file is read (a worker's).
pub fn known_cover(path: &Path, album: Option<&str>) -> Option<Cover> {
    index().known_cover(path, album)
}

/// Whether a song has lyrics, as far as this run knows (a read of its file, or its record):
/// from memory, for the UI.
pub fn lyrics_known(path: &Path) -> Option<bool> {
    index().known.locked().get(path).map(|k| k.lyrics)
}

/// The records of `path` forgotten (its file changed: a rescan).
pub fn invalidate(path: &Path) {
    index().forget(path);
    if let Some(folder) = path.parent() {
        covers::forget_listing(folder);
    }
}

/// The records of `folder`'s songs and albums forgotten (a cover chosen or given back for it).
pub fn invalidate_folder(folder: &Path) {
    index().forget_folder(folder);
}

// ---- reading ahead -------------------------------------------------------------------------

#[derive(Default)]
struct Ahead {
    /// The songs either side of the one that plays (and their albums): first.
    near: VecDeque<(PathBuf, Option<String>)>,
    /// One song of each album of the shown list (`warm`).
    albums: VecDeque<(PathBuf, String)>,
    hold_until: Option<Instant>,
    started: bool,
}

fn ahead_state() -> &'static (Mutex<Ahead>, Condvar) {
    static AHEAD: OnceLock<(Mutex<Ahead>, Condvar)> = OnceLock::new();
    AHEAD.get_or_init(Default::default)
}

/// Starts the reading-ahead threads, once.
fn start(a: &mut Ahead) {
    if std::mem::replace(&mut a.started, true) {
        return;
    }
    for k in 0..READERS {
        if spawn_reader(k).is_err() && k == 0 {
            a.started = false;
        }
    }
}

fn spawn_reader(k: usize) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new().name(format!("novaplayer-index-{k}")).spawn(|| {
        lower_priority();
        let (m, cv) = ahead_state();
        loop {
            let job = {
                let mut a = m.locked();
                loop {
                    let now = Instant::now();
                    match a.hold_until {
                        Some(t) if t > now => {
                            a = cv.wait_timeout(a, t - now).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
                            continue;
                        }
                        _ => {}
                    }
                    if let Some((p, album)) = a.near.pop_front() {
                        break (p, album, true);
                    }
                    if let Some((p, album)) = a.albums.pop_front() {
                        break (p, Some(album), false);
                    }
                    a = cv.wait(a).unwrap_or_else(|e| e.into_inner());
                }
            };
            let ix = index();
            match job {
                (path, album, true) => ix.prefetch(&path, album.as_deref(), true),
                // An album with a picture that holds is skipped with no read.
                (path, Some(album), false) => ix.prefetch_album(&path, &album),
                (_, None, false) => {}
            }
        }
    })
}

#[cfg(windows)]
fn lower_priority() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
    // SAFETY: the pseudo-handle of this thread; a failure leaves its priority as it was.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
}

#[cfg(not(windows))]
fn lower_priority() {}

/// A song begins to open: reading ahead holds back for a while (the song's own file is read
/// first), then reads `near` (the songs either side of it), before any album of the list.
pub fn ahead(near: Vec<(PathBuf, Option<String>)>) {
    let (m, cv) = ahead_state();
    let mut a = m.locked();
    start(&mut a);
    a.near = near.into_iter().filter(|(p, _)| crate::plugins::client::parse(p).is_none()).collect();
    a.hold_until = Some(Instant::now() + HOLD);
    cv.notify_all();
}

/// The shown list's songs (path, album) to read one of each album of, those with no record
/// (in the list's order, from `first`): replaces what was asked before.
pub fn warm(songs: Vec<(PathBuf, String)>) {
    let mut seen = std::collections::HashSet::new();
    let albums: VecDeque<(PathBuf, String)> =
        songs.into_iter().filter(|(p, a)| album_key(p, a).is_some_and(|k| seen.insert(k))).collect();
    let (m, cv) = ahead_state();
    let mut a = m.locked();
    start(&mut a);
    a.albums = albums;
    cv.notify_all();
}

// ---- tests' --------------------------------------------------------------------------------

#[cfg(any(test, feature = "test-support"))]
pub fn skipped() {
    index().stats.locked().skipped += 1;
}

#[cfg(feature = "test-support")]
pub fn stats() -> Stats {
    *index().stats.locked()
}

/// A new run, as far as the index knows: folders are listed again, nothing is known in memory
/// (what is kept on disk stays).
#[cfg(feature = "test-support")]
pub fn new_run() {
    covers::forget_listings();
    index().known.locked().clear();
}

/// Waits until reading ahead has nothing left (tests).
#[cfg(feature = "test-support")]
pub fn ahead_idle(timeout: Duration) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        {
            let a = ahead_state().0.locked();
            if a.near.is_empty() && a.albums.is_empty() && a.hold_until.is_none_or(|t| t <= Instant::now()) {
                // The job taken last may still run: a moment more.
                drop(a);
                std::thread::sleep(Duration::from_millis(300));
                let a = ahead_state().0.locked();
                if a.near.is_empty() && a.albums.is_empty() {
                    return true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("np-index-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A song's record reads back as written, and a damaged one reads as none.
    #[test]
    fn a_record_reads_back_and_a_damaged_one_is_none() {
        let path = PathBuf::from(r"D:\music\Album\01 Song.flac");
        let r = Record {
            stamp: (1234, 5678),
            tags: Tags { title: Some("Song".into()), album: Some("Album".into()), ..Default::default() },
            picture: Some("0123456789abcdef0123456789abcdef".into()),
            lyrics: true,
        };
        let text = encode(&path, &r);
        assert_eq!(decode(text.as_bytes(), &path), Some(r.clone()));
        assert_eq!(decode(text.as_bytes(), Path::new(r"D:\music\Album\02 Other.flac")), None, "another song's");
        assert_eq!(decode(&text.as_bytes()[..text.len() / 2], &path), None, "cut short");
        assert_eq!(decode(text.replace("picture 0123", "picture zz23").as_bytes(), &path), None);
        let none = Record { picture: None, lyrics: false, ..r };
        assert_eq!(decode(encode(&path, &none).as_bytes(), &path), Some(none));
    }

    /// An album is its folder and its name (the name in any case; the folder in any case where
    /// the file system's names are, Windows); a song with no album has none.
    #[test]
    fn an_album_is_its_folder_and_its_name() {
        let root = std::env::temp_dir();
        let song = |folder: &str, file: &str| root.join("Music").join(folder).join(file);
        let a = album_key(&song("Album", "01.flac"), "The Album");
        assert!(a.is_some());
        assert_eq!(a, album_key(&song("Album", "02.flac"), " the album "), "another song, the name in any case");
        assert_ne!(a, album_key(&song("Other", "01.flac"), "The Album"), "another folder");
        assert_ne!(a, album_key(&song("Album", "01.flac"), "Another"), "another album");
        assert_eq!(album_key(&song("Album", "01.flac"), "  "), None);
        let other_case = album_key(&song("ALBUM", "01.flac"), "The Album");
        assert_eq!(a == other_case, cfg!(windows), "the folder's case counts where the file system's does");
    }

    /// A small picture's face is the picture itself (its pixels as they came, for every song
    /// of its album alike); a large one's is a JPEG at the face's size.
    #[test]
    fn a_face_is_the_picture_when_small_and_a_jpeg_at_its_size_when_large() {
        let png = |w, h| {
            let mut out = Vec::new();
            image::RgbImage::from_pixel(w, h, image::Rgb([30, 90, 200]))
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        let small = png(480, 480);
        assert_eq!(render_face(&small).as_deref(), Some(&small[..]));
        let large = render_face(&png(2000, 1500)).unwrap();
        assert_eq!(image::guess_format(&large).unwrap(), image::ImageFormat::Jpeg);
        let img = image::load_from_memory(&large).unwrap();
        assert_eq!((img.width(), img.height()), (FACE_SIDE, FACE_SIDE * 3 / 4));
    }

    /// A picture is kept once by its content: its thumbnail made from it (or, kept no longer,
    /// from its face) and read back the same.
    #[test]
    fn a_pictures_thumbnail_is_kept_and_remade_from_its_face() {
        let ix = Index::open(scratch("pictures"));
        let mut src = Vec::new();
        image::RgbImage::from_fn(300, 200, |x, y| image::Rgb([x as u8, y as u8, 99]))
            .write_to(&mut std::io::Cursor::new(&mut src), image::ImageFormat::Png)
            .unwrap();
        let id = disk_cache::key(&src);
        let made = ix.thumb_or_make(&id, Some(&src)).unwrap();
        assert_eq!(made.size(), (THUMB_SIDE, THUMB_SIDE));
        assert_eq!(ix.thumb(&id).unwrap().pixels(), made.pixels());
        assert!(ix.face_or_make(&id, &src).is_some());
        ix.disk.forget_file(&id, THUMB);
        assert!(ix.thumb(&id).is_none());
        assert_eq!(
            ix.thumb_or_make(&id, None).unwrap().pixels(),
            made.pixels(),
            "from its face, the picture as it came"
        );
    }
}
