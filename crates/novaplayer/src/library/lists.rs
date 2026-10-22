//! The shown and playing lists: sort, tags, add, import, link, save, switch, sync.
use crate::app::*;
use crate::library::queue::Queue;
use crate::library::store::Origin;
use crate::library::{self, Song};
use crate::sync::Locked;
use novaui::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// How long one batch of a list's tag reading runs before its words land.
const TAG_BATCH: Duration = Duration::from_millis(250);

/// What a batch of tag reading starts from: the list's songs (its first batch looks through
/// them off the UI thread), or the paths the batch before left.
enum Unread {
    Songs(Arc<Vec<Arc<Song>>>),
    Paths(Vec<(usize, PathBuf)>),
}

impl Player {
    /// Puts `song` at the end of kept playlist `id` (in its play order as `Queue::append` does),
    /// wherever the list is now: shown, playing, or only in the store (a list never saved yet is
    /// empty). Its name; `None` when there is no such list.
    pub fn add_song_to(&mut self, song: Song, id: u32) -> Option<String> {
        if self.lists.shown_list.id == Some(id) {
            self.lists.shown_list.append(vec![Arc::new(song)]);
            self.shown_list_moved();
            self.schedule_save();
            return Some(self.lists.shown_list.list.name.clone());
        }
        if let Some(q) = self.lists.playing_list.as_mut().filter(|q| q.id == Some(id)) {
            q.append(vec![Arc::new(song)]);
            let name = q.list.name.clone();
            self.schedule_save();
            return Some(name);
        }
        let name = self.lists.store.entry(id)?.name.clone();
        // Read on the store's thread after the writes asked before: a save of it still on its
        // way lands first. The song joins when it lands (`appended`).
        self.read_list(id, ListFor::Append { id, song: Box::new(song) });
        Some(name)
    }

    /// Asks the store for list `id`, for `then` (`Job::ListRead`).
    fn read_list(&mut self, id: u32, then: ListFor) {
        let inbox = self.inbox.clone();
        self.lists.store.read(id, move |list| {
            inbox.send(Job::ListRead { list: list.map(Box::new), then: Box::new(then) });
        });
    }

    /// A list read from the store lands: what it was read for goes on.
    pub(crate) fn list_read(&mut self, ui: &mut Ui, list: Option<crate::library::store::List>, then: ListFor) {
        match then {
            ListFor::Restore(id) => self.restored(ui, list.map(|l| (id, l))),
            ListFor::Show(id) => {
                if self.lists.switching != Some(id) {
                    return;
                }
                self.lists.switching = None;
                let Some(list) =
                    list.or_else(|| self.lists.store.entry(id).map(crate::library::store::Entry::empty_list))
                else {
                    return;
                };
                let at = list.position;
                self.show_list(ui, list, id, at, false);
                self.sync_link(ui, false);
            }
            ListFor::Append { id, song } => self.appended(id, *song, list),
            ListFor::Refresh(r) => self.refreshed(ui, *r, list),
            ListFor::Reload(id) => self.reloaded(ui, id, list),
        }
    }

    /// `add_song_to` for a list neither shown nor playing: `list` as the store read it.
    fn appended(&mut self, id: u32, song: Song, list: Option<crate::library::store::List>) {
        // Shown or playing by now: it takes the song as such a list does.
        if self.lists.shown_list.id == Some(id) || self.lists.playing_list.as_ref().is_some_and(|q| q.id == Some(id)) {
            self.add_song_to(song, id);
            return;
        }
        let Some(e) = self.lists.store.entry(id).cloned() else { return };
        let list = list.unwrap_or_else(|| e.empty_list());
        let mut q = Queue::default();
        q.adopt(list, Some(id));
        q.append(vec![Arc::new(song)]);
        self.lists.store.save(id, q.to_list());
    }

