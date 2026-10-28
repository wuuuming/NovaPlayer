//! Songs, lyrics, CUE sheets, M3U lists, folder scans and the colour field (spec, "Lyrics" and "Formats, scanning, CUE").
pub mod covers;
pub mod disk_cache;
pub mod lists;
pub mod queue;
pub mod row_cache;
pub mod store;

use crate::playback::engine::Span;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SOUND_EXT: [&str; 11] = ["mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "ape", "wv", "tta", "tak"];

/// Whether `p`'s extension is one of `exts`, in any case.
pub fn has_ext(p: &Path, exts: &[&str]) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// A dialog filter of extensions: `*.a;*.b;…`.
pub fn ext_filter(exts: impl IntoIterator<Item = impl std::fmt::Display>) -> String {
    exts.into_iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(";")
}

pub fn is_sound(p: &Path) -> bool {
    has_ext(p, &SOUND_EXT) || crate::plugins::pcm::decoder_for(p).is_some()
}
const LIST_EXT: [&str; 2] = ["m3u", "m3u8"];
const CUE_EXT: &str = "cue";

pub fn is_list(p: &Path) -> bool {
    has_ext(p, &LIST_EXT)
}

/// The open dialog's filter: every file the player opens (sound files, decode plugins' among
/// them, CUE sheets, lists), as `*.mp3;*.flac;…`.
pub fn open_filter() -> String {
    let exts = SOUND_EXT.iter().map(|e| e.to_string()).chain(crate::plugins::pcm::decoded_extensions());
    let exts = exts.chain(std::iter::once(CUE_EXT.to_owned())).chain(LIST_EXT.iter().map(|e| e.to_string()));
    ext_filter(exts)
}
pub fn is_cue(p: &Path) -> bool {
    has_ext(p, &[CUE_EXT])
}

/// What is known of a song without opening it (CUE, #EXTINF, or last read tags).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track: Option<String>,
    pub genre: Option<String>,
    pub date: Option<String>,
    pub duration: Option<Duration>,
}

/// The words of what a probe found.
impl From<&crate::player_core::MediaTags> for Tags {
    fn from(t: &crate::player_core::MediaTags) -> Self {
        Tags {
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            album_artist: t.album_artist.clone(),
            track: t.track.clone(),
            genre: t.genre.clone(),
            date: t.date.clone(),
            duration: t.duration,
        }
    }
}

/// A song's words as the face's tags: a sound that is there, nothing known of its stream.
impl From<&Tags> for crate::player_core::MediaTags {
    fn from(t: &Tags) -> Self {
        crate::player_core::MediaTags {
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            album_artist: t.album_artist.clone(),
            date: t.date.clone(),
            duration: t.duration,
            genre: t.genre.clone(),
            track: t.track.clone(),
            has_audio: true,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub path: PathBuf,
    pub span: Option<Span>,
    /// Display name before tags are read: the file stem (or the CUE title).
    pub name: String,
    pub tags: Tags,
    /// Tags were read from the file.
    pub probed: bool,
}

impl Song {
    pub fn new(path: PathBuf) -> Self {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Song { path, span: None, name, tags: Tags::default(), probed: false }
    }
    pub fn title(&self) -> &str {
        self.tags.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or(&self.name)
    }

    /// How long it plays, as far as is known without opening it: its span's (a CUE track's own),
    /// else its tags' length. The one order wherever a song's length is shown or sorted by.
    pub fn length(&self) -> Option<Duration> {
        self.span.and_then(|s| s.length()).or(self.tags.duration)
    }

    /// Its tags are still to be read from its file: not read yet, and not a CUE track (whose words
    /// come with its sheet).
    pub fn unread(&self) -> bool {
        !self.probed && self.span.is_none()
    }

    /// What its file says was read: it takes the file's tags unless it is a CUE track (its
    /// sheet's words stay), and it counts as read.
    pub fn took_tags(&mut self, tags: Option<Tags>) {
        if let (Some(t), None) = (tags, self.span) {
            self.tags = t;
        }
        self.probed = true;
    }

    /// Whether this is the song at `path` and `span`: the same sound (a CUE track is its span of
    /// the file). The one test of a song's identity: the list's rows, the song handed to the
    /// engine, the face.
    pub fn is(&self, path: &Path, span: Option<Span>) -> bool {
        self.path == path && self.span == span
    }

    /// Whether `other` is of this song's album: of one file (its CUE tracks, or the song
    /// itself), or with the same album and album artist (the artist where none is given).
    pub fn same_album(&self, other: &Song) -> bool {
        fn album(s: &Song) -> Option<(&str, Option<&str>)> {
            let name = s.tags.album.as_deref().map(str::trim).filter(|a| !a.is_empty())?;
            let by = s.tags.album_artist.as_deref().filter(|a| !a.trim().is_empty()).or(s.tags.artist.as_deref());
            Some((name, by.map(str::trim)))
        }
        self.path == other.path || album(self).is_some_and(|a| album(other) == Some(a))
    }
}

// ---- lyrics (PH/lyrics.rs) ------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub at: Duration,
    pub lines: Vec<String>,
    /// Enhanced LRC (`<mm:ss.xx>word`): when each word of the first line starts, as char ranges
    /// of that line (the marks themselves are not in the text). A stamp after the last word
    /// (`word <00:14.00>`) is kept as an empty range: when the line is fully sung. Empty for a
    /// line without word stamps.
    pub words: Vec<(Duration, std::ops::Range<usize>)>,
    /// Layer lines laid under it (a translation, a romanisation): (layer id, text), in
    /// `LAYER_ORDER`. Shown or hidden by the player, not part of `lines`.
    pub layers: Vec<(String, String)>,
}

/// The lyrics layers the player knows and draws (docs/extensions.md, "Lyrics layers"), in the order they sit
/// under a line: the romanisation (a reading) first, then the translation.
pub const LAYER_ORDER: [&str; 2] = ["roma", "trans"];

impl Group {
    /// When word `i` ends: at the next word's stamp; the last at `end` (the next group), else a
    /// second after it starts.
    fn word_end(&self, i: usize, end: Option<Duration>) -> Duration {
        let t0 = self.words[i].0;
        match self.words.get(i + 1) {
            Some(n) => n.0,
            None => end.filter(|e| *e > t0).unwrap_or(t0 + Duration::from_secs(1)),
        }
    }

