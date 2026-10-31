//! Songs a plugin resolves (docs/plugins.md, protocol v3).
//!
//! NovaPlayer knows no service. A plugin declares URI schemes (`svc`, …); a song whose
//! path is such a URI (`svc:song:123`) is that plugin's, wherever it came from: a list the
//! plugin turned a link into (an ordinary M3U8 text), or any `.m3u8` on disk that names one.
//! Right before it plays, its plugin `resolve`s it to a standard audio address (http(s) or a
//! file), used once and never stored. Everything else about it is standard: its words, cover
//! and lyrics come from `metadata`, its codec and rates from probing the stream.

use crate::library::{Layer, Song, Tags};
use crate::plugins::accounts::Accounts;
use crate::plugins::pcm::host;
use crate::sync::Locked;
use novaplayer_plugin_protocol::json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// v1 lists stored songs as `plugin:<id>/<reference>`; the reference is the v2 URI.
const LEGACY: &str = "plugin:";

/// The URI of a song a plugin resolves, and that plugin.
pub fn parse(path: &Path) -> Option<(String, String)> {
    let s = path.to_str()?;
    let uri = match s.strip_prefix(LEGACY) {
        Some(rest) => rest.split_once('/')?.1,
        None => s,
    };
    let plugin = crate::plugins::pcm::resolver_for(scheme(uri)?)?;
    Some((plugin, uri.to_owned()))
}

/// A URI's scheme: two or more of `[A-Za-z0-9+.-]` before the first `:`, starting with a
/// letter (one letter is a Windows drive, not a scheme).
pub fn scheme(s: &str) -> Option<&str> {
    let (sc, _) = s.split_once(':')?;
    let ok = sc.len() >= 2
        && sc.starts_with(|c: char| c.is_ascii_alphabetic())
        && sc.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    ok.then_some(sc)
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_online(path: &Path) -> bool {
    parse(path).is_some()
}

/// Which enabled plugin turns `url` into a playlist (by scheme, or host for http(s)).
pub fn plugin_for(reg: &crate::plugins::registry::Registry, url: &str) -> Option<String> {
    let url = url.trim();
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase())?;
    let host = url.split_once("://").map(|(_, r)| r.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase());
    reg.list.iter().filter(|p| p.is_on()).find_map(|p| {
        let m = p.manifest.as_ref()?;
        let hit = m.provides.links.iter().any(|s| {
            let s = s.to_ascii_lowercase();
            s == scheme || host.as_deref().is_some_and(|h| h == s || h.ends_with(&format!(".{s}")))
        });
        hit.then(|| p.id())
    })
}

/// A playlist read from a link: its name and songs.
pub struct Fetched {
    pub name: String,
    pub songs: Vec<Song>,
    /// Songs the service listed but gave no way to play.
    pub unavailable: usize,
}

/// A plugin's address (`resolve`'s `url`, a cover) is a URL when it holds `://`, else a local
/// path (plugins.md).
pub fn is_url(addr: &str) -> bool {
    addr.contains("://")
}

fn with_credential(acc: &Accounts, plugin: &str, mut params: Vec<(&'static str, Value)>) -> Vec<(&'static str, Value)> {
    if let Some(c) = acc.credential(plugin) {
        params.push(("credential", Value::Str(c)));
    }
    params
}

/// Asks `plugin` for the playlist at `url`: an M3U8 text, read as any M3U8 is.
pub fn fetch(acc: &Accounts, plugin: &str, url: &str) -> Result<Fetched, String> {
    let params = with_credential(acc, plugin, vec![("url", Value::Str(url.to_owned()))]);
    // A big list is many requests on the plugin's side.
    let r = host().ask(plugin, "playlist", &params)?;
    let text = r.str("m3u8").ok_or_else(|| novaui::i18n::t("err.plugin-no-list", &[]))?;
    let list = crate::library::parse_m3u8(text, None);
    let name = list.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| novaui::i18n::t("list.online", &[]));
    let unavailable = list.missing + r.num("unavailable").map_or(0, |n| n.max(0.0) as usize);
    let mut songs = list.songs;
    for s in &mut songs {
        if let Some(n) = &s.tags.title {
            s.name = n.clone();
        }
        // Its words came with the list: there is no file to read them from.
        s.probed = true;
    }
    Ok(Fetched { name, songs, unavailable })
}

/// One of the connected account's own playlists (`lists`).
#[derive(Clone, Debug)]
pub struct UserList {
    pub url: String,
    pub name: String,
    pub count: Option<u64>,
    /// The heading it is listed under ("我创建的"), when the plugin groups them.
    pub group: String,
}