    /// 添加到 / 移动到 (the row menu, a row carried onto a playlist): the shown list's song at
    /// `pos` joins playlist `to`, a new one for `None`; moved, it leaves the shown list. The song
    /// that plays stays where it is; the shown list is not a target (the song is in it).
    pub fn put_row(&mut self, ui: &mut Ui, pos: usize, to: Option<u32>, mv: bool) {
        let Some(song) = self.lists.shown_list.song_at(pos).cloned() else { return };
        if (mv && pos == self.lists.shown_list.pos) || (to.is_some() && to == self.lists.shown_list.id) {
            return;
        }
        let id = to.unwrap_or_else(|| self.lists.store.create(&ui.t("list.new", &[]), Origin::Made));
        let Some(name) = self.add_song_to(song, id) else { return };
        if mv {
            self.lists.shown_list.remove(pos);
            self.schedule_save();
        }
        self.toast(ui, ui.t(if mv { "toast.moved-to" } else { "toast.added-to" }, &[("name", &name)]));
    }

    /// Sorts the queue by `key`. A sort by a tag needs every song's tags, while only the rows in
    /// view are read lazily: the rest are read first (from the row cache, else the file) off the
    /// UI thread, and the sort lands when they are in. Until then the order stays as it was.
    pub fn sort_by(&mut self, ui: &mut Ui, key: crate::library::queue::Sort, desc: bool) {
        // Shuffle owns the play order: a sort would end it (and the button is disabled for it).
        if !self.lists.shown_list.sortable() {
            return;
        }
        // What is known sorts now; songs still being read take their place when they land.
        self.lists.shown_list.sort(key, desc);
        self.side_panel.locate = true;
        self.schedule_save();
        if !self.lists.shown_list.unprobed(key).is_empty() {
            self.fill_tags(ui);
            self.lists.sort_waiting = Some((key, desc));
            if let Some((_, left)) = self.lists.tag_fill {
                self.toast(ui, ui.t("toast.reading-tags", &[("n", &left.to_string())]));
            }
        }
    }

    /// Reads, in the background and several at a time, the tags of every song of the open list
    /// not read yet (tags only, no pictures), as foobar2000 and its kind keep a list's metadata:
    /// read once when the songs come in, kept with the list, so a sort never waits on files.
    pub fn fill_tags(&mut self, ui: &mut Ui) {
        let list = self.lists.list_version;
        if self.lists.tag_fill.is_some_and(|(l, _)| l == list) {
            return;
        }
        // Counted here, collected on the reader: a path copy per song of a 20 000-song list
        // took ~2.5 ms of the turn that landed it. The reader sees these same songs (shared).
        let songs = self.lists.shown_list.list.songs.clone();
        let total = songs.iter().filter(|s| s.unread()).count();
        if total == 0 {
            self.lists.tag_fill = None;
            return;
        }
        self.lists.tag_fill = Some((list, total));
        self.read_tags(ui, Unread::Songs(songs), total);
    }

    /// Reads the next batch of a list's unread tags: what can be read in [`TAG_BATCH`] (a few
    /// files at once), so the rows take their words as they come; the rest is the next batch's.
    fn read_tags(&mut self, ui: &mut Ui, unread_songs: Unread, total: usize) {
        let task = ui.spawn_waiting(
            move |cancel| {
                let need: Vec<(usize, PathBuf)> = match unread_songs {
                    Unread::Songs(songs) => {
                        songs.iter().enumerate().filter(|(_, s)| s.unread()).map(|(i, s)| (i, s.path.clone())).collect()
                    }
                    Unread::Paths(paths) => paths,
                };
                let t0 = std::time::Instant::now();
                let got = std::sync::Mutex::new(Vec::new());
                let stop = || cancel.cancelled() || t0.elapsed() >= TAG_BATCH;
                library::read_tags(need.clone(), &stop, &|i, t| got.locked().push((i, t)));
                let tags = got.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
                let read: std::collections::HashSet<usize> = tags.iter().map(|(i, _)| *i).collect();
                let rest = need.into_iter().filter(|(i, _)| !read.contains(i)).collect();
                Job::Tags { tags, rest, total }
            },
            Landed,
        );
        self.lists.tasks.hold(Some(task));
    }

