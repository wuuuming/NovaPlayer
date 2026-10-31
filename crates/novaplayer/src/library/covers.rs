//! A song's artwork: a cover the listener chose for its folder, else the picture in its file,
//! else the nearest image the album marks as its cover (`resolve`). And each folder's listing,
//! read once a run: its pictures, and every file's size and time, by which the library index
//! knows a song's file has not changed (`stamp`) and that a song has lyrics beside it.
//!
//! Chosen covers live under `%LOCALAPPDATA%\NovaPlayer\chosen-covers` (never beside the
//! song): `index.txt` maps a folder (hex of its lowercased path) to a JPEG named by the folder's
//! hash and a generation, written before the index is, so a reader never sees half of one.
//!
//! Folder art: an image named as the song's own file (`CDImage.jpg` beside `CDImage.ape`); else in
//! the song's folder, then its parent (Disc1/, WAV/ under the album), an image whose name marks a
//! front cover (front > cover > folder > jacket, never back/rear/inlay/tray), else the first image
//! in an art folder (cover, scans, booklet, 画像 …). Listings are cached per directory.

use crate::sync::Locked;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const COVER_SIDE: u32 = 1040;

#[derive(Clone, Default)]
struct Listing {
    images: Vec<PathBuf>,
    named: Option<PathBuf>,
    art_folders: Vec<PathBuf>,
    /// Every file's size and time (seconds), by its lowercased name.
    stamps: std::sync::Arc<HashMap<String, (u64, u64)>>,
}

static LISTED: OnceLock<Mutex<HashMap<PathBuf, Listing>>> = OnceLock::new();
static CHOSEN: OnceLock<Chosen> = OnceLock::new();

/// The files a cover can be: pictures by their extension.
pub const IMAGE_EXT: [&str; 8] = ["jpg", "jpeg", "png", "webp", "bmp", "gif", "tif", "tiff"];

fn is_image(p: &Path) -> bool {
    super::has_ext(p, &IMAGE_EXT)
}

fn is_art_folder(name: &str) -> bool {
    const WORDS: [&str; 10] =
        ["cover", "covers", "bk", "booklet", "front", "jacket", "scan", "scans", "artwork", "画像"];
    name.split(|c: char| !c.is_alphanumeric()).any(|w| WORDS.iter().any(|a| w.eq_ignore_ascii_case(a)))
}

/// How strongly a name says front cover; None for a back/rear/inlay/tray image.
pub fn cover_rank(p: &Path) -> Option<usize> {
    let stem = p.file_stem()?.to_string_lossy().to_lowercase();
    if ["back", "rear", "inlay", "tray"].iter().any(|s| stem.contains(s)) {
        return None;
    }
    ["front", "cover", "folder", "jacket"].iter().position(|n| stem.contains(n))
}

fn list(folder: &Path) -> Listing {
    let m = LISTED.get_or_init(Default::default);
    if let Some(l) = m.locked().get(folder) {
        return l.clone();
    }
    let (mut images, mut folders, mut stamps) = (Vec::new(), Vec::new(), HashMap::new());
    if let Ok(rd) = std::fs::read_dir(folder) {
        for e in rd.filter_map(Result::ok) {
            let Ok(k) = e.file_type() else { continue };
            if k.is_symlink() {
                continue;
            }
            let p = e.path();
            if k.is_dir() && e.file_name().to_str().is_some_and(is_art_folder) {
                folders.push(p);
            } else if k.is_file() {
                // On Windows the listing carries each file's size and time: no file is opened.
                if let Ok(m) = e.metadata() {
                    stamps.insert(e.file_name().to_string_lossy().to_lowercase(), stamp_of(&m));
                }
                if is_image(&p) {
                    images.push(p);
                }
            }
        }
    }
    images.sort_by(|a, b| crate::library::by_name(a, b));
    folders.sort_by(|a, b| crate::library::by_name(a, b));
    let named =
        images.iter().filter_map(|p| cover_rank(p).map(|r| (r, p))).min_by_key(|(r, _)| *r).map(|(_, p)| p.clone());
    let l = Listing { images, named, art_folders: folders, stamps: std::sync::Arc::new(stamps) };
    m.locked().insert(folder.to_owned(), l.clone());
    l
}

/// The folder image for `song`, if any.
pub fn folder_image(song: &Path) -> Option<PathBuf> {
    let folder = song.parent()?;
    let own = song.file_stem()?.to_string_lossy().to_lowercase();
    let beside = list(folder)
        .images
        .into_iter()
        .find(|i| i.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == own));
    beside.or_else(|| {
        std::iter::once(folder).chain(folder.parent()).find_map(|place| {
            let l = list(place);
            l.named.or_else(|| {
                l.art_folders.iter().find_map(|a| {
                    let al = list(a);
                    al.named.or_else(|| al.images.first().cloned())
                })
            })
        })
    })
}