/// The connected account's own playlists; every url is one the plugin's `playlist` accepts.
pub fn user_lists(acc: &Accounts, plugin: &str) -> Result<Vec<UserList>, String> {
    let r = host().ask(plugin, "lists", &with_credential(acc, plugin, Vec::new()))?;
    let mut out = Vec::new();
    for l in r.value.get("lists").and_then(Value::as_array).into_iter().flatten() {
        let s = |k: &str| l.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        let Some(url) = s("url") else { continue };
        out.push(UserList {
            url: url.to_owned(),
            name: s("name").map_or_else(|| novaui::i18n::t("list.account", &[]), str::to_owned),
            count: l.get("count").and_then(Value::as_f64).filter(|n| *n >= 0.0).map(|n| n as u64),
            group: s("group").unwrap_or("").to_owned(),
        });
    }
    Ok(out)
}

/// A resolved song: the address to play, and the option value the plugin gave it at.
pub struct Resolved {
    pub url: String,
    /// The value's id and name ("lossless", "无损"), for plugins that declare an option.
    pub option: Option<(String, String)>,
}

/// The playable address for a song, asked for now (never stored), at the user's chosen value
/// of the plugin's option.
pub fn resolve(acc: &Accounts, plugin: &str, uri: &str) -> Result<Resolved, String> {
    let mut params = with_credential(acc, plugin, vec![("uri", Value::Str(uri.to_owned()))]);
    if let Some(v) = option::get(plugin) {
        params.push(("option", Value::Str(v)));
    }
    let r = host().ask(plugin, "resolve", &params)?;
    let url = r
        .str("url")
        .filter(|u| !u.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| novaui::i18n::t("err.song-unavailable", &[]))?;
    let id = r.str("option").filter(|q| !q.is_empty()).map(str::to_owned);
    let name = r.str("option_name").map(str::to_owned).unwrap_or_default();
    Ok(Resolved { url, option: id.map(|q| (q, name)) })
}

/// One value of a plugin's option: its id, its name, what it is ("96 kHz / 24-bit FLAC").
#[derive(Clone, Debug)]
pub struct OptionValue {
    pub id: String,
    pub name: String,
    pub label: String,
}

/// The values the song comes in, as the plugin orders them (`options`).
pub fn options(acc: &Accounts, plugin: &str, uri: &str) -> Result<Vec<OptionValue>, String> {
    let params = with_credential(acc, plugin, vec![("uri", Value::Str(uri.to_owned()))]);
    let r = host().ask(plugin, "options", &params)?;
    Ok(r.value
        .get("values")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|q| {
            let s = |k: &str| q.get(k).and_then(Value::as_str).map(str::to_owned);
            Some(OptionValue {
                id: s("id")?,
                name: s("name").unwrap_or_default(),
                label: s("label").unwrap_or_default(),
            })
        })
        .collect())
}

/// The user's chosen value of each plugin's option (none chosen: the plugin's own default),
/// kept in `NovaPlayer/options.txt` as `plugin<TAB>value` lines.
pub mod option {
    use crate::sync::Locked;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    pub fn file() -> Option<std::path::PathBuf> {
        crate::data_dir().map(|d| d.join("options.txt"))
    }

    fn all() -> &'static Mutex<BTreeMap<String, String>> {
        static M: std::sync::OnceLock<Mutex<BTreeMap<String, String>>> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            let text = file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
            Mutex::new(
                text.lines()
                    .filter_map(|l| l.split_once('\t'))
                    .map(|(a, b)| (a.trim().to_owned(), b.trim().to_owned()))
                    .collect(),
            )
        })
    }

    pub fn get(plugin: &str) -> Option<String> {
        all().locked().get(plugin).cloned()
    }

    /// Chooses `v` for `plugin`; the file is the caller's to keep (`app::kept`).
    pub fn set(plugin: &str, v: &str) {
        all().locked().insert(plugin.to_owned(), v.to_owned());
    }

    /// The file's text.
    pub fn text() -> String {
        all().locked().iter().map(|(a, b)| format!("{a}\t{b}\n")).collect()
    }

    /// Songs a plugin gave at another value than the user chose (the song does not come in
    /// it): the player says so once, until the choice changes or the player starts again.
    #[derive(Default)]
    pub struct Fallback {
        /// The open (song version) already looked at.
        seen: u64,
        /// Each plugin's chosen value already told of.
        told: BTreeMap<String, String>,
    }

    impl Fallback {
        /// Whether the open `version` was already looked at.
        pub fn seen(&self, version: u64) -> bool {
            self.seen == version
        }

        /// The open `version` of a `plugin`'s song resolved at `given` while the user chose
        /// `chosen`: true when it fell back and nothing was told of that choice yet.
        pub fn tell(&mut self, version: u64, plugin: &str, chosen: Option<&str>, given: &str) -> bool {
            if self.seen == version {
                return false;
            }
            self.seen = version;
            let Some(chosen) = chosen.filter(|c| *c != given) else { return false };
            if self.told.get(plugin).is_some_and(|c| c == chosen) {
                return false;
            }
            self.told.insert(plugin.to_owned(), chosen.to_owned());
            true
        }

        /// The user picked a value of `plugin`'s option: its next fallback is told again.
        pub fn forget(&mut self, plugin: &str) {
            self.told.remove(plugin);
        }
    }
}