    pub(crate) fn land_tags(
        &mut self,
        ui: &mut Ui,
        tags: Vec<(usize, Option<library::Tags>)>,
        rest: Vec<(usize, PathBuf)>,
        total: usize,
    ) {
        let list = self.lists.list_version;
        for (i, t) in tags {
            if let Some(s) = self.lists.shown_list.list.song_mut(i) {
                s.took_tags(t);
            }
        }
        let left = rest.len();
        self.lists.tag_fill = (left > 0).then_some((list, left));
        if left > 0 {
            self.read_tags(ui, Unread::Paths(rest), total);
        }
        // The rows in view take the new words (an edit re-fills them).
        self.lists.shown_list.version += 1;
        // A sorted list keeps its sort as the words come in.
        if let Some((k, d)) = self.lists.shown_list.sorted_by() {
            self.lists.shown_list.resort(k, d);
        }
        if self.lists.sort_waiting.is_some() {
            if left > 0 {
                // Progress: the text changes, the toast stays while reading goes on.
                let msg = ui.t("toast.reading-tags", &[("n", &left.to_string())]);
                self.toast.toast_text.clear();
                self.toast_for(ui, msg, TOAST_INFO);
            } else {
                self.lists.sort_waiting = None;
                self.side_panel.locate = true;
                self.hide_toast();
            }
        }
        self.schedule_save();
    }

    /// Adds what `paths` hold, scanned off the UI thread; a new add calls off one still
    /// scanning.
    pub fn add_paths(&mut self, ui: &mut Ui, paths: Vec<PathBuf>, replace: bool) {
        let origin =
            if paths.len() == 1 && (paths[0].is_dir() || library::is_list(&paths[0]) || library::is_cue(&paths[0])) {
                Some(if paths[0].is_dir() { Origin::Folder(paths[0].clone()) } else { Origin::File(paths[0].clone()) })
            } else {
                None
            };
        let task = ui.spawn_waiting(
            move |cancel| {
                let stop = || cancel.cancelled();
                let (mut songs, mut missing) = (Vec::new(), 0);
                for p in &paths {
                    let (s, m) = library::songs_for(p, &stop);
                    songs.extend(s.into_iter().map(Arc::new));
                    missing += m;
                }
                Job::Added { songs, missing, origin, replace }
            },
            Landed,
        );
        self.lists.adding.clear();
        self.lists.adding.hold(Some(task));
    }

    pub(crate) fn on_added(
        &mut self,
        ui: &mut Ui,
        songs: Vec<Arc<Song>>,
        missing: usize,
        origin: Option<Origin>,
        replace: bool,
    ) {
        if songs.is_empty() {
            self.toast(
                ui,
                if missing > 0 {
                    ui.t("toast.missing-skipped", &[("n", &missing.to_string())])
                } else {
                    ui.t("toast.nothing-playable", &[])
                },
            );
            return;
        }
        if replace {
            // An imported list / rescanned folder becomes (or refreshes) its own playlist.
            let origin = origin.unwrap_or_default();
            let name = origin.name().unwrap_or_else(|| ui.t("list.default", &[]));
            let after = After::Added { missing };
            return self.refresh_list(ui, Refresh { name, origin, songs, after });
        }
        let was_empty = self.lists.shown_list.is_empty();
        self.lists.shown_list.append(songs);
        self.added(ui, missing);
        if was_empty && !self.lists.shown_list.is_empty() && !self.playback.opened {
            // Selected, not played (spec, "Scope and start").
            self.lists.shown_list.set_pos(0);
            let auto = false;
            self.open_current(ui, Duration::ZERO, auto);
        }
    }

    /// The shown list took songs that were added: its rows, its tags, what was left out.
    fn added(&mut self, ui: &mut Ui, missing: usize) {
        self.shown_list_moved();
        self.fill_tags(ui);
        if missing > 0 {
            self.toast(ui, ui.t("toast.missing-skipped", &[("n", &missing.to_string())]));
        }
        self.schedule_save();
    }

    /// A list made from a source (a folder, a list file, a link) takes the source's songs as
    /// they are now: kept as its own playlist, found by its origin. The old list, shown or
    /// read from the store first, gives the sort it was in and a round of shuffle in progress.
    fn refresh_list(&mut self, ui: &mut Ui, r: Refresh) {
        let id = self.lists.store.find(&r.origin).unwrap_or_else(|| self.lists.store.create(&r.name, r.origin.clone()));
        self.save_now();
        if self.lists.shown_list.id == Some(id) {
            let old = self.lists.shown_list.to_list();
            self.refreshed(ui, r, Some(old));
        } else {
            self.read_list(id, ListFor::Refresh(Box::new(r)));
        }
    }

