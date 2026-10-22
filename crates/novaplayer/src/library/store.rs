//! Playlists kept between runs (spec, "Playlists"): `index.txt` + `p<id>.txt`, a text format
//! (`NVPLAYLISTS 1` / `NVPLAYLIST 1`), in `%LOCALAPPDATA%\NovaPlayer\playlists`. Writes are atomic
//! (temp + rename) on a writer thread, at most every 2 s while a list keeps changing.

use crate::library::{Song, Tags};
use crate::playback::engine::Span;
use crate::sync::Locked;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;

const INDEX_MAGIC: &str = "NVPLAYLISTS 1";
const LIST_MAGIC: &str = "NVPLAYLIST 1";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    #[default]
    Made,
    Folder(PathBuf),
    File(PathBuf),
    /// A link a plugin turned into the list (从链接导入): imported again on rescan.
    Link(String),
}

impl Origin {
    pub fn name(&self) -> Option<String> {
        match self {
            Origin::Made => None,
            Origin::Folder(p) => Some(
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.to_string_lossy().trim_end_matches(['\\', '/']).to_owned()),
            ),
            Origin::File(p) => p.file_stem().map(|s| s.to_string_lossy().into_owned()),
            Origin::Link(_) => None,
        }
        .filter(|n| !n.trim().is_empty())
    }
    pub fn same(&self, o: &Origin) -> bool {
        let f = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
        match (self, o) {
            (Origin::Folder(a), Origin::Folder(b)) | (Origin::File(a), Origin::File(b)) => f(a) == f(b),
            (Origin::Link(a), Origin::Link(b)) => a.trim() == b.trim(),
            _ => false,
        }
    }
    pub fn rescannable(&self) -> bool {
        !matches!(self, Origin::Made)
    }
    fn fields(&self) -> (&'static str, String) {
        match self {
            Origin::Made => ("-", String::new()),
            Origin::Folder(p) => ("folder", p.to_string_lossy().into_owned()),
            Origin::File(p) => ("file", p.to_string_lossy().into_owned()),
            Origin::Link(u) => ("link", u.clone()),
        }
    }
    fn read(kind: &str, path: &str) -> Option<Origin> {
        match kind {
            "-" => Some(Origin::Made),
            "folder" if !path.is_empty() => Some(Origin::Folder(PathBuf::from(path))),
            "file" if !path.is_empty() => Some(Origin::File(PathBuf::from(path))),
            "link" if !path.is_empty() => Some(Origin::Link(path.to_owned())),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub name: String,
    pub origin: Origin,
    pub count: usize,
}

impl Entry {
    /// The list a kept entry stands for when the store has no file of it: empty, under the
    /// entry's name and from its origin (a save writes the origin back).
    pub fn empty_list(&self) -> List {
        List { name: self.name.clone(), origin: self.origin.clone(), ..Default::default() }
    }
}

#[derive(Clone, Debug, Default)]
pub struct List {
    pub name: String,
    pub origin: Origin,
    /// Shared, copy-on-write at two levels: a save hands the store thread the same songs, not a
    /// copy of every song's strings on the UI thread (12–28 ms at 20 000 rows), and a change
    /// while the store still holds them copies the pointers and the one song changed, not the
    /// list (a tag batch landing during an encode took 7.5 ms). Change through `songs_mut` /
    /// `song_mut`.
    pub songs: std::sync::Arc<Vec<std::sync::Arc<Song>>>,
    /// Play order: positions into `songs`.
    pub order: Vec<usize>,
    /// The user's own arrangement (without shuffle); empty = found order.
    pub own: Vec<usize>,
    /// Index into `songs` of the current song.
    pub current: usize,
    pub position: Duration,
    /// An actual playback selection exists; importing an untouched list does not create one.
    pub resume: bool,
    pub shuffled: bool,
    /// The play mode the list was left in (repeat one / all / off; shuffle is `shuffled`).
    pub mode: Option<crate::library::queue::Order>,
    /// The sort the play order was put in (key, descending); cleared by a hand arrangement.
    pub sort: Option<(crate::library::queue::Sort, bool)>,
}

impl List {
    /// Where `song` is among the songs (`Song::is`: a CUE track by its span too).
    pub fn find(&self, song: &Song) -> Option<usize> {
        self.songs.iter().position(|s| s.is(&song.path, song.span))
    }

    /// Read again, the list's current song is the one that was current, while it is still there.
    pub fn keep_current(&mut self, current: Option<&Song>) {
        if let Some(i) = current.and_then(|c| self.find(c)) {
            self.current = i;
            self.order.retain(|&k| k < self.songs.len());
        }
    }

    /// The list keeps its play mode: shuffled or not, and the order mode it plays in.
    pub fn keep_mode(&mut self, mode: crate::library::queue::Order) {
        self.shuffled = mode == crate::library::queue::Order::Shuffle;
        self.mode = Some(mode);
    }

    /// The songs to change (copied first only if a save still holds them).
    pub fn songs_mut(&mut self) -> &mut Vec<std::sync::Arc<Song>> {
        std::sync::Arc::make_mut(&mut self.songs)
    }

    /// Song `i` to change (copied first only if a save still holds it).
    pub fn song_mut(&mut self, i: usize) -> Option<&mut Song> {
        self.songs_mut().get_mut(i).map(std::sync::Arc::make_mut)
    }
}

fn field(t: &str) -> String {
    t.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}
fn opt(t: &Option<String>) -> String {
    t.as_deref().map(field).unwrap_or_default()
}
fn read_opt(t: &str) -> Option<String> {
    (!t.is_empty()).then(|| t.to_owned())
}

pub fn encode_index(entries: &[Entry], active: Option<u32>) -> String {
    let mut s = format!("{INDEX_MAGIC}\nactive\t{}\n", active.map(|i| i.to_string()).unwrap_or_default());
    for e in entries {
        let (k, p) = e.origin.fields();
        s.push_str(&format!("list\t{}\t{}\t{k}\t{}\t{}\n", e.id, e.count, field(&p), field(&e.name)));
    }
    s
}

pub fn decode_index(text: &str) -> Option<(Vec<Entry>, Option<u32>)> {
    let mut lines = text.lines();
    if lines.next()? != INDEX_MAGIC {
        return None;
    }
    let active = lines.next()?.strip_prefix("active\t")?.parse().ok();
    let mut out = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.strip_prefix("list\t")?.splitn(5, '\t').collect();
        let [id, count, kind, path, name] = f[..] else { return None };
        out.push(Entry {
            id: id.parse().ok()?,
            count: count.parse().ok()?,
            origin: Origin::read(kind, path)?,
            name: name.to_owned(),
        });
    }
    Some((out, active))
}