/// The line under the artwork for a plugin's song, from the plugin's `style.res` template:
/// `{res}` the stream's own resolution, `{option}` the value's name. Split where `{option}`
/// starts: the part before is the `res` text, the rest the `res-note` text (styled apart in
/// ui/player.nui). Without a template the line is the resolution alone.
pub fn res_line(template: Option<&str>, res: &str, option: &str) -> (String, String) {
    let Some(t) = template else { return (res.to_owned(), String::new()) };
    let (head, tail) = match t.find("{option}") {
        Some(i) => (&t[..i], &t[i..]),
        None => (t, ""),
    };
    let fill = |s: &str| s.replace("{res}", res).replace("{option}", option);
    let (mut a, mut b) = (fill(head), fill(tail));
    // A missing half takes its separator with it ("· 无损" with no resolution yet).
    if option.is_empty() {
        b.clear();
        a = a.trim_end_matches(|c: char| c.is_whitespace() || "·|-/".contains(c)).to_owned();
    }
    if res.is_empty() {
        a.clear();
        b = b.trim_start_matches(|c: char| c.is_whitespace() || "·|-/".contains(c)).to_owned();
    }
    // The row sets the gap between the halves: no space at either text's edge.
    (a.trim().to_owned(), b.trim().to_owned())
}

/// What the face shows for an online song, from its plugin's `metadata`: the words (missing
/// ones filled), the cover's bytes, the lyrics (LRC text).
#[derive(Clone)]
pub struct TrackInfo {
    pub tags: Tags,
    pub cover: Option<Vec<u8>>,
    pub lyrics: Option<String>,
    /// Extra lines under the lyrics (docs/extensions.md, "Lyrics layers"), as the plugin gave them.
    pub layers: Vec<Layer>,
}

/// The song's face: from the disk cache, else asked of its plugin now (on play, or when its
/// row comes into view) and kept. `refresh` asks again (a resynced list).
pub fn track_info(song: &Song, refresh: bool) -> Option<TrackInfo> {
    match face_words(song, refresh)? {
        Words::Kept(f) => Some(f),
        Words::Asked { info, cover_at } => Some(face_cover(&song.path, info, cover_at)),
    }
}

/// A face's words, before its picture.
pub enum Words {
    /// Kept whole (its picture, if it has one, with it).
    Kept(TrackInfo),
    /// Asked of the plugin now: the words and lyrics, and where the picture is still to be
    /// fetched (`face_cover`).
    Asked { info: TrackInfo, cover_at: Option<String> },
}

/// The song's words and lyrics: kept, or asked of its plugin (the picture is fetched after, by
/// `face_cover`: the words need not wait for it).
pub fn face_words(song: &Song, refresh: bool) -> Option<Words> {
    parse(&song.path)?;
    if !refresh {
        if let Some(f) = cache::read(&song.path) {
            return Some(Words::Kept(merge(song, f)));
        }
    }
    let (tags, cover_at, lyrics, layers) = complete_full(song)?;
    Some(Words::Asked { info: TrackInfo { tags, cover: None, lyrics, layers }, cover_at })
}

/// Fetches the picture `face_words` named and keeps the face whole. A picture named but not
/// arrived (a dropped connection) is asked for again next time: the face is kept only whole.
pub fn face_cover(path: &Path, mut info: TrackInfo, cover_at: Option<String>) -> TrackInfo {
    let named = cover_at.is_some();
    let cover = cover_at.and_then(|c| if is_url(&c) { fetch_cover(&c) } else { std::fs::read(c).ok() });
    info.cover = cover.map(|b| cache::compact(&b).unwrap_or(b));
    if !named || info.cover.is_some() {
        cache::write(path, &info);
    }
    info
}

/// A control's `plugin.<name>`: the plugin's `command` call. → (toast, refresh the face).
pub fn command(acc: &Accounts, plugin: &str, name: &str, uri: &str) -> Result<(Option<String>, bool), String> {
    let params =
        with_credential(acc, plugin, vec![("name", Value::Str(name.to_owned())), ("uri", Value::Str(uri.to_owned()))]);
    let r = host().ask(plugin, "command", &params)?;
    let toast = r.str("toast").filter(|t| !t.trim().is_empty()).map(str::to_owned);
    let refresh = r.value.get("refresh").and_then(Value::as_bool).unwrap_or(false);
    Ok((toast, refresh))
}

/// Drops one song's kept face (a plugin said it changed).
pub fn forget_face(path: &Path) {
    cache::forget(path);
}

/// The build of the plugin a song belongs to (its enabled fingerprint's first 16 hex), "" for
/// none: kept faces carry it, so a plugin's update asks its songs again.
pub fn plugin_stamp(path: &Path) -> String {
    let Some((plugin, _)) = parse(path) else { return String::new() };
    STAMPS.locked().iter().find(|(p, _)| *p == plugin).map(|(_, s)| s.clone()).unwrap_or_default()
}