    /// Chars of the first line sung at `pos` (fractional inside the word being sung), or `None`
    /// when the line has no word stamps. A word lasts until the next stamp; the last one until
    /// the trailing stamp, else `end` (the next group), else a second.
    pub fn sung(&self, pos: Duration, end: Option<Duration>) -> Option<f32> {
        let first = self.words.first()?;
        if pos < first.0 {
            return Some(first.1.start as f32);
        }
        let i = self.words.partition_point(|w| w.0 <= pos) - 1;
        let (t0, r) = &self.words[i];
        let t1 = self.word_end(i, end);
        let k = if t1 > *t0 { ((pos - *t0).as_secs_f32() / (t1 - *t0).as_secs_f32()).min(1.0) } else { 1.0 };
        Some(r.start as f32 + k * r.len() as f32)
    }

    /// When the first line's parts are sung, as spans (first char, from, to) in seconds, a span
    /// lasting to the next one's first char: one per stamped word (to the next stamp, the last
    /// as `sung` ends it); a line without word stamps is one span from its stamp to `end` (the
    /// next group), at most `UNSTAMPED_MAX`.
    pub fn timing(&self, end: Option<Duration>) -> Vec<(u32, f32, f32)> {
        const UNSTAMPED_MAX: f32 = 8.0;
        let at = self.at.as_secs_f32();
        if self.words.is_empty() {
            let end = end.map_or(at + 4.0, |e| e.as_secs_f32()).clamp(at, at + UNSTAMPED_MAX);
            return vec![(0, at, end)];
        }
        let mut out = Vec::with_capacity(self.words.len());
        for (i, (t0, r)) in self.words.iter().enumerate() {
            if r.is_empty() {
                continue;
            }
            let t1 = self.word_end(i, end);
            out.push((r.start as u32, t0.as_secs_f32(), t1.as_secs_f32()));
        }
        out
    }
}

/// A line's `<mm:ss.xx>` word stamps taken out: the text shown, and each word's start and char
/// range. Something in angle brackets that is not a stamp stays text.
fn split_words(line: &str) -> (String, Vec<(Duration, std::ops::Range<usize>)>) {
    if !line.contains('<') {
        return (line.to_owned(), Vec::new());
    }
    let (mut text, mut words) = (String::with_capacity(line.len()), Vec::new());
    let (mut chars, mut rest) = (0usize, line);
    let mut open: Option<(Duration, usize)> = None;
    while let Some(i) = rest.find('<') {
        let stamp = rest[i + 1..].split_once('>').and_then(|(tag, after)| parse_stamp(tag).map(|t| (t, after)));
        let Some((t, after)) = stamp else {
            let lit = &rest[..=i];
            chars += lit.chars().count();
            text.push_str(lit);
            rest = &rest[i + 1..];
            continue;
        };
        let lit = &rest[..i];
        chars += lit.chars().count();
        text.push_str(lit);
        if let Some((t0, a)) = open.take() {
            words.push((t0, a..chars));
        }
        open = Some((t, chars));
        rest = after;
    }
    chars += rest.chars().count();
    text.push_str(rest);
    if let Some((t0, a)) = open {
        words.push((t0, a..chars));
    }
    // Leading/trailing blanks are trimmed from the shown line: ranges follow.
    let lead = text.chars().take_while(|c| c.is_whitespace()).count();
    let shown: String = text.trim().to_owned();
    let n = shown.chars().count();
    for (_, r) in &mut words {
        *r = r.start.saturating_sub(lead).min(n)..r.end.saturating_sub(lead).min(n);
    }
    (shown, words)
}

/// A layer of lines under the lyrics (docs/extensions.md, "Lyrics layers"), as a plugin gave it: its id
/// (`trans`, `roma`), its name, and an LRC of the lyrics' stamps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    pub id: String,
    pub name: String,
    pub lrc: String,
}

/// The song's plugin and the layers its lyrics come with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricLayers {
    pub plugin: String,
    pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lyrics {
    pub groups: Vec<Group>,
}

fn is_header(tag: &str) -> bool {
    tag.split_once(':').is_some_and(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphabetic()))
}

fn parse_stamp(tag: &str) -> Option<Duration> {
    let (m, rest) = tag.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let (s, f) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (rest, ""),
    };
    let s: u64 = s.trim().parse().ok().filter(|s| *s < 60)?;
    let f = f.trim();
    if !f.chars().all(|d| d.is_ascii_digit()) || f.len() > 3 {
        return None;
    }
    let ms = match f.len() {
        0 => 0,
        n => f.parse::<u64>().ok()? * 10u64.pow(3 - n as u32),
    };
    Some(Duration::from_millis((m * 60 + s) * 1000 + ms))
}