/// A file's size and time (seconds since 1970) as its folder's listing says.
pub(crate) fn stamp_of(m: &std::fs::Metadata) -> (u64, u64) {
    let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    (m.len(), t)
}

/// `file`'s size and time as its folder's listing (read once a run) says; `None` when the
/// folder cannot be listed (a share that is away) or does not list it. Asked of a file whose
/// folder is not listed yet, the folder is listed now.
pub fn stamp(file: &Path) -> Option<(u64, u64)> {
    let l = list(file.parent()?);
    l.stamps.get(&file.file_name()?.to_string_lossy().to_lowercase()).copied()
}

/// Whether timed lyrics lie beside `song` (`<name>.lrc`, `library::sidecar`), by its folder's
/// listing.
pub fn lyrics_beside(song: &Path) -> bool {
    let Some(name) = song.with_extension("lrc").file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
        return false;
    };
    song.parent().is_some_and(|f| list(f).stamps.contains_key(&name))
}

/// Every folder is listed again when next asked (tests: a new run).
#[cfg(feature = "test-support")]
pub fn forget_listings() {
    if let Some(m) = LISTED.get() {
        m.locked().clear();
    }
}

/// `folder` is listed again when next asked (its files were read again: a rescan).
pub fn forget_listing(folder: &Path) {
    if let Some(m) = LISTED.get() {
        m.locked().remove(folder);
    }
}

/// One resolution policy for both primary artwork and row thumbnails, including CUE paths:
/// the listener's choice for the folder wins (it is how a wrong embedded picture is put
/// right), then the picture in the file, then the folder's artwork.
pub fn resolve(song: &Path, embedded: Option<Vec<u8>>) -> Option<Vec<u8>> {
    chosen().read(song).or(embedded).or_else(|| std::fs::read(folder_image(song)?).ok())
}

/// The covers of `song`'s folder changed (one chosen, or given back): what is kept of the folder
/// — its listing, its rows' thumbnails — is read again. Given back, the folder's own picture
/// shows, as it is now.
pub fn folder_changed(song: &Path) {
    let Some(folder) = song.parent() else { return };
    if let Some(m) = LISTED.get() {
        m.locked().remove(folder);
    }
    super::index::invalidate_folder(folder);
}

pub struct Chosen {
    folder: Option<PathBuf>,
    entries: Mutex<HashMap<PathBuf, String>>,
}

pub fn chosen() -> &'static Chosen {
    CHOSEN.get_or_init(|| Chosen::open(crate::data_dir().map(|d| d.join("chosen-covers"))))
}

/// The key a chosen cover of `song` is kept under: its folder's (`key`); none for a song with no
/// folder.
fn folder_key(song: &Path) -> Option<PathBuf> {
    song.parent().map(key)
}

fn key(folder: &Path) -> PathBuf {
    PathBuf::from(folder.to_string_lossy().to_lowercase())
}