static STAMPS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Tests that set `STAMPS` by hand hold this, and so does `set_stamps` (registry tests reload in
/// parallel): a reload cannot clear a stamp such a test is relying on.
#[cfg(test)]
static STAMPS_BY_HAND: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The enabled plugins' builds, from the registry (on start and every reload).
pub fn set_stamps(reg: &crate::plugins::registry::Registry) {
    #[cfg(test)]
    let _by_hand = STAMPS_BY_HAND.locked();
    let mut s = STAMPS.locked();
    s.clear();
    for p in &reg.list {
        if let Some(h) = reg.stamp_of(&p.id()) {
            s.push((p.id(), h.chars().take(16).collect()));
        }
    }
}

/// Small covers for many of a plugin's songs (`thumbs`): uri → cover URL. Empty when the plugin
/// does not offer it or the call fails.
pub fn thumbs(acc: &Accounts, plugin: &str, uris: &[String]) -> Vec<(String, String)> {
    let params =
        with_credential(acc, plugin, vec![("uris", Value::Arr(uris.iter().map(|u| Value::Str(u.clone())).collect()))]);
    let Ok(r) = host().ask(plugin, "thumbs", &params) else { return Vec::new() };
    let Some(Value::Obj(m)) = r.value.get("thumbs") else { return Vec::new() };
    m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect()
}

/// A row's thumbnail as kept (the prefetched small cover's, else its kept face's), when it is.
pub fn kept_thumb(path: &Path) -> Option<novaui::ImageHandle> {
    cache::read_thumb(path)
}

/// Fetches a small cover and keeps it as the row's thumbnail; false when it did not arrive.
pub fn fetch_thumb(path: &Path, url: &str) -> bool {
    if cache::read_thumb(path).is_some() {
        return true;
    }
    let Some(b) = fetch_cover(url) else { return false };
    cache::write_thumb(path, &b)
}

/// The song's face is kept (its picture, if it has one, came with it). False after a fetch
/// that failed: its row asks again later.
pub fn track_kept(path: &Path) -> bool {
    cache::read(path).is_some()
}

/// Drops every kept face (the app menu's 清除在线缓存).
pub fn clear_cache() {
    cache::clear();
}

/// The list's own words win over cached ones (as over the plugin's).
fn merge(song: &Song, mut f: TrackInfo) -> TrackInfo {
    let t = &song.tags;
    let pick = |a: &Option<String>, b: Option<String>| a.clone().filter(|v| !v.trim().is_empty()).or(b);
    f.tags.title = pick(&t.title, f.tags.title.take());
    f.tags.artist = pick(&t.artist, f.tags.artist.take());
    f.tags.album = pick(&t.album, f.tags.album.take());
    f.tags.duration = t.duration.or(f.tags.duration);
    f
}

/// The on-disk face cache: only what the pages show (words, lyrics, which picture is its cover
/// and its row's thumbnail). Never a stream or cover address: services forbid keeping those. A
/// song's words in its plugin's group of its own disk cache (`library::disk_cache`); its
/// pictures in the library index's picture store, by their content (`index::keep_face`): the
/// songs of an album, one picture kept.
mod cache {
    use super::TrackInfo;
    use crate::library::disk_cache::{self, DiskCache};
    use crate::library::{Layer, Tags};
    use std::path::Path;
    use std::time::Duration;

    const LYRICS: &str = "--lyrics--";
    const LAYER: &str = "--layer--";
    /// Faces kept before this revision lack what later ones hold (lyrics layers): read as
    /// absent, so the song asks its plugin once more.
    const REV: u32 = 3;
    /// What the kept faces' words may take on disk, together (their pictures are the index's).
    const BUDGET: u64 = 64 << 20;
    /// A kept cover's longest side: what the face draws.
    const COVER_SIDE: u32 = 1040;

    /// How many songs' faces may be kept, at most.
    const ENTRIES: usize = 50_000;

    fn disk() -> Option<&'static DiskCache> {
        static DISK: std::sync::OnceLock<Option<DiskCache>> = std::sync::OnceLock::new();
        DISK.get_or_init(|| {
            crate::data_dir().map(|d| {
                // The cache before pictures moved to the index (each song's own copy): read again.
                let _ = std::fs::remove_dir_all(d.join("online-v3"));
                DiskCache::open(d.join("online-v4"), BUDGET, ENTRIES)
            })
        })
        .as_ref()
    }

    /// A song's key and its plugin's group.
    fn keys(path: &Path) -> (String, String) {
        let plugin = super::parse(path).map(|(p, _)| p).unwrap_or_default();
        (disk_cache::path_key(path), disk_cache::key(plugin.as_bytes()))
    }

    /// A cover as kept: at most 1040 px, re-encoded as JPEG (quality 88) when that is smaller
    /// than what came. Covers are photographs; a 7.9 MB PNG master keeps as ~200 KB.
    pub fn compact(bytes: &[u8]) -> Option<Vec<u8>> {
        // Bytes from a service: decoded within novaui's bounds (`image::load`).
        let img = novaui::image::load(bytes)?;
        let img = if img.width().max(img.height()) > COVER_SIDE {
            img.resize(COVER_SIDE, COVER_SIDE, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        let rgb = img.into_rgb8();
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88).encode_image(&rgb).ok()?;
        (out.len() < bytes.len()).then_some(out)
    }

    /// Text, deflated (the words and an LRC compress four to one).
    fn pack(text: &str) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 6)
    }

    fn unpack(b: &[u8]) -> Option<String> {
        String::from_utf8(miniz_oxide::inflate::decompress_to_vec_with_limit(b, 8 << 20).ok()?).ok()
    }

    /// Where a song's face's words are kept (tests).
    #[cfg(test)]
    pub fn meta_file(path: &Path) -> Option<std::path::PathBuf> {
        disk()?.find(&keys(path).0, "meta")
    }

    pub fn clear() {
        if let Some(d) = disk() {
            d.clear();
        }
    }

    /// A row's thumbnail, kept apart from the face (a list's are prefetched long before any of
    /// its songs is played): at most 96 px, JPEG.
    pub fn forget(path: &Path) {
        if let Some(d) = disk() {
            let (key, _) = keys(path);
            d.forget_file(&key, "meta");
            d.forget_file(&key, "cover");
        }
    }

    /// A picture this song names (`what`: "thumb" or "cover"), by its key in the store.
    fn named(path: &Path, what: &str) -> Option<String> {
        let f = disk()?.find(&keys(path).0, what)?;
        let id = String::from_utf8(std::fs::read(f).ok()?).ok()?;
        crate::library::index::is_picture_id(&id).then_some(id)
    }

    /// Its row's thumbnail: the small cover prefetched for it, else its kept face's.
    pub fn read_thumb(path: &Path) -> Option<novaui::ImageHandle> {
        let id = named(path, "thumb").or_else(|| {
            let (d, key) = (disk()?, keys(path).0);
            d.find(&key, "meta")?;
            named(path, "cover")
        })?;
        crate::library::index::thumb_of(&id)
    }

    pub fn write_thumb(path: &Path, bytes: &[u8]) -> bool {
        let Some(d) = disk() else { return false };
        let Some(id) = crate::library::index::keep_thumb(bytes) else { return false };
        let (key, group) = keys(path);
        d.put(&group, &key, "thumb", id.as_bytes()).is_ok()
    }

    pub fn read(path: &Path) -> Option<TrackInfo> {
        let (d, key) = (disk()?, keys(path).0);
        // Read is use (`DiskCache::find`): it moves to the back of the line for trimming.
        let text = unpack(&std::fs::read(d.find(&key, "meta")?).ok()?)?;
        let (head, rest) = match text.split_once(&format!("\n{LYRICS}\n")) {
            Some((h, l)) => (h, Some(l)),
            None => (text.as_str(), None),
        };
        let rev: u32 = head.lines().find_map(|l| l.strip_prefix("rev\t")).and_then(|v| v.parse().ok()).unwrap_or(1);
        if rev < REV {
            return None;
        }
        // A face kept for another build of its plugin is not trusted: the plugin is asked again.
        let kept_by = head.lines().find_map(|l| l.strip_prefix("plugin\t")).unwrap_or("");
        if kept_by != super::plugin_stamp(path) {
            return None;
        }
        let sep = format!("\n{LAYER}\t");
        let mut parts = rest.unwrap_or("").split(sep.as_str());
        let lyrics = parts.next().map(str::to_owned).filter(|l| !l.trim().is_empty());
        let layers = parts
            .filter_map(|p| {
                let (head, lrc) = p.split_once('\n')?;
                let (id, name) = head.split_once('\t').unwrap_or((head, head));
                Some(Layer { id: id.to_owned(), name: name.to_owned(), lrc: lrc.to_owned() })
            })
            .collect();
        let mut t = Tags::default();
        for line in head.lines() {
            let Some((k, v)) = line.split_once('\t') else { continue };
            let v = Some(v.to_owned()).filter(|v| !v.is_empty());
            match k {
                "title" => t.title = v,
                "artist" => t.artist = v,
                "album" => t.album = v,
                "album_artist" => t.album_artist = v,
                "track" => t.track = v,
                "date" => t.date = v,
                "genre" => t.genre = v,
                "duration" => t.duration = v.and_then(|d| d.parse().ok()).map(Duration::from_secs_f64),
                _ => {}
            }
        }
        // A cover named but no longer kept (the store let it go): the face is asked again.
        let cover = match named(path, "cover") {
            Some(id) => Some(crate::library::index::face_of(&id)?),
            None => None,
        };
        Some(TrackInfo { tags: t, cover, lyrics, layers })
    }

    pub fn write(path: &Path, f: &TrackInfo) {
        let Some(d) = disk() else { return };
        let (key, group) = keys(path);
        let t = &f.tags;
        let one = |v: &Option<String>| v.as_deref().unwrap_or("").replace(['\r', '\n', '\t'], " ");
        let mut text = format!("rev\t{REV}\nplugin\t{}\n", super::plugin_stamp(path));
        for (k, v) in [
            ("title", one(&t.title)),
            ("artist", one(&t.artist)),
            ("album", one(&t.album)),
            ("album_artist", one(&t.album_artist)),
            ("track", one(&t.track)),
            ("date", one(&t.date)),
            ("genre", one(&t.genre)),
            ("duration", t.duration.map(|d| d.as_secs_f64().to_string()).unwrap_or_default()),
        ] {
            text.push_str(&format!("{k}\t{v}\n"));
        }
        if f.lyrics.is_some() || !f.layers.is_empty() {
            text.push_str(&format!("{LYRICS}\n{}", f.lyrics.as_deref().unwrap_or("")));
        }
        for l in &f.layers {
            let one = |s: &str| s.replace(['\r', '\n', '\t'], " ");
            text.push_str(&format!("\n{LAYER}\t{}\t{}\n{}", one(&l.id), one(&l.name), l.lrc));
        }
        // The cover first (in the store, by content; its name here): a .meta file means the
        // face is whole. Each file is written whole or not at all: a row and the face asking at
        // once never leave half a file.
        match f.cover.as_deref().map(crate::library::index::keep_face) {
            Some(Some(id)) => {
                if d.put(&group, &key, "cover", id.as_bytes()).is_err() {
                    return;
                }
            }
            Some(None) => return,
            None => d.forget_file(&key, "cover"),
        }
        let _ = d.put(&group, &key, "meta", &pack(&text));
    }
}