pub fn encode_list(l: &List) -> String {
    let (k, o) = l.origin.fields();
    let mut s = format!(
        "{LIST_MAGIC}\nname\t{}\norigin\t{k}\t{}\nshuffled\t{}\ncurrent\t{}\nposition\t{}\n",
        field(&l.name),
        field(&o),
        u8::from(l.shuffled),
        l.current,
        l.position.as_millis()
    );
    s.push_str(&format!("resume\t{}\n", u8::from(l.resume)));
    if let Some((k, desc)) = l.sort {
        s.push_str(&format!("sort\t{}\t{}\n", k.key(), u8::from(desc)));
    }
    if let Some(m) = l.mode {
        s.push_str(&format!("mode\t{}\n", m.key()));
    }
    for (n, ps) in [("order", &l.order), ("own", &l.own)] {
        s.push_str(n);
        for p in ps {
            s.push_str(&format!(" {p}"));
        }
        s.push('\n');
    }
    let ns = |t: Option<Duration>| t.map(|t| t.as_nanos().to_string()).unwrap_or_default();
    for song in l.songs.iter() {
        let t = &song.tags;
        let track = t
            .track
            .as_deref()
            .and_then(|x| x.split('/').next())
            .and_then(|x| x.trim().parse::<u32>().ok())
            .map(|n| n.to_string())
            .unwrap_or_default();
        s.push_str(&format!(
            "item\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            field(&song.path.to_string_lossy()),
            ns(song.span.map(|s| s.start)),
            ns(song.span.and_then(|s| s.end)),
            field(&song.name),
            opt(&t.title),
            opt(&t.artist),
            opt(&t.album),
            opt(&t.album_artist),
            track,
            opt(&t.genre),
            opt(&t.date),
            t.duration.map(|d| d.as_millis().to_string()).unwrap_or_default(),
            u8::from(song.probed)
        ));
    }
    s
}