impl Lyrics {
    pub fn parse(text: &str) -> Option<Self> {
        let mut offset = 0i64;
        /// A timed line as read: its time, its text, its words' times (with their bytes in it).
        type Stamped = (Duration, String, Vec<(Duration, std::ops::Range<usize>)>);
        let mut stamped: Vec<Stamped> = Vec::new();
        for line in text.trim_start_matches('\u{FEFF}').lines() {
            let mut rest = line.trim();
            let mut times = Vec::new();
            while let Some((tag, after)) = rest.strip_prefix('[').and_then(|i| i.split_once(']')) {
                match parse_stamp(tag) {
                    Some(t) => times.push(t),
                    None => {
                        if let Some(v) = tag.strip_prefix("offset:") {
                            offset = v.trim().parse().unwrap_or(0);
                        } else if !is_header(tag) {
                            break;
                        }
                    }
                }
                rest = after.trim_start();
            }
            let (shown, words) = split_words(rest.trim());
            for t in times {
                stamped.push((t, shown.clone(), words.clone()));
            }
        }
        if stamped.is_empty() {
            return None;
        }
        let shift = |t: &mut Duration| {
            let s = i64::try_from(t.as_millis()).unwrap_or(i64::MAX).saturating_sub(offset);
            *t = Duration::from_millis(u64::try_from(s).unwrap_or(0));
        };
        for (t, _, words) in &mut stamped {
            shift(t);
            words.iter_mut().for_each(|(w, _)| shift(w));
            // Word stamps out of order are not karaoke: the line shows whole-line lit.
            if words.windows(2).any(|p| p[1].0 < p[0].0) {
                words.clear();
            }
        }
        stamped.sort_by_key(|(t, ..)| *t);
        let mut groups: Vec<Group> = Vec::new();
        for (at, w, words) in stamped {
            match groups.last_mut() {
                Some(g) if g.at == at => {
                    if !w.is_empty() {
                        if g.lines.is_empty() {
                            g.words = words;
                        }
                        g.lines.push(w);
                    }
                }
                _ => {
                    let (lines, words) = if w.is_empty() { (Vec::new(), Vec::new()) } else { (vec![w], words) };
                    groups.push(Group { at, lines, words, layers: Vec::new() });
                }
            }
        }
        Some(Lyrics { groups })
    }

    /// The layers (a plugin's `layers`) laid under the groups: a layer's
    /// line goes under the group of the same stamp (within a frame's rounding, 50 ms). Layers the
    /// player does not know, and lines with no group, are left out.
    pub fn with_layers(mut self, layers: &[Layer]) -> Self {
        for id in LAYER_ORDER {
            let Some(l) = layers.iter().find(|l| l.id == id) else { continue };
            let Some(layer) = Lyrics::parse(&l.lrc) else { continue };
            for lg in layer.groups {
                let Some(text) = lg.lines.first().filter(|t| !t.trim().is_empty()) else { continue };
                let i = self.groups.partition_point(|g| g.at <= lg.at + Duration::from_millis(50));
                let Some(g) = i.checked_sub(1).map(|i| &mut self.groups[i]) else { continue };
                if lg.at.abs_diff(g.at) <= Duration::from_millis(50)
                    && !g.lines.is_empty()
                    && !g.layers.iter().any(|(l, _)| l == id)
                {
                    g.layers.push((id.to_owned(), text.clone()));
                }
            }
        }
        self
    }

    /// Every line is stamped word by word (enhanced LRC): the lyrics can be sung word by word.
    /// Lines without text (a break) need no stamps.
    pub fn word_timed(&self) -> bool {
        let mut lines = self.groups.iter().filter(|g| !g.lines.is_empty()).peekable();
        lines.peek().is_some() && lines.all(|g| !g.words.is_empty())
    }

    /// The layer ids some group carries, in `LAYER_ORDER`.
    pub fn layer_ids(&self) -> Vec<&'static str> {
        LAYER_ORDER.into_iter().filter(|id| self.groups.iter().any(|g| g.layers.iter().any(|(l, _)| l == id))).collect()
    }

    pub fn current(&self, pos: Duration) -> Option<usize> {
        self.groups.partition_point(|g| g.at <= pos).checked_sub(1)
    }

    pub fn until_next(&self, pos: Duration) -> Option<Duration> {
        let n = self.groups.partition_point(|g| g.at <= pos);
        self.groups.get(n).map(|g| g.at.saturating_sub(pos))
    }
}

pub fn sidecar(media: &Path) -> Option<Lyrics> {
    let bytes = std::fs::read(media.with_extension("lrc")).ok()?;
    Lyrics::parse(&crate::player_core::legacy_text::decode_guessing(&bytes))
}

// ---- CUE (PH/cue.rs) ------------------------------------------------------------------------

pub fn parse_cue_time(text: &str) -> Option<Duration> {
    let mut p = text.trim().split(':').map(|x| x.trim().parse::<u64>().ok());
    let (m, s, f) = (p.next()??, p.next()??, p.next()??);
    if p.next().is_some() || s >= 60 || f >= 75 {
        return None;
    }
    let frames = (m * 60 + s) * 75 + f;
    Some(Duration::from_nanos(frames * 1_000_000_000 / 75))
}