/// A cover the plugin gave as a URL, fetched over http(s) for this face (the picture is kept,
/// never the address). The size is the plugin's to ask for; whatever arrives is kept at most
/// 1040 px (`cache::compact`). Tried three times; at most 24 MB.
fn fetch_cover(url: &str) -> Option<Vec<u8>> {
    if !crate::files::is_web_address(url) {
        return None;
    }
    for k in 0..3u64 {
        if k > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400 * k));
        }
        if let Some(b) = fetch_url(url, 24 << 20).filter(|b| image::guess_format(b).is_ok()) {
            return Some(b);
        }
    }
    None
}

/// The bytes at an http(s) `url`, at most `max`, within 20 s; none on any failure (a status
/// other than success among them). The same on every host (principles B5).
fn fetch_url(url: &str, max: usize) -> Option<Vec<u8>> {
    let config = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(20)));
    // Windows: SChannel with the system's roots (Cargo.toml); elsewhere rustls with Mozilla's.
    #[cfg(windows)]
    let config = config.tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build(),
    );
    let config = config.build();
    let mut answer = ureq::Agent::new_with_config(config).get(url).call().ok()?;
    let bytes = answer.body_mut().with_config().limit(max as u64).read_to_vec().ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

/// Online metadata for a song of a plugin's list (never a local song): fills what is missing.
#[cfg(any(test, feature = "test-support"))]
pub fn complete(song: &Song) -> Option<Tags> {
    complete_full(song).map(|(t, ..)| t)
}