pub fn decode_list(text: &str) -> Option<List> {
    let mut lines = text.lines();
    if lines.next()? != LIST_MAGIC {
        return None;
    }
    let mut named = |n: &str| lines.next()?.strip_prefix(n)?.strip_prefix('\t').map(str::to_owned);
    let name = named("name")?;
    let origin = {
        let o = named("origin")?;
        let (k, p) = o.split_once('\t')?;
        Origin::read(k, p)?
    };
    let shuffled = named("shuffled")? == "1";
    let current: usize = named("current")?.parse().ok()?;
    let position = Duration::from_millis(named("position")?.parse().ok()?);
    let pos = |l: Option<&str>, n: &str| -> Option<Vec<usize>> {
        l?.strip_prefix(n)?.split_whitespace().map(|x| x.parse().ok()).collect()
    };
    let mut order_line = lines.next()?;
    // Legacy lists lack an explicit marker. Nonzero progress/selection is evidence of playback;
    // a never-played first row at zero cannot be distinguished and remains stopped.
    let resume = if let Some(v) = order_line.strip_prefix("resume\t") {
        let resume = v == "1";
        order_line = lines.next()?;
        resume
    } else {
        !position.is_zero() || current > 0
    };
    let mut sort = None;
    if let Some(v) = order_line.strip_prefix("sort\t") {
        let (k, d) = v.split_once('\t')?;
        sort = crate::library::queue::Sort::from_key(k).map(|k| (k, d == "1"));
        order_line = lines.next()?;
    }
    let mut mode = None;
    if let Some(v) = order_line.strip_prefix("mode\t") {
        mode = crate::library::queue::Order::from_key(v);
        order_line = lines.next()?;
    }
    let order = pos(Some(order_line), "order")?;
    let own = pos(lines.next(), "own")?;
    let mut songs = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.strip_prefix("item\t")?.split('\t').collect();
        if !(12..=13).contains(&f.len()) {
            return None;
        }
        let [path, start, end, title, tt, artist, album, aa, track, genre, date, dur] = f[..12] else { return None };
        let t = |x: &str| -> Option<Option<Duration>> {
            if x.is_empty() {
                Some(None)
            } else {
                x.parse::<u64>().ok().map(|n| Some(Duration::from_nanos(n)))
            }
        };
        let (start, end) = (t(start)?, t(end)?);
        let mut s = Song::new(PathBuf::from(path));
        s.name = title.to_owned();
        s.span = start.map(|start| Span { start, end });
        s.tags = Tags {
            title: read_opt(tt),
            artist: read_opt(artist),
            album: read_opt(album),
            album_artist: read_opt(aa),
            track: read_opt(track),
            genre: read_opt(genre),
            date: read_opt(date),
            duration: if dur.is_empty() { None } else { Some(Duration::from_millis(dur.parse().ok()?)) },
        };
        s.probed = f.get(12).is_some_and(|&v| v == "1");
        songs.push(s);
    }
    let n = songs.len();
    let valid = |v: &Vec<usize>| v.iter().all(|&i| i < n);
    if !valid(&order) || !valid(&own) {
        return None;
    }
    Some(List {
        name,
        origin,
        songs: std::sync::Arc::new(songs.into_iter().map(std::sync::Arc::new).collect()),
        order,
        own,
        current: current.min(n.saturating_sub(1)),
        position,
        resume,
        shuffled,
        mode,
        sort,
    })
}

pub fn folder() -> Option<PathBuf> {
    crate::data_dir().map(|d| d.join("playlists"))
}