fn cue_value(text: &str) -> Option<String> {
    let t = text.trim();
    let t = match t.strip_prefix('"') {
        Some(r) => r.rfind('"').map_or(r, |e| &r[..e]),
        None => t,
    };
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

fn keyword(line: &str) -> (&str, &str) {
    let l = line.trim();
    match l.find(char::is_whitespace) {
        Some(a) => (&l[..a], l[a..].trim_start()),
        None => (l, ""),
    }
}

/// CUE tracks saved with text a wrong code page made (U+FFFD in it): read again from the sheets
/// beside their files, matched by file and start. Lists saved before the Linux decoder knew
/// GBK / Shift-JIS hold such text.
pub fn repair_cue_text(songs: &mut [std::sync::Arc<Song>]) -> bool {
    let bad = |s: &Song| {
        s.span.is_some()
            && [Some(&s.name), s.tags.title.as_ref(), s.tags.artist.as_ref(), s.tags.album.as_ref()]
                .into_iter()
                .flatten()
                .any(|t| t.contains('\u{FFFD}'))
    };
    let dirs: std::collections::BTreeSet<PathBuf> =
        songs.iter().filter(|s| bad(s)).filter_map(|s| s.path.parent().map(Path::to_path_buf)).collect();
    let mut fresh: std::collections::HashMap<(PathBuf, Duration), Song> = std::collections::HashMap::new();
    for d in dirs {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if is_cue(&e.path()) {
                for s in read_cue(&e.path()) {
                    if let Some(sp) = s.span {
                        fresh.insert((s.path.clone(), sp.start), s);
                    }
                }
            }
        }
    }
    let mut changed = false;
    // A file's own tags read wrong (a WAV's INFO in Shift-JIS / EUC-JP): read them again.
    for s in songs.iter_mut().filter(|s| s.span.is_none() && s.probed) {
        let t = &s.tags;
        if [t.title.as_ref(), t.artist.as_ref(), t.album.as_ref()].into_iter().flatten().any(|x| x.contains('\u{FFFD}'))
        {
            std::sync::Arc::make_mut(s).probed = false;
            changed = true;
        }
    }
    for s in songs.iter_mut().filter(|s| bad(s)) {
        if let Some(f) = s.span.and_then(|sp| fresh.get(&(s.path.clone(), sp.start))) {
            let s = std::sync::Arc::make_mut(s);
            s.name = f.name.clone();
            s.tags = f.tags.clone();
            changed = true;
        }
    }
    changed
}

/// The songs of a CUE sheet at `path` (each a span of its file).
pub fn read_cue(path: &Path) -> Vec<Song> {
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    if bytes.len() > 1 << 20 {
        return Vec::new();
    }
    let text = crate::player_core::legacy_text::decode_guessing(&bytes);
    let dir = path.parent().unwrap_or(Path::new("."));
    let (mut album, mut performer, mut genre, mut date) = (None, None, None, None);
    struct T {
        file: usize,
        title: Option<String>,
        performer: Option<String>,
        number: u32,
        start: Option<Duration>,
        audio: bool,
    }
    let mut files: Vec<PathBuf> = Vec::new();
    let mut tracks: Vec<T> = Vec::new();
    for line in text.lines() {
        let (cmd, rest) = keyword(line);
        match cmd.to_ascii_uppercase().as_str() {
            "REM" => {
                let (k, r) = keyword(rest);
                if tracks.is_empty() {
                    match k.to_ascii_uppercase().as_str() {
                        "GENRE" => genre = cue_value(r),
                        "DATE" => date = cue_value(r),
                        _ => {}
                    }
                }
            }
            "TITLE" => match tracks.last_mut() {
                Some(t) => t.title = cue_value(rest),
                None => album = cue_value(rest),
            },
            "PERFORMER" => match tracks.last_mut() {
                Some(t) => t.performer = cue_value(rest),
                None => performer = cue_value(rest),
            },
            "FILE" => {
                let r = rest.trim();
                let name = if let Some(inner) = r.strip_prefix('"') {
                    inner.rfind('"').map(|e| inner[..e].to_owned())
                } else {
                    r.rsplit_once(char::is_whitespace).map(|(a, _)| a.trim().to_owned()).or(Some(r.to_owned()))
                };
                if let Some(n) = name.filter(|n| !n.is_empty()) {
                    files.push(resolve_cue_file(dir, &n, path));
                }
            }
            "TRACK" => {
                let (n, kind) = keyword(rest);
                if !files.is_empty() {
                    tracks.push(T {
                        file: files.len() - 1,
                        title: None,
                        performer: None,
                        number: n.parse().unwrap_or(0),
                        start: None,
                        audio: kind.trim().eq_ignore_ascii_case("AUDIO"),
                    });
                }
            }
            "INDEX" => {
                let (n, t) = keyword(rest);
                if n.parse::<u32>().ok() == Some(1) {
                    if let Some(tr) = tracks.last_mut() {
                        tr.start = tr.start.or(parse_cue_time(t));
                    }
                }
            }
            _ => {}
        }
    }
    // An audio track with its INDEX 01, paired with where it starts.
    let tracks: Vec<(T, _)> =
        tracks.into_iter().filter(|t| t.audio).filter_map(|t| t.start.map(|start| (t, start))).collect();
    let mut out = Vec::new();
    for (i, (t, start)) in tracks.iter().enumerate() {
        let start = *start;
        let end = tracks.get(i + 1).filter(|(n, _)| n.file == t.file).map(|&(_, at)| at);
        let file = files[t.file].clone();
        let mut s = Song::new(file);
        s.span = Some(Span { start, end });
        s.name = t.title.clone().unwrap_or_else(|| format!("Track {:02}", t.number));
        s.tags = Tags {
            title: t.title.clone(),
            artist: t.performer.clone().or(performer.clone()),
            album: album.clone(),
            album_artist: performer.clone(),
            track: Some(t.number.to_string()),
            genre: genre.clone(),
            date: date.clone(),
            duration: end.map(|e| e.saturating_sub(start)),
        };
        out.push(s);
    }
    out
}

fn resolve_cue_file(dir: &Path, name: &str, sheet: &Path) -> PathBuf {
    let p = dir.join(name);
    if p.exists() {
        return p;
    }
    // Ripped as .wav, compressed since: the same stem with another audio extension.
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    for st in [stem, sheet.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()] {
        for e in SOUND_EXT {
            let c = dir.join(format!("{st}.{e}"));
            if c.exists() {
                return c;
            }
        }
    }
    p
}