/// A plugin's metadata for a song: its tags, its cover's address, its lyrics, its lyric layers.
type Metadata = (Tags, Option<String>, Option<String>, Vec<Layer>);

fn complete_full(song: &Song) -> Option<Metadata> {
    let (plugin, reference) = parse(&song.path)?;
    let t = &song.tags;
    let s = |v: &Option<String>| Value::Str(v.clone().unwrap_or_default());
    let mut params =
        vec![("uri", Value::Str(reference)), ("title", s(&t.title)), ("artist", s(&t.artist)), ("album", s(&t.album))];
    if let Some(d) = t.duration {
        params.push(("duration", Value::Num(d.as_secs_f64())));
    }
    let r = host().ask(&plugin, "metadata", &params).ok()?;
    let words = r.tags();
    let mut out = t.clone();
    out.title = out.title.or(words.title);
    out.artist = out.artist.or(words.artist);
    out.album = out.album.or(words.album);
    out.album_artist = out.album_artist.or(words.album_artist);
    out.track = out.track.or(words.track);
    out.date = out.date.or(words.date);
    out.genre = out.genre.or(words.genre);
    // `layers`: { "<id>": { "name": "翻译", "lrc": "…" } }.
    let mut layers = Vec::new();
    if let Some(Value::Obj(m)) = r.value.get("layers") {
        for (id, l) in m {
            let lrc = l.get("lrc").and_then(Value::as_str).filter(|t| t.contains('['));
            if let Some(lrc) = lrc {
                let name = l.get("name").and_then(Value::as_str).unwrap_or(id).to_owned();
                layers.push(Layer { id: id.clone(), name, lrc: lrc.to_owned() });
            }
        }
    }
    Some((out, r.text("cover"), r.text("lyrics"), layers))
}