    /// `refresh_list` goes on with the old list (none: there was none).
    fn refreshed(&mut self, ui: &mut Ui, r: Refresh, old: Option<crate::library::store::List>) {
        let Refresh { name, origin, songs, after } = r;
        let Some(id) = self.lists.store.find(&origin) else { return };
        let mut list = Queue::new_empty(&name);
        list.origin = origin;
        list.sort = old.as_ref().and_then(|l| l.sort);
        let mode = self.lists.shown_list.order_mode;
        let mut staged = Queue { order_mode: mode, ..Default::default() };
        staged.adopt(list, Some(id));
        staged.append(songs);
        staged.carry_round(old.as_ref(), mode);
        let mut l = staged.list.clone();
        l.keep_mode(staged.order_mode);
        l.current = staged.list.order.first().copied().unwrap_or(0);
        match after {
            After::Added { missing } => {
                self.show_list(ui, l, id, Duration::ZERO, true);
                self.added(ui, missing);
            }
            After::Imported { unavailable } => {
                self.lists.store.rename(id, &name);
                self.show_list(ui, l, id, Duration::ZERO, true);
                if let Some(b) = &mut self.lists.import {
                    b.done += 1;
                    b.skipped += unavailable;
                    b.names.push(name);
                }
                self.import_landed(ui, None);
                self.schedule_save();
            }
        }
    }

    /// Imports the playlist at `url` through the plugin that takes such links: a list of its own
    /// (imported again when the list is rescanned).
    /// A list of the running import landed (or failed): when it was the last, one toast says
    /// what came in.
    pub(crate) fn import_landed(&mut self, ui: &mut Ui, err: Option<String>) {
        let Some(job) = &mut self.lists.import else {
            if let Some(e) = err {
                self.toast_error(ui, ui.t("toast.import-failed", &[("e", &e)]));
            }
            return;
        };
        job.errs.extend(err);
        if job.done + job.failed < job.asked {
            return;
        }
        let Some(ImportJob { done: ok, failed: bad, names, skipped, errs, .. }) = self.lists.import.take() else {
            return;
        };
        let mut msg = match (ok, names.as_slice()) {
            (0, _) => ui.t("toast.import-failed", &[("e", errs.first().map_or("", String::as_str))]),
            (1, [n]) => ui.t("toast.imported-one", &[("name", n)]),
            (k, _) => ui.t("toast.imported-many", &[("n", &k.to_string())]),
        };
        if ok > 0 && bad > 0 {
            msg.push_str(&ui.t("toast.import-some-failed", &[("n", &bad.to_string())]));
        }
        if ok > 0 && skipped > 0 {
            msg.push_str(&ui.t("toast.import-skipped", &[("n", &skipped.to_string())]));
        }
        if ok == 0 || bad > 0 {
            self.toast_error(ui, msg);
        } else {
            self.toast(ui, msg);
        }
    }

    /// Asks a signed-in account for its own playlists; the import dialog opens on the answer.
    pub fn pick_lists(&mut self, ui: &mut Ui, plugin: &str) {
        let (id, acc) = (plugin.to_owned(), self.plugin.accounts.clone());
        let task = ui.spawn_waiting(
            move |_| {
                let lists = crate::plugins::client::user_lists(&acc, &id);
                Job::UserLists { plugin: id, lists }
            },
            Landed,
        );
        self.plugin.account_tasks.hold(Some(task));
    }

    /// Whether a link is already one of the playlists (or on its way in): a link is imported
    /// once, and 重新扫描 brings it up to date.
    pub fn link_known(&self, url: &str) -> bool {
        self.lists.store.find(&Origin::Link(url.to_owned())).is_some() || self.lists.importing.contains(url.trim())
    }