// ---- M3U (PH/m3u.rs) ------------------------------------------------------------------------

/// A parsed M3U8: its `#PLAYLIST` name, its songs, the count of local entries not found.
pub struct M3u {
    pub name: Option<String>,
    pub songs: Vec<Song>,
    pub missing: usize,
}

/// Reads M3U8 text (the extended form): `#EXTINF:<secs>,<artist> - <title>`, `#EXTALB:`,
/// `#EXTART:`, `#PLAYLIST:`. An entry is a local path (relative to `dir`), a `file:///` URL, or
/// a URI an enabled plugin resolves (`svc:song:1`); http(s) entries are played as they are.
pub fn parse_m3u8(text: &str, dir: Option<&Path>) -> M3u {
    let dir = dir.unwrap_or(Path::new("."));
    let (mut songs, mut missing, mut name) = (Vec::new(), 0, None);
    let mut info: Option<(Option<Duration>, String)> = None;
    let (mut album, mut artist): (Option<String>, Option<String>) = (None, None);
    for line in text.trim_start_matches('\u{FEFF}').lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(r) = l.strip_prefix("#EXTINF:") {
            let (len, n) = r.split_once(',').unwrap_or((r, ""));
            let secs: Option<f64> = len.split_whitespace().next().and_then(|v| v.parse().ok());
            info = Some((secs.filter(|s| *s > 0.0).map(Duration::from_secs_f64), n.trim().to_owned()));
            continue;
        }
        if let Some(r) = l.strip_prefix("#PLAYLIST:") {
            name = Some(r.trim().to_owned()).filter(|n| !n.is_empty());
            continue;
        }
        if let Some(r) = l.strip_prefix("#EXTALB:") {
            album = Some(r.trim().to_owned()).filter(|n| !n.is_empty());
            continue;
        }
        if let Some(r) = l.strip_prefix("#EXTART:") {
            artist = Some(r.trim().to_owned()).filter(|n| !n.is_empty());
            continue;
        }
        if l.starts_with('#') {
            continue;
        }
        let (alb, art) = (album.take(), artist.take());
        let path = if let Some(f) = l.strip_prefix("file:///") {
            let p = PathBuf::from(f.replace("%20", " "));
            if !p.exists() {
                missing += 1;
                info = None;
                continue;
            }
            p
        } else if crate::files::is_web_address(l) || crate::plugins::client::parse(Path::new(l)).is_some() {
            PathBuf::from(l)
        } else {
            let p = Path::new(l);
            let p = if p.is_absolute() { p.to_path_buf() } else { dir.join(p) };
            if !p.exists() {
                missing += 1;
                info = None;
                continue;
            }
            p
        };
        let mut s = Song::new(path);
        if let Some((d, n)) = info.take() {
            s.tags.duration = d;
            if let Some((a, t)) = n.split_once(" - ") {
                s.tags.artist = Some(a.trim().to_owned());
                s.tags.title = Some(t.trim().to_owned());
            } else if !n.is_empty() {
                s.tags.title = Some(n);
            }
        }
        if art.is_some() {
            s.tags.artist = art;
        }
        s.tags.album = alb;
        songs.push(s);
    }
    M3u { name, songs, missing }
}

/// (songs, count of lines whose file was not found)
pub fn read_m3u(path: &Path) -> (Vec<Song>, usize) {
    let Ok(bytes) = std::fs::read(path) else { return (Vec::new(), 0) };
    if bytes.len() > 64 << 20 {
        return (Vec::new(), 0);
    }
    let text =
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("m3u8")) || std::str::from_utf8(&bytes).is_ok() {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            crate::player_core::legacy_text::decode_ansi(&bytes)
        };
    let m = parse_m3u8(&text, path.parent());
    (m.songs, m.missing)
}

pub fn write_m3u8(path: &Path, songs: &[&Song]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut s = String::from("#EXTM3U\n");
    for song in songs {
        let secs = song.tags.duration.map_or(-1, |d| d.as_secs() as i64);
        let name = match &song.tags.artist {
            Some(a) => format!("{a} - {}", song.title()),
            None => song.title().to_owned(),
        };
        s.push_str(&format!("#EXTINF:{secs},{name}\n"));
        let rel = song.path.strip_prefix(dir).map(Path::to_path_buf).unwrap_or_else(|_| song.path.clone());
        s.push_str(&rel.to_string_lossy());
        s.push('\n');
    }
    crate::files::write_atomic(path, s.as_bytes())
}

// ---- scanning -------------------------------------------------------------------------------

fn housekeeping(name: &str) -> bool {
    name.starts_with(['$', '@', '#', '.']) || name.eq_ignore_ascii_case("System Volume Information")
}

/// File-name order (case-insensitive, then exact), as folder scans sort.
pub fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b))
}

/// Paths in the order of their file names (`name_order`); the same name in two folders (CUE
/// files pointing into subfolders) by the whole path. The one order of every folder scan and
/// of a folder's pictures.
pub fn by_name(a: &Path, b: &Path) -> std::cmp::Ordering {
    let n = |p: &Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
    name_order(&n(a), &n(b)).then_with(|| a.cmp(b))
}