enum W {
    Write(PathBuf, String),
    /// A list, encoded on the writer thread: a long list's text is a sizeable job (tens of ms
    /// for 20 000 songs) and must not hold up the UI thread.
    WriteList(PathBuf, Box<List>),
    Delete(PathBuf),
    /// A list read and decoded, after every write asked before it, handed to its continuation.
    Read(PathBuf, Box<dyn FnOnce(Option<List>) + Send>),
    /// The index read again, handed to its continuation.
    ReadIndex(PathBuf, Box<dyn FnOnce(Option<Vec<Entry>>) + Send>),
    Flush(Sender<()>),
    /// A kept file outside the folder (`app::kept`), whole; `failed` hears why it was not.
    Keep(PathBuf, Vec<u8>, Option<Box<dyn FnOnce(std::io::Error) + Send>>),
    /// Work that writes kept files itself (the effects chain's states), in turn with the rest.
    Run(Box<dyn FnOnce() + Send>),
}

pub struct Store {
    dir: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub active: Option<u32>,
    tx: Sender<W>,
    /// Each file's modification time as this process last left it: a different one on disk is
    /// someone else's edit (another tool, a sync client) and is read back in.
    seen: Seen,
}

type Seen = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<PathBuf, std::time::SystemTime>>>;

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// The list files and the index in `dir`, with their times.
fn listing(dir: &Path) -> Vec<(PathBuf, std::time::SystemTime)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .filter_map(|p| mtime(&p).map(|t| (p, t)))
        .collect()
}

/// A list file read and decoded (a damaged one set aside), its CUE texts repaired.
fn read_list(p: &Path) -> Option<List> {
    let mut l = crate::files::read_kept(p, decode_list)?;
    crate::library::repair_cue_text(l.songs_mut());
    Some(l)
}

/// The system's notice that something in a folder changed (a file written, renamed, removed):
/// `wait` blocks until it does. Windows: a change notification handle; Linux: inotify.
struct FolderChanges {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
    #[cfg(not(windows))]
    fd: std::os::raw::c_int,
}

#[cfg(windows)]
impl FolderChanges {
    fn open(dir: &Path) -> Option<Self> {
        use windows::Win32::Storage::FileSystem::*;
        let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE;
        // SAFETY: the path is a live HSTRING for the call; the handle it returns is ours, closed
        // in `Drop`.
        let handle =
            unsafe { FindFirstChangeNotificationW(&windows::core::HSTRING::from(dir.as_os_str()), false, filter) }
                .ok()?;
        Some(FolderChanges { handle })
    }

    /// Blocks until the folder changes; false when it cannot be watched any more.
    fn wait(&mut self) -> bool {
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::Storage::FileSystem::FindNextChangeNotification;
        use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
        // SAFETY: `handle` is the change notification this value owns, open until `Drop`.
        unsafe {
            WaitForSingleObject(self.handle, INFINITE) == WAIT_OBJECT_0
                && FindNextChangeNotification(self.handle).is_ok()
        }
    }
}

#[cfg(windows)]
impl Drop for FolderChanges {
    fn drop(&mut self) {
        // SAFETY: `handle` was opened by `FindFirstChangeNotificationW` and is closed once, here.
        unsafe {
            let _ = windows::Win32::Storage::FileSystem::FindCloseChangeNotification(self.handle);
        }
    }
}

#[cfg(not(windows))]
mod inotify {
    use std::os::raw::{c_char, c_int, c_void};
    pub const IN_CLOEXEC: c_int = 0o2000000;
    pub const IN_MODIFY: u32 = 0x2;
    pub const IN_CLOSE_WRITE: u32 = 0x8;
    pub const IN_MOVED_FROM: u32 = 0x40;
    pub const IN_MOVED_TO: u32 = 0x80;
    pub const IN_CREATE: u32 = 0x100;
    pub const IN_DELETE: u32 = 0x200;
    extern "C" {
        pub fn inotify_init1(flags: c_int) -> c_int;
        pub fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
        pub fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
        pub fn close(fd: c_int) -> c_int;
    }
}