    pub fn import_link(&mut self, ui: &mut Ui, url: String) {
        let Some(plugin) = crate::plugins::client::plugin_for(&self.plugin.registry, &url) else {
            self.toast(ui, ui.t("toast.no-link-plugin", &[]));
            return;
        };
        if let Some(id) = self.lists.store.find(&Origin::Link(url.clone())) {
            // Already a playlist: open it and bring it up to date instead of a second copy.
            if self.lists.shown_list.id != Some(id) {
                self.switch_list(ui, id);
            }
            self.toast(ui, ui.t("toast.list-known", &[]));
            self.sync_link(ui, true);
            return;
        }
        if !self.lists.importing.insert(url.trim().to_owned()) {
            return;
        }
        let job = self.lists.import.get_or_insert_with(Default::default);
        job.asked += 1;
        if job.asked == 1 {
            self.toast(ui, ui.t("toast.importing-online", &[]));
        }
        let acc = self.plugin.accounts.clone();
        let task = ui.spawn_waiting(
            move |_| match crate::plugins::client::fetch(&acc, &plugin, &url) {
                Ok(f) => Job::LinkFetched {
                    url,
                    name: f.name,
                    songs: f.songs.into_iter().map(Arc::new).collect(),
                    unavailable: f.unavailable,
                },
                Err(e) => Job::LinkFailed(url, e),
            },
            Landed,
        );
        self.tasks.hold(Some(task));
    }

    pub(crate) fn on_link(
        &mut self,
        ui: &mut Ui,
        url: String,
        name: String,
        songs: Vec<Arc<Song>>,
        unavailable: usize,
    ) {
        self.lists.importing.remove(url.trim());
        if songs.is_empty() {
            if let Some(b) = &mut self.lists.import {
                b.failed += 1;
            }
            self.import_landed(ui, Some(ui.t("toast.list-empty", &[("name", &name)])));
            return;
        }
        let after = After::Imported { unavailable };
        self.refresh_list(ui, Refresh { name, origin: Origin::Link(url), songs, after });
    }

    /// Re-reads the list's source: new files join, missing ones leave (the playing one stays).
    pub fn rescan(&mut self, ui: &mut Ui, path: PathBuf) {
        let known: Vec<(PathBuf, Option<crate::playback::engine::Span>)> =
            self.lists.shown_list.list.songs.iter().map(|s| (s.path.clone(), s.span)).collect();
        // Incremental: a known song is read again only when its file changed after the list was
        // last saved; new files are read as any added song; the rest keep what is known.
        let saved = self.lists.shown_list.id.and_then(|id| self.lists.store.saved_at(id));
        // Every file is looked at here, never on the UI thread: on a network drive each look is a
        // round trip, and a list of thousands froze the app.
        let task = ui.spawn_waiting(
            move |cancel| {
                // A source that does not answer (a network drive offline) changes nothing: its
                // songs are not taken for gone.
                if std::fs::metadata(&path).is_err() {
                    return Job::Rescanned(None);
                }
                let (changed, gone) = library::changed_since(known.iter().map(|(p, _)| p.as_path()), saved);
                for &i in &changed {
                    crate::library::row_cache::invalidate(&known[i].0);
                }
                let (found, _) = library::songs_for(&path, &|| cancel.cancelled());
                let fresh =
                    found.into_iter().filter(|s| !known.iter().any(|(p, sp)| s.is(p, *sp))).map(Arc::new).collect();
                Job::Rescanned(Some(Rescan { changed, gone, fresh }))
            },
            Landed,
        );
        self.lists.tasks.hold(Some(task));
    }

    pub fn schedule_save(&mut self) {
        self.lists.shown_list.unsaved = true;
        self.keeping.changed(crate::app::kept::Kept::Lists);
    }

    pub fn save_now(&mut self) {
        let at = (self.playback.opened && !self.playback.snap.opening).then(|| self.position());
        fn stamp(q: &mut Queue, at: Option<Duration>) {
            if let Some(a) = at {
                q.list.position = a;
            }
            q.current_from_pos();
            q.list.keep_mode(q.order_mode);
        }
        let detached = self.lists.playing_list.is_some();
        if let Some(p) = self.lists.playing_list.as_mut() {
            stamp(p, at);
            if let Some(id) = p.id {
                self.lists.store.save(id, p.list.clone());
                // Restarting resumes the song that played (and shows its list).
                self.lists.store.set_active(Some(id));
            }
            p.unsaved = false;
        }
        if let Some(id) = self.lists.shown_list.id {
            stamp(&mut self.lists.shown_list, if detached { None } else { at });
            self.lists.store.save(id, self.lists.shown_list.list.clone());
            if !detached {
                self.lists.store.set_active(Some(id));
            }
        }
        self.lists.shown_list.unsaved = false;
    }