/// Everything playable under `root`, in name order; a CUE beside its audio replaces the file by
/// its tracks. `cancel` is polled between directories.
pub fn scan(root: &Path, cancel: &dyn Fn() -> bool) -> Vec<Song> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel() {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if housekeeping(&name) {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => dirs.push(p),
                Ok(_) => files.push(p),
                _ => {}
            }
        }
        files.sort_by(|a, b| by_name(a, b));
        dirs.sort_by(|a, b| by_name(b, a));
        let mut covered = std::collections::HashSet::new();
        let mut here = Vec::new();
        for f in files.iter().filter(|f| is_cue(f)) {
            let songs = read_cue(f);
            for s in &songs {
                covered.insert(s.path.clone());
            }
            here.extend(songs);
        }
        for f in files.iter().filter(|f| is_sound(f) && !covered.contains(*f)) {
            here.push(Song::new(f.clone()));
        }
        here.sort_by(|a, b| by_name(&a.path, &b.path).then(a.span.map(|s| s.start).cmp(&b.span.map(|s| s.start))));
        out.extend(here);
        stack.extend(dirs);
    }
    out
}

/// Reads the tags of `songs` (index, path) on a few threads at once, giving each result to
/// `got` as it comes; stops early when `cancel` says so. A file read once is not read again: the
/// tags are kept on the list's songs (and saved with it).
pub fn read_tags(
    songs: Vec<(usize, PathBuf)>,
    cancel: &(dyn Fn() -> bool + Sync),
    got: &(dyn Fn(usize, Option<Tags>) + Sync),
) {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((i, p)) = songs.get(k) else { break };
                if cancel() {
                    break;
                }
                let t = crate::playback::engine::probe_tags(p).as_ref().map(Tags::from);
                got(*i, t);
            });
        }
    });
}