fn hex(p: &Path) -> String {
    p.to_string_lossy().as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(h: &str) -> Option<PathBuf> {
    if !h.len().is_multiple_of(2) {
        return None;
    }
    let b: Vec<u8> = h
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
        .collect::<Option<_>>()?;
    String::from_utf8(b).ok().map(PathBuf::from)
}

impl Chosen {
    pub fn open(folder: Option<PathBuf>) -> Self {
        let mut entries = HashMap::new();
        if let Some(text) = folder.as_ref().and_then(|f| std::fs::read_to_string(f.join("index.txt")).ok()) {
            for line in text.lines() {
                if let Some((name, path)) = line.split_once('\t') {
                    if name.ends_with(".jpg") && !name.contains(['/', '\\']) {
                        if let Some(p) = unhex(path) {
                            entries.insert(key(&p), name.to_owned());
                        }
                    }
                }
            }
        }
        Chosen { folder, entries: Mutex::new(entries) }
    }

    pub fn has(&self, song: &Path) -> bool {
        folder_key(song).is_some_and(|k| self.entries.locked().contains_key(&k))
    }

    pub fn read(&self, song: &Path) -> Option<Vec<u8>> {
        let name = self.entries.locked().get(&folder_key(song)?)?.clone();
        std::fs::read(self.folder.as_ref()?.join(name)).ok()
    }

    fn write_index(&self, root: &Path, e: &HashMap<PathBuf, String>) -> std::io::Result<()> {
        let mut lines: Vec<String> = e.iter().map(|(p, n)| format!("{n}\t{}\n", hex(p))).collect();
        lines.sort();
        crate::files::write_atomic(&root.join("index.txt"), lines.concat().as_bytes())
    }

    /// Keeps `image` (scaled to the cover's size, as JPEG) as the cover of `song`'s folder. A
    /// gain-map JPEG is kept as it is: a re-encoding would drop what an HDR display shows of it.
    pub fn choose(&self, song: &Path, image: &Path) -> std::io::Result<Vec<u8>> {
        let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string());
        let k = folder_key(song).ok_or_else(|| bad("song has no folder"))?;
        let root = self.folder.clone().ok_or_else(|| bad("no data folder"))?;
        let bytes = std::fs::read(image)?;
        let img = novaui::image::load(&bytes).ok_or_else(|| bad("image does not decode"))?;
        let kept = if novaui::image::has_gain_map(&bytes) {
            bytes
        } else {
            let img = img.resize_to_fill(
                COVER_SIDE.min(img.width().min(img.height())).max(1),
                COVER_SIDE.min(img.width().min(img.height())).max(1),
                image::imageops::FilterType::Triangle,
            );
            let mut kept = Vec::new();
            img.into_rgb8()
                .write_to(&mut std::io::Cursor::new(&mut kept), image::ImageFormat::Jpeg)
                .map_err(|_| bad("encode failed"))?;
            kept
        };
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        k.hash(&mut h);
        let stamp =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        let name = format!("{:016x}-{:016x}.jpg", h.finish(), stamp);
        crate::files::write_atomic(&root.join(&name), &kept)?;
        let mut e = self.entries.locked();
        let mut next = e.clone();
        let old = next.insert(k, name.clone());
        if let Err(err) = self.write_index(&root, &next) {
            let _ = std::fs::remove_file(root.join(name));
            return Err(err);
        }
        *e = next;
        if let Some(o) = old {
            let _ = std::fs::remove_file(root.join(o));
        }
        Ok(kept)
    }

    pub fn remove(&self, song: &Path) -> std::io::Result<()> {
        let Some(k) = folder_key(song) else { return Ok(()) };
        let Some(root) = self.folder.clone() else { return Ok(()) };
        let mut e = self.entries.locked();
        let mut next = e.clone();
        if let Some(o) = next.remove(&k) {
            self.write_index(&root, &next)?;
            *e = next;
            let _ = std::fs::remove_file(root.join(o));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_thumbnail_survives_cold_lookup_and_released_cache() {
        let root = std::env::temp_dir().join(format!("np-cold-art-{}", std::process::id()));
        let disc = root.join("Disc1");
        std::fs::create_dir_all(&disc).unwrap();
        let song = disc.join("CDImage.wav"); // Also the source path used by CUE spans.
        let cover = root.join("cover.png");
        image::RgbaImage::from_pixel(16, 24, image::Rgba([19, 93, 207, 255])).save(&cover).unwrap();
        let first = crate::library::thumb(&resolve(&song, None).unwrap(), 96).unwrap();
        let pixels = first.pixels().to_vec();
        let weak = first.downgrade();
        drop(first);
        assert!(weak.upgrade().is_none(), "completed decode must not pin the thumbnail");
        let again = crate::library::thumb(&resolve(&song, None).unwrap(), 96).unwrap();
        assert_eq!(again.pixels(), pixels.as_slice());
        assert_eq!(resolve(&song, Some(vec![1, 2, 3])), Some(vec![1, 2, 3]), "embedded picture wins");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Regression: a cover given back left the folder's listing as read before, so a picture
    /// put in the folder since was not shown.
    #[test]
    fn a_folder_whose_covers_changed_is_listed_again() {
        let root = std::env::temp_dir().join(format!("np-cover-changed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let album = root.join("album");
        std::fs::create_dir_all(&album).unwrap();
        let song = album.join("01.flac");
        assert!(folder_image(&song).is_none());
        image::RgbImage::from_pixel(8, 8, image::Rgb([9, 9, 9])).save(album.join("cover.png")).unwrap();
        folder_changed(&song);
        assert_eq!(folder_image(&song), Some(album.join("cover.png")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn front_covers_rank_and_backs_do_not() {
        assert_eq!(cover_rank(Path::new("Front.jpg")), Some(0));
        assert_eq!(cover_rank(Path::new("folder.png")), Some(2));
        assert_eq!(cover_rank(Path::new("Cover_back.jpg")), None);
        assert_eq!(cover_rank(Path::new("scan01.jpg")), None);
        assert!(is_art_folder("KIDA-5501 Booklet"));
        assert!(!is_art_folder("New folder"));
    }

    #[test]
    fn chosen_round_trips_through_the_index() {
        let dir = std::env::temp_dir().join(format!("np-chosen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let img = dir.join("pic.png");
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbImage::from_pixel(8, 8, image::Rgb([200, 10, 10])).save(&img).unwrap();
        let song = dir.join("album").join("01.flac");
        let c = Chosen::open(Some(dir.join("chosen")));
        assert!(!c.has(&song));
        c.choose(&song, &img).unwrap();
        let again = Chosen::open(Some(dir.join("chosen")));
        assert!(again.has(&song) && again.read(&song).is_some());
        again.remove(&song).unwrap();
        assert!(!Chosen::open(Some(dir.join("chosen"))).has(&song));
        let _ = std::fs::remove_dir_all(dir);
    }
}