/// Tells the plugin what happened to its song (services that ask for play reports). Fire and
/// forget: playback never waits on it. One reporter thread sends them in the order they
/// happened (a thread each let `pause` overtake `start`).
pub fn play_event(acc: &Arc<Accounts>, path: &Path, event: &'static str, position: Duration) {
    use std::sync::mpsc::{channel, Sender};
    use std::sync::{Mutex, OnceLock};
    type Report = (Arc<Accounts>, String, String, &'static str, Duration);
    static REPORTER: OnceLock<Mutex<Sender<Report>>> = OnceLock::new();
    let Some((plugin, reference)) = parse(path) else { return };
    let tx = REPORTER.get_or_init(|| {
        let (tx, rx) = channel::<Report>();
        std::thread::spawn(move || {
            for (acc, plugin, reference, event, position) in rx {
                let params = vec![
                    ("uri", Value::Str(reference)),
                    ("event", Value::Str(event.into())),
                    ("position", Value::Num(position.as_secs_f64())),
                ];
                let params = with_credential(&acc, &plugin, params);
                let _ = host().ask(&plugin, "play_event", &params);
            }
        });
        Mutex::new(tx)
    });
    let _ = tx.locked().send((acc.clone(), plugin, reference, event, position));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() {
        let exe = crate::plugins::host::demo_plugin();
        if !host().registered("demo") {
            host().register("demo", exe.parent().unwrap().to_path_buf(), exe, true);
        }
        crate::plugins::pcm::add_scheme_for_test("demo", "demo");
    }

    #[test]
    fn uris_find_their_plugin() {
        demo();
        assert_eq!(parse(Path::new("demo:tone:440")), Some(("demo".into(), "demo:tone:440".into())));
        // v1 lists named songs `plugin:<id>/<reference>`: the reference is the URI.
        assert_eq!(parse(Path::new("plugin:demo/demo:tone:440")), Some(("demo".into(), "demo:tone:440".into())));
        assert!(parse(Path::new("D:\\Music\\a.flac")).is_none());
        assert!(parse(Path::new("nobody:song:1")).is_none());
        assert_eq!(scheme("C:\\x"), None);
        assert_eq!(scheme("svc:song:1"), Some("svc"));
    }

    #[test]
    fn a_demo_link_becomes_a_playable_list() {
        demo();
        let acc = Accounts::default();
        let f = fetch(&acc, "demo", "demo:3").unwrap();
        assert_eq!((f.name.as_str(), f.songs.len(), f.unavailable), ("Demo list 3", 3, 0));
        let s = &f.songs[0];
        assert_eq!(s.tags.title.as_deref(), Some("Tone 1"));
        assert_eq!(s.tags.artist.as_deref(), Some("Demo"));
        assert!(s.probed && is_online(&s.path));
        let (plugin, uri) = parse(&s.path).unwrap();
        let r = resolve(&acc, &plugin, &uri).unwrap();
        assert!(Path::new(&r.url).is_file(), "the demo serves a local file: {}", r.url);
        assert_eq!(r.option.as_ref().map(|o| o.1.as_str()), Some("标准"));
        let values = options(&acc, &plugin, &uri).unwrap();
        assert_eq!(values.len(), 2);
        let full = complete(s).unwrap();
        assert_eq!(full.album.as_deref(), Some("Demo Album"));
        // A local song is never sent for metadata.
        assert!(complete(&Song::new(std::path::PathBuf::from("D:\\Music\\a.flac"))).is_none());
    }

    #[test]
    fn a_fallback_is_told_once_until_the_choice_changes() {
        let mut f = option::Fallback::default();
        // No choice made, or the song came in the chosen value: nothing to tell.
        assert!(!f.tell(1, "ncm", None, "lossless"));
        assert!(!f.tell(2, "ncm", Some("jymaster"), "jymaster"));
        // The first song below the choice is told; later ones, and the same open, are not.
        assert!(f.tell(3, "ncm", Some("jymaster"), "lossless"));
        assert!(!f.tell(3, "ncm", Some("jymaster"), "lossless"));
        assert!(!f.tell(4, "ncm", Some("jymaster"), "exhigh"));
        // Another plugin's choice is its own.
        assert!(f.tell(5, "qq", Some("flac"), "mp3"));
        assert!(!f.tell(6, "ncm", Some("jymaster"), "lossless"));
        // A value picked: told again.
        f.forget("ncm");
        assert!(f.tell(7, "ncm", Some("jymaster"), "lossless"));
    }

    #[test]
    fn the_res_line_follows_the_template() {
        let t = Some("{res} · {option}");
        assert_eq!(
            res_line(t, "48 kHz / 24-bit FLAC", "高清臻音"),
            ("48 kHz / 24-bit FLAC ·".into(), "高清臻音".into())
        );
        assert_eq!(res_line(t, "48 kHz / 24-bit FLAC", ""), ("48 kHz / 24-bit FLAC".into(), String::new()));
        assert_eq!(res_line(t, "", "无损"), (String::new(), "无损".into()));
        assert_eq!(res_line(None, "x", "y"), ("x".into(), String::new()));
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn a_big_png_cover_keeps_as_a_small_jpeg() {
        // A 2000 × 2000 PNG with detail (a photograph's master).
        let mut img = image::RgbaImage::new(2000, 2000);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 7 % 251) as u8, (y * 13 % 241) as u8, ((x ^ y) % 239) as u8, 255]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let small = cache::compact(&png).expect("smaller");
        let back = image::load_from_memory(&small).unwrap();
        assert_eq!((back.width(), back.height()), (1040, 1040));
        assert!(small.len() * 4 < png.len(), "{} vs {}", small.len(), png.len());
    }

    #[test]
    fn kept_faces_round_trip_compressed() {
        // Regression: a registry test's reload (set_stamps) in parallel cleared the stamp set
        // below, so an "other build" read as this one's now and then.
        let _by_hand = STAMPS_BY_HAND.locked();
        let dir = std::env::temp_dir().join(format!("np-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("LOCALAPPDATA", &dir);
        crate::plugins::pcm::add_scheme_for_test("cachet", "cachet");
        let path = std::path::PathBuf::from("cachet:song:1");
        let lrc: String = (0..200).map(|k| format!("[00:{:02}.00]一句歌词 line {k}\n", k % 60)).collect();
        let tags = Tags { title: Some("海屿你".into()), album: Some("专辑".into()), ..Default::default() };
        let tr = "[00:01.00]A line\n[00:02.00]Another\n".to_string();
        let f = TrackInfo {
            tags,
            cover: Some(vec![1, 2, 3]),
            lyrics: Some(lrc.clone()),
            layers: vec![Layer { id: "trans".into(), name: "翻译".into(), lrc: tr.clone() }],
        };
        cache::write(&path, &f);
        let g = cache::read(&path).expect("kept");
        assert_eq!(g.tags.title.as_deref(), Some("海屿你"));
        assert_eq!(g.lyrics.as_deref(), Some(lrc.as_str()));
        assert_eq!(
            g.layers,
            vec![Layer { id: "trans".into(), name: "翻译".into(), lrc: tr.clone() }],
            "the layer kept beside the lyrics"
        );
        // Another build of the plugin: the kept face is not trusted.
        STAMPS.locked().retain(|(p, _)| p != "cachet");
        STAMPS.locked().push(("cachet".into(), "otherbuild".into()));
        assert!(cache::read(&path).is_none(), "a plugin update asks again");
        STAMPS.locked().retain(|(p, _)| p != "cachet");
        assert!(cache::read(&path).is_some(), "the build it was kept for reads it");
        assert_eq!(g.cover, Some(vec![1, 2, 3]));
        let meta = cache::meta_file(&path).expect("kept on disk");
        // Deflated: the file is far smaller than its text, and holds no address.
        assert!(std::fs::metadata(meta).unwrap().len() * 3 < lrc.len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The songs of an album keep one picture (the library index's, by content), each names
    /// it; a row's thumbnail comes from the face kept; a face whose picture is gone is not
    /// trusted (its plugin is asked again).
    #[test]
    fn an_albums_online_songs_keep_one_picture() {
        let _by_hand = STAMPS_BY_HAND.locked();
        crate::plugins::pcm::add_scheme_for_test("cachea", "cachea");
        let mut png = Vec::new();
        image::RgbImage::from_fn(200, 200, |x, y| image::Rgb([x as u8, y as u8, 7]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let songs: Vec<std::path::PathBuf> = (1..=3).map(|k| format!("cachea:song:{k}").into()).collect();
        for p in &songs {
            let tags = Tags { title: Some(p.display().to_string()), ..Default::default() };
            cache::write(p, &TrackInfo { tags, cover: Some(png.clone()), lyrics: None, layers: vec![] });
        }
        let id = crate::library::disk_cache::key(&png);
        let faces = crate::library::index::face_of(&id).expect("the picture kept");
        assert_eq!(faces, png);
        for p in &songs {
            assert_eq!(cache::read(p).unwrap().cover.as_deref(), Some(&png[..]), "each names the one picture");
            let t = cache::read_thumb(p).expect("its row's thumbnail, from the face");
            assert_eq!(t.size(), (96, 96));
        }
        // The picture forgotten (the store's budget): the face is asked again, not shown bare.
        crate::library::index::forget_picture_for_test(&id);
        assert!(cache::read(&songs[0]).is_none());
    }
}