#[cfg(not(windows))]
impl FolderChanges {
    fn open(dir: &Path) -> Option<Self> {
        use inotify::*;
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: plain system calls; the descriptor is ours, closed in `Drop` (or here on
        // failure); `path` is a NUL-terminated string alive for the call.
        unsafe {
            let fd = inotify_init1(IN_CLOEXEC);
            if fd < 0 {
                return None;
            }
            let mask = IN_MODIFY | IN_CLOSE_WRITE | IN_MOVED_FROM | IN_MOVED_TO | IN_CREATE | IN_DELETE;
            if inotify_add_watch(fd, path.as_ptr(), mask) < 0 {
                close(fd);
                return None;
            }
            Some(FolderChanges { fd })
        }
    }

    /// Blocks until the folder changes; false when it cannot be watched any more.
    fn wait(&mut self) -> bool {
        let mut buf = [0u8; 4096];
        loop {
            // SAFETY: `fd` is the inotify descriptor this value owns; the buffer outlives the
            // call and its length is passed. The events read are not looked into: any is a change.
            let n = unsafe { inotify::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                return true;
            }
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
    }
}

#[cfg(not(windows))]
impl Drop for FolderChanges {
    fn drop(&mut self) {
        // SAFETY: `fd` was opened by `inotify_init1` and is closed once, here.
        unsafe {
            inotify::close(self.fd);
        }
    }
}

/// The playlists' index file's name, in the playlists folder.
const INDEX: &str = "index.txt";

/// The index file of the playlists folder `dir`.
fn index_path(dir: &Path) -> PathBuf {
    dir.join(INDEX)
}

/// A write of this process to the playlists folder, noted as seen so the watcher does not read
/// it back.
fn write_seen(seen: &Seen, p: PathBuf, bytes: &[u8]) {
    let _ = crate::files::write_atomic(&p, bytes);
    if let Some(t) = mtime(&p) {
        seen.locked().insert(p, t);
    }
}

impl Store {
    pub fn open() -> Store {
        let dir = folder();
        if let Some(d) = &dir {
            let _ = std::fs::create_dir_all(d);
        }
        let (entries, active) =
            dir.as_ref().and_then(|d| crate::files::read_kept(&index_path(d), decode_index)).unwrap_or_default();
        let (tx, rx) = channel::<W>();
        let seen: Seen = Default::default();
        if let Some(d) = &dir {
            seen.locked().extend(listing(d));
        }
        let seen_w = seen.clone();
        std::thread::Builder::new()
            .name("novaplayer-store".into())
            .spawn(move || {
                for w in rx {
                    match w {
                        W::Write(p, s) => write_seen(&seen_w, p, s.as_bytes()),
                        W::WriteList(p, l) => write_seen(&seen_w, p, encode_list(&l).as_bytes()),
                        W::Delete(p) => {
                            let _ = std::fs::remove_file(&p);
                            seen_w.locked().remove(&p);
                        }
                        W::Read(p, then) => then(read_list(&p)),
                        W::ReadIndex(p, then) => {
                            then(std::fs::read_to_string(p).ok().and_then(|t| decode_index(&t)).map(|(e, _)| e))
                        }
                        W::Flush(done) => {
                            let _ = done.send(());
                        }
                        W::Keep(p, bytes, failed) => {
                            if let (Err(e), Some(failed)) = (crate::files::write_atomic(&p, &bytes), failed) {
                                failed(e);
                            }
                        }
                        W::Run(work) => work(),
                    }
                }
            })
            .ok();
        Store { dir, entries, active, tx, seen }
    }

    /// Watches the folder for edits made outside this process, on its own thread, woken by the
    /// system when something in it changes. `changed` gets the files whose time moved.
    pub fn watch(&self, changed: impl Fn(Vec<PathBuf>) + Send + 'static) {
        let (Some(dir), seen) = (self.dir.clone(), self.seen.clone()) else { return };
        let _ = std::thread::Builder::new().name("novaplayer-store-watch".into()).spawn(move || {
            let Some(mut folder) = FolderChanges::open(&dir) else { return };
            // Asleep in the system until something in the folder changes: an idle player does
            // no work for it.
            while folder.wait() {
                // A burst of changes (and this process's own write, whose time the writer notes
                // as it renames) settles first.
                std::thread::sleep(Duration::from_millis(250));
                let now = listing(&dir);
                let mut moved = Vec::new();
                {
                    let mut s = seen.locked();
                    for (p, t) in now {
                        if s.get(&p) != Some(&t) {
                            s.insert(p.clone(), t);
                            moved.push(p);
                        }
                    }
                }
                if !moved.is_empty() {
                    changed(moved);
                }
            }
        });
    }

    /// Reads the index again (an outside edit), on the store's thread: `then` gets the lists
    /// and their names (none: it could not be read), for `set_entries`.
    pub fn read_index(&self, then: impl FnOnce(Option<Vec<Entry>>) + Send + 'static) {
        match &self.dir {
            Some(d) => {
                let _ = self.tx.send(W::ReadIndex(index_path(d), Box::new(then)));
            }
            None => then(None),
        }
    }

    /// The lists as the index read again says (`read_index`). The active list stays this
    /// process's own choice.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
    }

    /// The list a file in the folder holds (`p<id>.txt`).
    /// Whether `file` is the playlists' index.
    pub fn is_index(&self, file: &Path) -> bool {
        file.file_name().is_some_and(|n| n == INDEX)
    }

    pub fn id_of(&self, file: &Path) -> Option<u32> {
        let stem = file.file_stem()?.to_str()?;
        stem.strip_prefix('p')?.parse().ok()
    }

    fn path(&self, id: u32) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("p{id}.txt")))
    }

    /// When the list was last written (its songs' tags are as of then).
    pub fn saved_at(&self, id: u32) -> Option<std::time::SystemTime> {
        std::fs::metadata(self.path(id)?).ok()?.modified().ok()
    }

    /// Reads list `id` on the store's thread, after the writes asked before (a list just saved
    /// reads as saved): `then` gets it there (none: no such list, or it could not be read).
    pub fn read(&self, id: u32, then: impl FnOnce(Option<List>) + Send + 'static) {
        match self.path(id) {
            Some(p) => {
                let _ = self.tx.send(W::Read(p, Box::new(then)));
            }
            None => then(None),
        }
    }

    /// List `id`, read here (the screenshot suites, Windows only).
    #[cfg(all(feature = "test-support", windows))]
    pub fn load(&self, id: u32) -> Option<List> {
        read_list(&self.path(id)?)
    }

    pub fn create(&mut self, name: &str, origin: Origin) -> u32 {
        let id = self.entries.iter().map(|e| e.id).max().map_or(1, |m| m + 1);
        self.entries.push(Entry { id, name: name.to_owned(), origin, count: 0 });
        self.save_index();
        id
    }

    pub fn find(&self, origin: &Origin) -> Option<u32> {
        self.entries.iter().find(|e| e.origin.same(origin)).map(|e| e.id)
    }

    pub fn entry(&self, id: u32) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn rename(&mut self, id: u32, name: &str) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.name = name.to_owned();
        }
        self.save_index();
    }

    pub fn delete(&mut self, id: u32) {
        self.entries.retain(|e| e.id != id);
        if self.active == Some(id) {
            self.active = None;
        }
        if let Some(p) = self.path(id) {
            let _ = self.tx.send(W::Delete(p));
        }
        self.save_index();
    }

    pub fn set_active(&mut self, id: Option<u32>) {
        if self.active != id {
            self.active = id;
            self.save_index();
        }
    }

    pub fn save(&mut self, id: u32, l: List) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.count = l.order.len();
            e.name = l.name.clone();
            e.origin = l.origin.clone();
        }
        if let Some(p) = self.path(id) {
            let _ = self.tx.send(W::WriteList(p, Box::new(l)));
        }
        self.save_index();
    }

    fn save_index(&self) {
        if let Some(d) = &self.dir {
            let _ = self.tx.send(W::Write(index_path(d), encode_index(&self.entries, self.active)));
        }
    }

    /// Writes a kept file (`app::kept`) on the store's thread, after what was asked before;
    /// `failed` hears why it could not be.
    pub fn keep(&self, file: PathBuf, bytes: Vec<u8>, failed: Option<Box<dyn FnOnce(std::io::Error) + Send>>) {
        let _ = self.tx.send(W::Keep(file, bytes, failed));
    }

    /// Runs `work` (which writes kept files itself) on the store's thread, in turn.
    pub fn run(&self, work: impl FnOnce() + Send + 'static) {
        let _ = self.tx.send(W::Run(Box::new(work)));
    }

    /// Waits (at most 5 s) until everything asked of the store's thread before is done.
    pub fn flush(&self) {
        let (tx, rx) = channel();
        if self.tx.send(W::Flush(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: a playlist shown without a file of its own came back as one made here, and
    /// the next save wrote that over its folder, file or link.
    #[test]
    fn an_entry_without_a_file_is_an_empty_list_from_its_origin() {
        let e = Entry { id: 3, name: "Live".into(), origin: Origin::Folder("C:/m".into()), count: 0 };
        let l = e.empty_list();
        assert_eq!((l.name.as_str(), &l.origin, l.songs.len()), ("Live", &Origin::Folder("C:/m".into()), 0));
    }

    /// Regression: a list read again found the current song by its path alone, so a CUE track
    /// came back as the first track of its file.
    #[test]
    fn a_list_read_again_keeps_its_current_cue_track() {
        let track = |i: u64| {
            let mut s = Song::new(PathBuf::from("C:/m/cd.flac"));
            s.span = Some(Span { start: Duration::from_secs(i * 60), end: Some(Duration::from_secs(i * 60 + 60)) });
            std::sync::Arc::new(s)
        };
        let mut list = List {
            songs: std::sync::Arc::new((0..3).map(track).collect()),
            order: vec![0, 1, 2],
            ..Default::default()
        };
        list.keep_current(Some(&track(2)));
        assert_eq!(list.current, 2);
    }

    #[test]
    fn list_roundtrip() {
        let mut s = Song::new(PathBuf::from("C:\\m\\a b.flac"));
        s.probed = true;
        s.tags.title = Some("T\tx".into());
        s.tags.duration = Some(Duration::from_millis(1234));
        let mut c = Song::new(PathBuf::from("C:\\m\\cd.flac"));
        c.span = Some(Span { start: Duration::from_nanos(5), end: Some(Duration::from_nanos(99)) });
        let l = List {
            name: "n".into(),
            origin: Origin::Folder("C:\\m".into()),
            songs: std::sync::Arc::new(vec![s.into(), c.into()]),
            order: vec![1, 0],
            own: vec![],
            current: 1,
            position: Duration::from_millis(42),
            resume: true,
            shuffled: true,
            mode: Some(crate::library::queue::Order::RepeatOne),
            sort: Some((crate::library::queue::Sort::Album, true)),
        };
        let d = decode_list(&encode_list(&l)).unwrap();
        assert_eq!(d.order, vec![1, 0]);
        assert_eq!(d.mode, Some(crate::library::queue::Order::RepeatOne));
        assert_eq!(d.sort, Some((crate::library::queue::Sort::Album, true)));
        assert_eq!(d.songs[0].tags.title.as_deref(), Some("T x"));
        assert_eq!(d.songs[1].span, l.songs[1].span);
        assert_eq!(d.position, l.position);
        assert!(d.resume && d.songs[0].probed);
        let legacy = encode_list(&l)
            .lines()
            .filter(|line| !line.starts_with("resume\t"))
            .map(|line| if line.starts_with("item\t") { line.rsplit_once('\t').unwrap().0 } else { line })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(decode_list(&legacy).unwrap().resume);
        let mut untouched = l.clone();
        untouched.current = 0;
        untouched.position = Duration::ZERO;
        untouched.resume = false;
        assert!(!decode_list(&encode_list(&untouched)).unwrap().resume);
        untouched.resume = true;
        assert!(decode_list(&encode_list(&untouched)).unwrap().resume, "played at zero differs from never played");
        let idx = encode_index(&[Entry { id: 3, name: "x".into(), origin: Origin::Made, count: 2 }], Some(3));
        assert_eq!(decode_index(&idx).unwrap().1, Some(3));
    }
}