    /// The shown list changed under its rows (another list, songs added or read again, a new
    /// default one): what was asked of the old one is called off, and the side panel lists it
    /// again, following its current song. The one way every change of the shown list ends.
    pub fn shown_list_moved(&mut self) {
        self.lists.moved();
        self.side_panel.list_changed();
    }

    /// 新建播放列表: shown empty; the song that plays goes on playing.
    pub fn show_new_list(&mut self, ui: &mut Ui, id: u32) {
        self.show_list(ui, Queue::new_empty(&ui.t("list.new", &[])), id, Duration::ZERO, false);
    }

    /// Shows playlist `id`. The song that plays goes on playing (from its own list); back on
    /// the list it plays from, the page shows it again as it is.
    pub fn switch_list(&mut self, ui: &mut Ui, id: u32) {
        if self.lists.shown_list.id == Some(id) {
            return;
        }
        if self.lists.playing_list.as_ref().is_some_and(|p| p.id == Some(id)) {
            self.save_now();
            let Some(back) = self.lists.playing_list.take() else { return };
            let version = self.lists.shown_list.version.max(back.version);
            self.lists.shown_list = back;
            self.lists.shown_list.version = version + 1;
            self.lists.store.set_active(Some(id));
            self.shown_list_moved();
            self.fill_tags(ui);
            return;
        }
        if self.lists.store.entry(id).is_none() {
            return;
        }
        // Shown once the store has read it (`list_read`); the last list asked for wins.
        self.lists.switching = Some(id);
        self.read_list(id, ListFor::Show(id));
    }

    /// A list made from a link is read again from its service the first time it is opened in a
    /// session (quietly), and on 重新扫描 (told): songs added there join, songs removed leave, the
    /// playing one stays where it plays.
    pub fn sync_link(&mut self, ui: &mut Ui, told: bool) {
        let (Some(id), Origin::Link(url)) = (self.lists.shown_list.id, self.lists.shown_list.list.origin.clone())
        else {
            return;
        };
        if !told && !self.lists.synced.insert(id) {
            return;
        }
        self.fetch_link(ui, id, url, told, false);
    }

    pub(crate) fn fetch_link(&mut self, ui: &mut Ui, id: u32, url: String, told: bool, retried: bool) {
        let Some(plugin) = crate::plugins::client::plugin_for(&self.plugin.registry, &url) else { return };
        let (version, acc) = (self.lists.shown_list.version, self.plugin.accounts.clone());
        // Kept by the app: the answer is checked against the list shown when it lands.
        let task = ui.spawn_waiting(
            move |_| match crate::plugins::client::fetch(&acc, &plugin, &url) {
                Ok(f) => Job::LinkSynced { id, version, retried, songs: f.songs, unavailable: f.unavailable, told },
                Err(_) if told => Job::LinkFailed(String::new(), novaui::i18n::t("err.list-read", &[])),
                // A quiet sync that failed changes nothing (no songs: nothing is laid over).
                Err(_) => Job::LinkSynced { id, version, retried, songs: Vec::new(), unavailable: 0, told },
            },
            Landed,
        );
        self.tasks.hold(Some(task));
    }