/// One look at each of a list's files (by index), off the UI thread (on a network drive every
/// look is a round trip): (changed, gone). Changed: modified after `saved`, what a rescan reads
/// again; without a save time every file that exists counts. Gone: the file is not there.
/// A file that cannot be looked at for another reason (a drive that stopped answering) is
/// neither.
pub fn changed_since<'a>(
    paths: impl Iterator<Item = &'a Path>,
    saved: Option<std::time::SystemTime>,
) -> (Vec<usize>, Vec<usize>) {
    let (mut changed, mut gone) = (Vec::new(), Vec::new());
    for (i, p) in paths.enumerate() {
        match std::fs::metadata(p) {
            Ok(m) => {
                let newer = match (m.modified().ok(), saved) {
                    (Some(m), Some(s)) => m > s,
                    (Some(_), None) => true,
                    _ => false,
                };
                if newer {
                    changed.push(i);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => gone.push(i),
            Err(_) => {}
        }
    }
    (changed, gone)
}

/// Songs for a dropped / picked path: a folder (scanned), a list, a CUE, or a sound file.
pub fn songs_for(p: &Path, cancel: &dyn Fn() -> bool) -> (Vec<Song>, usize) {
    if p.is_dir() {
        (scan(p, cancel), 0)
    } else if is_list(p) {
        read_m3u(p)
    } else if is_cue(p) {
        (read_cue(p), 0)
    } else if is_sound(p) {
        (vec![Song::new(p.to_path_buf())], 0)
    } else {
        (Vec::new(), 0)
    }
}

// ---- cover and colour field (PH/sound_face.rs colour_field) ----------------------------------

pub const FIELD: usize = 6;
const FIELD_MAX_LUMA: f32 = 104.0;
const FIELD_SATURATION: f32 = 1.35;

#[derive(Clone)]
pub struct Cover {
    /// Premultiplied RGBA8.
    pub image: novaui::ImageHandle,
    /// The 6×6 field, opaque RGBA8.
    pub field: novaui::ImageHandle,
}

/// A premultiplied pixel's colour, straight.
fn straight(p: &[u8]) -> [u8; 3] {
    let a = u32::from(p[3]);
    match a {
        255 => [p[0], p[1], p[2]],
        0 => [0; 3],
        _ => [0, 1, 2].map(|i| ((u32::from(p[i]) * 255 + a / 2) / a).min(255) as u8),
    }
}

fn vividness(p: &[u8]) -> f32 {
    let lo = p[..3].iter().min().copied().unwrap_or(0);
    let hi = p[..3].iter().max().copied().unwrap_or(0);
    if hi == 0 {
        0.0
    } else {
        f32::from(hi - lo) / f32::from(hi)
    }
}

fn saturate([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    [r, g, b].map(|c| (l + (c - l) * FIELD_SATURATION).clamp(0.0, 255.0))
}

fn tone([r, g, b]: [f32; 3]) -> [u8; 3] {
    let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let k = if l > FIELD_MAX_LUMA { FIELD_MAX_LUMA / l } else { 1.0 };
    [(r * k) as u8, (g * k) as u8, (b * k) as u8]
}

/// The colour lattice of a cover (premultiplied RGBA8 pixels, `w`×`h`).
pub fn colour_field(px: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut means = [[0.0f32; 3]; FIELD * FIELD];
    for row in 0..FIELD {
        for col in 0..FIELD {
            let (x0, x1) = (col * w / FIELD, ((col + 1) * w / FIELD).max(col * w / FIELD + 1));
            let (y0, y1) = (row * h / FIELD, ((row + 1) * h / FIELD).max(row * h / FIELD + 1));
            let (sx, sy) = (((x1 - x0) / 16).max(1), ((y1 - y0) / 16).max(1));
            let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
            for y in (y0..y1.min(h)).step_by(sy) {
                for x in (x0..x1.min(w)).step_by(sx) {
                    let p = straight(&px[(y * w + x) * 4..][..4]);
                    let wt = (vividness(&p) + 0.08).powi(2);
                    for (t, c) in sum.iter_mut().zip(&p) {
                        *t += f32::from(*c) * wt;
                    }
                    n += wt;
                }
            }
            if n > 0.0 {
                means[row * FIELD + col] = sum.map(|t| t / n);
            }
        }
    }
    let mut out = vec![255u8; FIELD * FIELD * 4];
    for row in 0..FIELD {
        for col in 0..FIELD {
            let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
            for r in row.saturating_sub(1)..=(row + 1).min(FIELD - 1) {
                for c in col.saturating_sub(1)..=(col + 1).min(FIELD - 1) {
                    if (r, c) == (row, col) {
                        continue;
                    }
                    for (t, v) in sum.iter_mut().zip(means[r * FIELD + c]) {
                        *t += v;
                    }
                    n += 1.0;
                }
            }
            let own = means[row * FIELD + col];
            let mixed = [0, 1, 2].map(|i| own[i] * 0.5 + sum[i] / n.max(1.0) * 0.5);
            let [r, g, b] = tone(saturate(mixed));
            out[(row * FIELD + col) * 4..][..3].copy_from_slice(&[r, g, b]);
        }
    }
    out
}

/// A neutral field for songs without a cover.
pub fn default_field() -> novaui::ImageHandle {
    let mut out = vec![255u8; FIELD * FIELD * 4];
    for row in 0..FIELD {
        for col in 0..FIELD {
            let t = (row + col) as f32 / (2 * FIELD - 2) as f32;
            let c = [34.0 + 14.0 * (1.0 - t), 38.0 + 10.0 * t, 52.0 + 18.0 * (1.0 - t)];
            out[(row * FIELD + col) * 4..][..3].copy_from_slice(&c.map(|v| v as u8));
        }
    }
    novaui::ImageHandle::rgba8(FIELD as i32, FIELD as i32, out)
}

/// Decodes a cover, keeps it ≤ `max_side`, premultiplies it, and computes its field.
pub fn decode_cover(bytes: &[u8], max_side: u32) -> Option<Cover> {
    let image = novaui::image::decode(bytes, novaui::image::Fit::Within(max_side))?;
    let (w, h) = image.size();
    let field = colour_field(image.pixels(), w as usize, h as usize);
    Some(Cover { image, field: novaui::ImageHandle::rgba8(FIELD as i32, FIELD as i32, field) })
}

/// A square thumbnail (centred crop), premultiplied.
pub fn thumb(bytes: &[u8], side: u32) -> Option<novaui::ImageHandle> {
    novaui::image::decode(bytes, novaui::image::Fit::Square(side))
}

// ---- song info formatting (PH/song_info.rs) --------------------------------------------------

pub const MISSING: &str = "-";

pub fn clock(t: Duration) -> String {
    let s = t.as_secs();
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", s / 60 % 60, s % 60),
    }
}

pub fn track_text(w: &str) -> String {
    match w.split_once('/') {
        Some((n, c)) if !c.trim().is_empty() => format!("{} / {}", n.trim(), c.trim()),
        Some((n, _)) => n.trim().to_owned(),
        None => w.trim().to_owned(),
    }
}

pub fn year(date: &str) -> Option<&str> {
    let b = date.as_bytes();
    (0..b.len().saturating_sub(3))
        .find(|&s| {
            b[s..s + 4].iter().all(u8::is_ascii_digit)
                && b.get(s + 4).is_none_or(|n| !n.is_ascii_digit())
                && (s == 0 || !b[s - 1].is_ascii_digit())
        })
        .map(|s| &date[s..s + 4])
}

pub fn codec(name: &str) -> String {
    match name {
        "" => MISSING.to_owned(),
        c if c.starts_with("pcm_") => "PCM".to_owned(),
        "vorbis" => "Vorbis".to_owned(),
        "opus" => "Opus".to_owned(),
        "wavpack" => "WavPack".to_owned(),
        "mp3float" | "mp3" => "MP3".to_owned(),
        c => c.to_ascii_uppercase(),
    }
}

pub fn rate_depth(rate: Option<u32>, depth: Option<u8>) -> String {
    if rate.is_none() && depth.is_none() {
        return MISSING.to_owned();
    }
    let r = rate.map_or_else(
        || MISSING.to_owned(),
        |r| if r % 1000 == 0 { format!("{} kHz", r / 1000) } else { format!("{:.1} kHz", r as f64 / 1000.0) },
    );
    let d = depth.map_or_else(|| MISSING.to_owned(), |b| format!("{b} bit"));
    format!("{r} / {d}")
}

pub fn size_text(bytes: u64) -> String {
    const U: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return novaui::i18n::t("size.bytes", &[("n", &bytes.to_string())]);
    }
    let (mut v, mut u) = (bytes as f64 / 1024.0, 0);
    while v >= 1024.0 && u + 1 < U.len() {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", U[u])
}

pub fn kind_text(ext: Option<&str>, container: Option<&str>) -> String {
    let c = container.map(|n| match n {
        "mov" => "MPEG-4",
        "mp3" => "MP3",
        "flac" => "FLAC",
        "ogg" => "Ogg",
        "wav" => "WAV",
        "aiff" => "AIFF",
        "matroska" => "Matroska",
        "ape" => "APE",
        "wv" => "WavPack",
        "asf" => "ASF",
        "dsf" => "DSF",
        o => o,
    });
    let e = ext.filter(|e| !e.is_empty()).map(str::to_ascii_uppercase);
    match (e, c) {
        (Some(e), Some(c)) if !e.eq_ignore_ascii_case(c) => {
            novaui::i18n::t("kind.in", &[("ext", &e), ("container", c)])
        }
        (Some(e), _) => e,
        (None, Some(c)) => c.to_owned(),
        (None, None) => MISSING.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One order for a song's length (it was tags first here, span first on the info page): a
    /// span is the song's own.
    #[test]
    fn a_cue_track_lasts_its_span() {
        let mut song = Song::new(PathBuf::from("cd.flac"));
        song.tags.duration = Some(Duration::from_secs(300));
        song.span = Some(Span { start: Duration::from_secs(60), end: Some(Duration::from_secs(120)) });
        assert_eq!(song.length(), Some(Duration::from_secs(60)));
    }

    /// One name order for folder scans and a folder's pictures: case-insensitive, then exact,
    /// then the whole path (it was the whole path before the exact name in scans).
    #[test]
    fn files_sort_by_name_then_exact_name_then_path() {
        let mut paths: Vec<PathBuf> =
            ["a/b.flac", "z/B.flac", "a/c.flac", "y/A.flac"].iter().map(PathBuf::from).collect();
        paths.sort_by(|a, b| by_name(a, b));
        let want: Vec<PathBuf> = ["y/A.flac", "z/B.flac", "a/b.flac", "a/c.flac"].iter().map(PathBuf::from).collect();
        assert_eq!(paths, want);
    }

    #[test]
    fn artwork_cache_shares_content_not_album_metadata() {
        let image =
            image::RgbaImage::from_fn(40, 24, |x, y| image::Rgba([19 + x as u8 * 2, 40 + y as u8 * 3, 80, 255]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image).write_to(&mut encoded, image::ImageFormat::Png).unwrap();
        let bytes = encoded.into_inner();
        let a = thumb(&bytes, 96).unwrap();
        let b = thumb(&bytes.clone(), 96).unwrap();
        assert!(a == b);
        let smaller = thumb(&bytes, 64).unwrap();
        assert!(a != smaller);
        let cover = decode_cover(&bytes, 1040).unwrap();
        let same_cover = decode_cover(&bytes, 1040).unwrap();
        assert!(cover.image == same_cover.image);
        assert!(cover.field == same_cover.field);
        assert!(cover.image != a);
        let weak = cover.image.downgrade();
        drop(cover);
        drop(same_cover);
        assert!(weak.upgrade().is_none(), "decode cache must not retain unused high-resolution pixels");
    }
    #[test]
    fn lrc_groups_and_offset() {
        let l = Lyrics::parse(
            "\u{FEFF}[ar:x]\n[00:01.50]a\n[00:01.50]reading\n[00:03]b\n[offset:500]\n[00:05.123][00:07]c\n[00:08]\n",
        )
        .unwrap();
        assert_eq!(l.groups.len(), 5);
        assert_eq!(l.groups[0].at, Duration::from_millis(1000));
        assert_eq!(l.groups[0].lines, vec!["a", "reading"]);
        assert!(l.groups[4].lines.is_empty());
        assert_eq!(l.current(Duration::from_millis(2600)), Some(1));
        assert_eq!(l.until_next(Duration::from_millis(2000)), Some(Duration::from_millis(500)));
    }
    #[test]
    fn cue_time() {
        assert_eq!(parse_cue_time("01:02:37"), Some(Duration::from_nanos((62 * 75 + 37) * 1_000_000_000 / 75)));
        assert_eq!(parse_cue_time("00:60:00"), None);
    }
    #[test]
    fn formats() {
        crate::app::speak_zh_cn();
        assert_eq!(clock(Duration::from_secs(3725)), "1:02:05");
        assert_eq!(track_text("3/12"), "3 / 12");
        assert_eq!(year("2021-05-01T00"), Some("2021"));
        assert_eq!(rate_depth(Some(44100), Some(16)), "44.1 kHz / 16 bit");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(kind_text(Some("m4a"), Some("mov")), "M4A（MPEG-4）");
    }
}

#[cfg(test)]
mod rescan_tests {
    use super::*;
    #[test]
    fn rescan_reads_only_changed_files() {
        let d = std::env::temp_dir().join(format!("np-rescan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("a.flac"), d.join("b.flac"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let saved = std::time::SystemTime::now();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(&b, b"b2").unwrap();
        let gone = d.join("gone.flac");
        let paths = [a.as_path(), b.as_path(), gone.as_path()];
        assert_eq!(changed_since(paths.iter().copied(), Some(saved)).0, vec![1]);
        assert_eq!(changed_since(paths.iter().copied(), None).0, vec![0, 1]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn layers_sit_under_their_lines_in_order() {
        let l = Lyrics::parse("[00:01.00]夜に駆ける\n[00:03.50]沈むように\n").unwrap();
        let layer = |id: &str, lrc: &str| Layer { id: id.into(), name: String::new(), lrc: lrc.into() };
        let layers = [
            layer("trans", "[00:01.00]向夜晚奔去\n[00:09.00]无处可去\n"),
            layer("roma", "[00:01.02]yoru ni kakeru\n"),
            layer("other", "[00:03.50]x\n"),
        ];
        let l = l.with_layers(&layers);
        assert_eq!(l.groups[0].lines, vec!["夜に駆ける".to_string()]);
        let ids: Vec<&str> = l.groups[0].layers.iter().map(|(i, _)| i.as_str()).collect();
        assert_eq!(ids, ["roma", "trans"]);
        assert!(l.groups[1].layers.is_empty(), "no line of a known layer at 3.5 s");
        assert_eq!(l.layer_ids(), ["roma", "trans"]);
    }

    #[test]
    fn translation_under_each_line() {
        let l = "[00:01.00]夜に駆ける\n[00:03.50]沈むように\n";
        let t = "[00:01.00]向夜晚奔去\n[00:03.50]像是沉没一般\n";
        let both = Lyrics::parse(&format!("{l}\n{t}")).unwrap();
        assert_eq!(both.groups.len(), 2);
        assert_eq!(both.groups[0].lines, vec!["夜に駆ける".to_string(), "向夜晚奔去".to_string()]);
        assert_eq!(both.groups[1].lines, vec!["沈むように".to_string(), "像是沉没一般".to_string()]);
    }
}