    /// Lays a link's fresh songs over the open list: the service's order, the words already
    /// known kept, the playing song kept (and where it plays).
    pub(crate) fn on_link_synced(
        &mut self,
        ui: &mut Ui,
        id: u32,
        (version, retried): (u64, bool),
        songs: Vec<Song>,
        unavailable: usize,
        told: bool,
    ) {
        if self.lists.shown_list.id != Some(id) || songs.is_empty() {
            return;
        }
        // The list changed while the service was read (a move, a removal, a sort): laying the
        // old answer over it would undo that. Ask again, once, against the list as it is now;
        // still moving, it waits for the next open or 重新扫描.
        if self.lists.shown_list.version != version {
            if let (false, Origin::Link(url)) = (retried, self.lists.shown_list.list.origin.clone()) {
                self.fetch_link(ui, id, url, told, true);
            }
            return;
        }
        let old = self.lists.shown_list.list.songs.clone();
        let cur = self.lists.shown_list.current().map(|s| s.path.clone());
        let mut fresh: Vec<std::sync::Arc<Song>> = songs
            .into_iter()
            .map(|s| old.iter().find(|o| o.path == s.path).cloned().unwrap_or_else(|| std::sync::Arc::new(s)))
            .collect();
        let same = fresh.len() == old.len() && fresh.iter().zip(old.iter()).all(|(a, b)| a.path == b.path);
        if same {
            if told {
                self.toast(ui, ui.t("toast.list-current", &[]));
            }
            return;
        }
        if let Some(c) = &cur {
            if !fresh.iter().any(|s| &s.path == c) {
                if let Some(s) = old.iter().find(|o| &o.path == c) {
                    fresh.insert(0, s.clone());
                }
            }
        }
        let added = fresh.iter().filter(|s| !old.iter().any(|o| o.path == s.path)).count();
        let removed = old.iter().filter(|o| !fresh.iter().any(|s| s.path == o.path)).count();
        let mut list = self.lists.shown_list.list.clone();
        let current = cur.as_ref().and_then(|c| fresh.iter().position(|s| &s.path == c)).unwrap_or(0);
        list.songs = std::sync::Arc::new(fresh);
        list.order = Vec::new();
        list.own = Vec::new();
        list.current = current;
        let old_list = self.lists.shown_list.to_list();
        let mode = self.lists.shown_list.order_mode;
        list.shuffled = false;
        self.lists.shown_list.adopt(list, Some(id));
        self.lists.shown_list.carry_round(Some(&old_list), mode);
        self.shown_list_moved();
        self.schedule_save();
        let mut note = Vec::new();
        if added > 0 {
            note.push(ui.t("sync.added", &[("n", &added.to_string())]));
        }
        if removed > 0 {
            note.push(ui.t("sync.removed", &[("n", &removed.to_string())]));
        }
        if unavailable > 0 && told {
            note.push(ui.t("sync.unavailable", &[("n", &unavailable.to_string())]));
        }
        if !note.is_empty() {
            self.toast(
                ui,
                ui.t(
                    "toast.synced",
                    &[(
                        "note",
                        &note.into_iter().reduce(|a, b| ui.t("list.join", &[("a", &a), ("b", &b)])).unwrap_or_default(),
                    )],
                ),
            );
        }
    }

    /// The playlists folder was edited outside this process: its index and the open list are
    /// read again on the store's thread (the open list only while it has no unsaved change of
    /// its own).
    pub(crate) fn on_store_changed(&mut self, _ui: &mut Ui, files: Vec<PathBuf>) {
        if files.iter().any(|f| self.lists.store.is_index(f)) {
            let inbox = self.inbox.clone();
            self.lists.store.read_index(move |entries| {
                if let Some(entries) = entries {
                    inbox.send(Job::IndexRead(entries));
                }
            });
        }
        if let Some(id) =
            self.lists.shown_list.id.filter(|id| files.iter().any(|f| self.lists.store.id_of(f) == Some(*id)))
        {
            self.read_list(id, ListFor::Reload(id));
        }
    }

    /// The index as read again (an outside edit).
    pub(crate) fn index_read(&mut self, entries: Vec<crate::library::store::Entry>) {
        self.lists.store.set_entries(entries);
        if self.lists.shown_list.id.is_some_and(|id| self.lists.store.entry(id).is_none()) {
            // The open list was removed elsewhere: it stays open, and is saved back.
            let name = self.lists.shown_list.list.name.clone();
            let origin = self.lists.shown_list.list.origin.clone();
            let nid = self.lists.store.create(&name, origin);
            self.lists.shown_list.id = Some(nid);
            self.schedule_save();
        }
    }

    /// The open list as read again (an outside edit): taken unless it changed here meanwhile.
    fn reloaded(&mut self, ui: &mut Ui, id: u32, list: Option<crate::library::store::List>) {
        if self.lists.shown_list.id != Some(id)
            || self.lists.shown_list.unsaved
            || self.keeping.pending(crate::app::kept::Kept::Lists)
        {
            return;
        }
        let Some(mut list) = list else { return };
        list.keep_current(self.lists.shown_list.current());
        self.lists.shown_list.adopt(list, Some(id));
        self.lists.shown_list.unsaved = false;
        self.shown_list_moved();
        self.fill_tags(ui);
        self.toast(ui, ui.t("toast.list-reloaded", &[]));
    }
}

/// What a list read from the store is for (`Player::list_read`).
pub enum ListFor {
    /// The list kept active, at the start.
    Restore(u32),
    /// A playlist switched to.
    Show(u32),
    /// A song put at the end of a list neither shown nor playing (`add_song_to`).
    Append { id: u32, song: Box<Song> },
    /// A list refreshed from its source (`refresh_list`).
    Refresh(Box<Refresh>),
    /// The open list edited outside this process.
    Reload(u32),
}

/// A list made from a source, taking the source's songs (`refresh_list`).
pub struct Refresh {
    name: String,
    origin: Origin,
    songs: Vec<Arc<Song>>,
    after: After,
}

/// What a refresh was for.
enum After {
    /// Added paths (a folder, a list file): `missing` of them left out.
    Added { missing: usize },
    /// A link imported: `unavailable` of its songs left out.
    Imported { unavailable: usize },
}

/// `Player` state owned by this area.
pub struct Lists {
    pub store: crate::library::store::Store,
    /// The list the queue page shows (and edits act on).
    pub shown_list: Queue,
    /// The list the loaded song plays from, while the page shows another (a playlist switched
    /// to while a song played). None: the shown list is the one that plays.
    pub playing_list: Option<Queue>,
    /// The open song's list-edit version it belongs to (row tag reads).
    pub list_version: u64,
    /// A sort asked for while tags were still being read: it lands when they are in.
    pub sort_waiting: Option<(crate::library::queue::Sort, bool)>,
    /// The list whose tags are being read, and how many are left (the sort waits on them).
    pub tag_fill: Option<(u64, usize)>,
    /// The shown list's requests (its tags, a rescan, its thumbnails): the list moving on
    /// (`moved`) drops them.
    pub tasks: crate::app::Tasks,
    /// The playlist asked for (`switch_list`), while the store reads it.
    pub switching: Option<u32>,
    /// An add being scanned: a new add calls it off.
    pub adding: crate::app::Tasks,
    /// Lists made from links already read again from their service this session.
    pub synced: std::collections::HashSet<u32>,
    /// The links of one press of 导入 / one link being imported (the first toast says it
    /// started, the last that it is done).
    pub import: Option<ImportJob>,
    /// Links being imported now (a second press of 导入 waits for the first).
    pub importing: std::collections::HashSet<String>,
}

impl Lists {
    /// The list the loaded song plays from: the detached playing list, else the shown one.
    pub fn playing(&self) -> &Queue {
        self.playing_list.as_ref().unwrap_or(&self.shown_list)
    }

    /// `playing`, to change.
    pub fn playing_mut(&mut self) -> &mut Queue {
        match self.playing_list.as_mut() {
            Some(p) => p,
            None => &mut self.shown_list,
        }
    }

    /// With no list left: a new empty one named as the default, kept active and shown. Its id.
    pub fn show_default(&mut self, ui: &Ui) -> u32 {
        let name = ui.t("list.default", &[]);
        let id = self.store.create(&name, crate::library::store::Origin::Made);
        self.store.set_active(Some(id));
        self.shown_list.adopt(Queue::new_empty(&name), Some(id));
        id
    }

    /// The shown list changed under its rows (another list, songs added, removed or read
    /// again): what was asked of the old one is called off, and what lands is asked again.
    pub fn moved(&mut self) {
        self.list_version += 1;
        self.tasks.clear();
    }
}

/// One import of links: how many were asked, how many came in and failed, and what the last
/// toast says of them.
#[derive(Default)]
pub struct ImportJob {
    pub asked: usize,
    pub done: usize,
    pub failed: usize,
    /// The lists that came in.
    pub names: Vec<String>,
    /// Their songs that cannot play now (left out).
    pub skipped: usize,
    pub errs: Vec<String>,
}
