//! Work done off the UI thread and how its results land. A request is a novaui `Task` held by
//! the scope it serves (`Tasks`: the open song's, the shown list's, an account's, a
//! connection's, the app's): when the scope moves on it drops them, the work is cancelled and
//! its result never lands, so a result that lands is about what is shown. What another thread
//! reports on its own (a row read on the assets queue, the effects' host thread, the
//! playlists folder's watcher) comes through the inbox (`JobSender`, `Msg::Jobs`).
use crate::app::*;
use crate::library::store::Origin;
use crate::library::{self, Lyrics, Song};
use crate::player_core::MediaTags;
use novaui::Waker;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

/// The tasks one scope holds. Dropped with it, or by `clear` when it moves on (another song,
/// another list, an account signed in or out), their work is cancelled and never lands.
#[derive(Default)]
pub struct Tasks(Vec<novaui::Task>);

impl Tasks {
    /// Holds `task` (none: the work was not admitted) beside those still running.
    pub fn hold(&mut self, task: Option<novaui::Task>) {
        self.0.retain(novaui::Task::pending);
        self.0.extend(task);
    }

    /// The scope moved on: its work is called off.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// A task's result, delivered as its message (`Player::event` → `Player::land`).
pub struct Landed(pub Job);

/// The inbox: another thread's report sent to the UI thread (`Msg::Jobs`).
#[derive(Clone)]
pub struct JobSender {
    tx: Sender<Job>,
    waker: Waker,
}

impl JobSender {
    pub fn new(tx: Sender<Job>, waker: Waker) -> Self {
        JobSender { tx, waker }
    }

    /// Sends `j` and wakes the UI thread. `false`: the app is gone (closing); `j` is dropped.
    pub fn send(&self, j: Job) -> bool {
        let sent = self.tx.send(j).is_ok();
        if sent {
            self.waker.wake();
        }
        sent
    }
}

/// The artwork a face job brings: the picture itself (none: the song has no picture), or
/// nothing yet (an online song's words and lyrics land first; its picture follows as
/// `Job::OnlineCover`).
pub enum Art {
    Later,
    Now(Option<library::Cover>),
}

/// An online song's face, from its plugin (words filled, lyrics, and the cover unless it is
/// still being fetched).
pub struct OnlineFace {
    pub path: PathBuf,
    pub tags: Option<library::Tags>,
    pub art: Art,
    pub lyrics: Option<Lyrics>,
    pub layers: crate::library::LyricLayers,
    /// The picture still to fetch (`Art::Later`): the words as asked and where it is.
    pub fetch: Option<(crate::plugins::client::TrackInfo, Option<String>)>,
}

/// Work done off the UI thread, reported back.
pub enum Job {
    /// An online song's face (boxed: it is the largest job by far).
    OnlineTrack(Box<OnlineFace>),
    /// An online song's picture, fetched after its words (`Art::Later`); none: it did not come.
    OnlineCover {
        path: PathBuf,
        cover: Option<library::Cover>,
    },
    /// A connection's `connect.begin` answered (`plugins::connect`).
    ConnectBegun(Result<crate::plugins::connect::Begun, String>),
    /// A connection's `connect.poll` answered.
    ConnectPolled(Result<crate::plugins::connect::Polled, String>),
    /// The plugin folders as read (`Player::rescan_plugins`).
    Plugins(Vec<crate::plugins::registry::Plugin>),
    /// What a reload read: what is enabled, the folders (`Registry::reloader`).
    PluginsReloaded(std::collections::BTreeMap<String, String>, Vec<crate::plugins::registry::Plugin>),
    /// A plugin to enable, its files' fingerprint as they are now.
    PluginHashed {
        id: String,
        print: String,
    },
    /// A plugin deleted (its id), or why not; `name`: as it was shown.
    PluginRemoved {
        name: String,
        result: Result<String, String>,
    },
    /// A plugin package installed: its id and name, or why not.
    PluginInstalled(Result<(String, String), String>),
    /// A lyric style installed: its id and name, or why not.
    LyricStyleInstalled(Result<(String, String), String>),
    /// A lyric style deleted (its id), or why not.
    LyricStyleRemoved(Result<String, String>),
    /// A kept file could not be written (`kept`): what to say.
    NotKept(String),
    /// A list written as M3U8 (`Answer::Export`): how many songs, or why not.
    Exported(Result<usize, String>),
    /// A list the store read (`Store::read`), and what for.
    ListRead {
        list: Option<Box<crate::library::store::List>>,
        then: Box<crate::library::lists::ListFor>,
    },
    /// The playlists' index as read again (an outside edit).
    IndexRead(Vec<crate::library::store::Entry>),
    /// The output systems and their devices, as listed (`Player::list_outputs`).
    Outputs(Vec<crate::player_core::AudioOutputSystem>),
    /// The installed plugins' accounts were read from the system's store (`Player::read_accounts`).
    AccountsRead,
    /// The effects that can be added (`playback::effects`'s scan).
    EffectsScanned(Vec<crate::playback::effects::Found>),
    /// An effect could not be added, or its window opened (`Command::Effect`, `Add`, `Window`):
    /// why.
    EffectFailed {
        window: bool,
        why: String,
    },
    /// The parameters of the effect at a place in the chain, as read (`EffectsModel::read_params`).
    EffectParams {
        at: usize,
        key: crate::player_core::clap::EffectKey,
        params: Result<Vec<crate::player_core::clap::EffectParam>, String>,
    },
    /// A playlist a plugin read from a link.
    LinkFetched {
        url: String,
        name: String,
        songs: Vec<std::sync::Arc<Song>>,
        unavailable: usize,
    },
    LinkFailed(String, String),
    /// A plugin command's answer: a toast to show, and whether the song's face is to be asked again.
    /// `path`: the song the command was given for; `song`: the open song's version then.
    PluginCommandDone {
        path: PathBuf,
        song: u64,
        result: Result<(Option<String>, bool), String>,
    },
    /// Files of the playlists folder edited outside this process.
    StoreChanged(Vec<PathBuf>),
    /// Prefetched thumbnails landed (the songs' indexes): their rows ask again. `rest`: what is
    /// left to fetch.
    Thumbs {
        idxs: Vec<usize>,
        rest: crate::plugins::prefetch::Rest,
    },
    /// A link's list read again from its service (opened this session, or 重新扫描).
    /// `version`: the list's version when it was asked for (a list edited since is asked again).
    LinkSynced {
        id: u32,
        version: u64,
        retried: bool,
        songs: Vec<Song>,
        unavailable: usize,
        told: bool,
    },
    /// The values of its plugin's option the playing song comes in (the option flyout).
    Options {
        list: Result<Vec<crate::plugins::client::OptionValue>, String>,
    },
    /// A connected plugin account's own playlists for the import dialog (docs/plugins.md
    /// §lists).
    UserLists {
        plugin: String,
        lists: Result<Vec<crate::plugins::client::UserList>, String>,
    },
    /// Tags + cover + lyrics of the song opened.
    /// `cover_editable`: the file is on disk and carries no picture of its own, so a cover can
    /// be chosen for it (the cover menu).
    Track {
        tags: Option<MediaTags>,
        cover: Option<library::Cover>,
        lyrics: Option<Lyrics>,
        cover_editable: bool,
    },
    /// A queue row's song's file read for the info page (`SidePanel::info_of`).
    SongInfo {
        song: Song,
        tags: Option<Box<MediaTags>>,
    },
    /// A cover chosen (or restored) for the folder of the song open at version `song`.
    Cover {
        song: u64,
        cover: Option<library::Cover>,
    },
    /// Songs found for an add (append to the open list).
    Added {
        songs: Vec<std::sync::Arc<Song>>,
        missing: usize,
        origin: Option<Origin>,
        replace: bool,
    },
    /// A rescan's look at the list's files (song indexes): `changed` since the list was saved
    /// (their tags are read again), `gone` no longer there (they leave the list), `fresh` new
    /// files (they join it). `None`: the source did not answer, nothing changes.
    Rescanned(Option<Rescan>),
    /// Tags read for a queue row (song index, tags, thumb), on the assets queue: `list` and
    /// `version` say which list and which rows asked.
    RowTags {
        list: u64,
        idx: usize,
        version: u64,
        tags: Option<library::Tags>,
        thumb: Option<novaui::ImageHandle>,
    },
    RowSkipped {
        list: u64,
        idx: usize,
        version: u64,
    },
    /// Tags read in the background for the open list (a batch); `rest` still to read, of
    /// `total`.
    Tags {
        tags: Vec<(usize, Option<library::Tags>)>,
        rest: Vec<(usize, PathBuf)>,
        total: usize,
    },
}

/// What a rescan found (`Job::Rescanned`).
pub struct Rescan {
    pub changed: Vec<usize>,
    pub gone: Vec<usize>,
    pub fresh: Vec<std::sync::Arc<Song>>,
}

impl Player {
    /// Scans for the effects that can be added, unless that is done or under way.
    pub fn scan_effects(&mut self, ui: &mut Ui) {
        if self.playback.effects.found.is_none() {
            self.playback.effects.scan(ui);
        }
    }

    /// Reads the plugin folders again, as a task; what changed follows when it lands
    /// (`Registry::adopt`, `plugins_changed`).
    pub fn rescan_plugins(&mut self, ui: &mut Ui) {
        let scanner = self.plugin.registry.scanner();
        let task = ui.spawn_waiting(move |_| scanner.read(), |list| Landed(Job::Plugins(list)));
        self.tasks.hold(Some(task));
    }

    /// Lists the output devices again, as a task (on Linux the core asks the sound server, a
    /// process); the output flyout, when up, shows them once they land, and those listed last
    /// until then.
    pub fn list_outputs(&mut self, ui: &mut Ui) {
        let task = ui.spawn_waiting(|_| crate::player_core::enumerate_output_systems(), |s| Landed(Job::Outputs(s)));
        self.tasks.hold(Some(task));
    }

    /// Reads the installed plugins' accounts from the system's store, those not read yet, as a
    /// task: the store may wait on the user (a locked keyring), the views never do.
    pub fn read_accounts(&mut self, ui: &mut Ui) {
        let ids: Vec<String> = self.plugin.registry.list.iter().map(|p| p.id()).collect();
        let acc = self.plugin.accounts.clone();
        let task = ui.spawn_waiting(move |_| acc.read(&ids), |()| Landed(Job::AccountsRead));
        self.tasks.hold(Some(task));
    }

    /// Reads the parameters of the effect whose level the settings page shows, if it does
    /// (`Job::EffectParams`).
    pub fn read_effect_params(&mut self) {
        let crate::views::settings_page::Level::Effect(at) = self.settings_page.level else { return };
        if !self.settings_page.open {
            return;
        }
        let inbox = self.inbox.clone();
        self.playback.effects.read_params(at, move |key, params| {
            inbox.send(Job::EffectParams { at, key, params });
        });
    }

    /// An online song's picture, named by its plugin with the words (`Job::OnlineTrack`), fetched
    /// and kept with them; it lands as `Job::OnlineCover`. The song's.
    fn fetch_online_cover(
        &mut self,
        ui: &mut Ui,
        path: PathBuf,
        info: crate::plugins::client::TrackInfo,
        at: Option<String>,
    ) {
        let task = ui.spawn_waiting(
            move |_| {
                let f = crate::plugins::client::face_cover(&path, info, at);
                let cover = f.cover.as_deref().and_then(|b| library::decode_cover(b, 1040));
                Job::OnlineCover { path, cover }
            },
            Landed,
        );
        self.playback.tasks.hold(Some(task));
    }

    /// The rows of the shown list that are `path` ask for their thumbnail again (its face is in
    /// the cache now).
    fn forget_row_thumbs(&mut self, path: &std::path::Path) {
        let rows: Vec<usize> = self
            .lists
            .shown_list
            .list
            .songs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.path == path)
            .map(|(i, _)| i)
            .collect();
        for i in rows {
            self.side_panel.forget_thumb(i);
        }
    }

    /// What other threads reported (the inbox).
    pub(crate) fn drain_jobs(&mut self, ui: &mut Ui) {
        while let Ok(j) = self.jobs_rx.try_recv() {
            self.land(ui, j);
        }
    }

    /// A job's result lands: a task's (its scope still holds it, so it is about what is shown)
    /// or the inbox's.
    /// A row read is for the rows shown now: the same list, and the same rows of it.
    fn rows_current(&self, list: u64, version: u64) -> bool {
        list == self.lists.list_version && version == self.side_panel.row_version()
    }

    pub fn land(&mut self, ui: &mut Ui, j: Job) {
        {
            match j {
                Job::Track { tags, cover, lyrics, cover_editable } => {
                    self.now_playing.cover_editable = cover_editable;
                    self.apply_now_playing(tags, Art::Now(cover), lyrics, Default::default());
                }
                Job::OnlineCover { path, cover } => {
                    if cover.is_some() {
                        self.forget_row_thumbs(&path);
                    }
                    // A picture that did not come after the stand-in is up changes nothing.
                    if cover.is_some() || self.now_playing.cover_version != self.playback.song_version {
                        self.show_open_cover(cover);
                    }
                }
                Job::OnlineTrack(face) => {
                    let OnlineFace { path, tags, art, lyrics, layers, fetch } = *face;
                    // The picture follows the words, a fetch of its own.
                    if let Some((info, at)) = fetch {
                        self.fetch_online_cover(ui, path.clone(), info, at);
                    }
                    // The plugin's words fill the list's song too (kept with the list).
                    // Its row's thumbnail: asked again (the face is in the cache now).
                    if matches!(art, Art::Now(Some(_))) {
                        self.forget_row_thumbs(&path);
                    }
                    if let Some(tg) = &tags {
                        for s in self
                            .lists
                            .shown_list
                            .list
                            .songs_mut()
                            .iter_mut()
                            .filter(|s| s.path == path)
                            .map(std::sync::Arc::make_mut)
                        {
                            s.tags = tg.clone();
                        }
                        self.lists.shown_list.version += 1;
                        if let Some(p) = self.lists.playing_list.as_mut() {
                            for s in
                                p.list.songs_mut().iter_mut().filter(|s| s.path == path).map(std::sync::Arc::make_mut)
                            {
                                s.tags = tg.clone();
                            }
                        }
                    }
                    {
                        let mt = tags.as_ref().map(MediaTags::from);
                        self.apply_now_playing(mt, art, lyrics, layers);
                    }
                }
                Job::Cover { song, cover } => {
                    if song == self.playback.song_version {
                        self.side_panel.invalidate_rows();
                        if cover.is_some() {
                            self.show_cover(cover);
                        }
                    }
                }
                Job::SongInfo { song, tags } => {
                    if self.side_panel.info_of.as_ref() == Some(&song) {
                        self.side_panel.show_song_info(&song, tags.map(|t| *t));
                    }
                }
                Job::Added { songs, missing, origin, replace } => self.on_added(ui, songs, missing, origin, replace),
                Job::LinkFetched { url, name, songs, unavailable } => self.on_link(ui, url, name, songs, unavailable),
                Job::EffectsScanned(found) => {
                    self.playback.effects.scanned(found);
                    self.views(|v, md| crate::views::settings_page::refresh(v, ui, md));
                }
                Job::EffectFailed { window, why } => {
                    let text = ui.t(if window { "effects.window.failed" } else { "effects.failed" }, &[("why", &why)]);
                    self.toast(ui, text);
                }
                Job::EffectParams { at, key, params } => {
                    if self.playback.effects.params_read(at, key, params) {
                        self.read_effect_params();
                    }
                    self.views(|v, md| crate::views::settings_page::params_read(v, ui, md));
                }
                Job::LinkSynced { id, version, retried, songs, unavailable, told } => {
                    self.on_link_synced(ui, id, (version, retried), songs, unavailable, told)
                }
                Job::StoreChanged(files) => self.on_store_changed(ui, files),
                Job::PluginCommandDone { path, song, result } => match result {
                    Ok((toast, refresh)) => {
                        if let Some(msg) = toast {
                            self.toast(ui, msg);
                        }
                        if refresh {
                            // The face changed for the song the command was given for; only
                            // when that song is still the open one does it open again.
                            crate::plugins::client::forget_face(&path);
                        }
                        if refresh && song == self.playback.song_version {
                            self.reopen(ui);
                        }
                    }
                    Err(e) => self.toast_error(ui, e),
                },
                Job::Thumbs { idxs, rest } => {
                    for i in idxs {
                        self.side_panel.forget_thumb(i);
                    }
                    self.prefetch_more(ui, rest);
                }
                Job::LinkFailed(url, e) => {
                    let ours = self.lists.importing.remove(url.trim());
                    if ours {
                        if let Some(b) = &mut self.lists.import {
                            b.failed += 1;
                        }
                        self.import_landed(ui, Some(e));
                    } else {
                        self.toast_error(ui, ui.t("toast.import-failed", &[("e", &e)]));
                    }
                }
                Job::AccountsRead => self.view_asks.push(ViewAsk::RefreshPlugins),
                Job::ListRead { list, then } => self.list_read(ui, list.map(|l| *l), *then),
                Job::IndexRead(entries) => self.index_read(entries),
                Job::NotKept(note) => self.toast(ui, note),
                Job::Exported(result) => match result {
                    Ok(n) => self.toast(ui, ui.t("toast.exported", &[("n", &n.to_string())])),
                    Err(e) => self.toast_error(ui, ui.t("toast.export-failed", &[("e", &e)])),
                },
                Job::Plugins(list) => {
                    if self.plugin.registry.adopt(list) {
                        self.plugins_changed(ui);
                    }
                }
                Job::PluginsReloaded(enabled, list) => {
                    self.plugin.registry.reloaded(enabled, list);
                    let on = self.plugin.registry.on_count();
                    self.toast(ui, ui.t("toast.reloaded", &[("on", &on.to_string())]));
                    self.plugins_changed(ui);
                }
                Job::PluginHashed { id, print } => {
                    self.plugin.registry.enable(&id, print);
                    let name = self.plugin.registry.name_of(&id);
                    self.toast(ui, ui.t("toast.enabled", &[("name", &name)]));
                    self.rescan_plugins(ui);
                }
                Job::PluginRemoved { name, result } => {
                    match result {
                        Ok(id) => {
                            self.plugin.registry.disable(&id);
                            self.toast(ui, ui.t("toast.deleted", &[("name", &name)]));
                        }
                        Err(e) => self.toast_error(ui, e),
                    }
                    self.rescan_plugins(ui);
                }
                Job::PluginInstalled(result) => {
                    match result {
                        Ok((id, name)) => {
                            // A reinstall is a change: confirm again.
                            self.plugin.registry.disable(&id);
                            self.toast(ui, ui.t("toast.installed", &[("name", &name)]));
                        }
                        Err(e) => self.toast_install_failed(ui, &e),
                    }
                    self.rescan_plugins(ui);
                }
                Job::LyricStyleInstalled(result) => match result {
                    Ok((id, name)) => {
                        // Chosen at once: what was installed is what is wanted.
                        self.choose_setting(ui, "lyric-style", Some(&id));
                        self.toast(ui, ui.t("toast.installed", &[("name", &name)]));
                    }
                    Err(e) => self.toast_install_failed(ui, &e),
                },
                Job::LyricStyleRemoved(result) => match result {
                    Ok(id) => self.toast(ui, ui.t("toast.deleted", &[("name", &id)])),
                    Err(e) => self.toast_error(ui, e),
                },
                Job::Outputs(systems) => {
                    let output = &mut self.playback.output;
                    output.systems = systems;
                    output.listed = true;
                    let asked = std::mem::take(&mut output.asked);
                    if asked || self.views(|v, _| crate::views::menus::output::is_up(v)) {
                        self.views(|v, m| crate::views::menus::output::open_output_menu(v, ui, m));
                    }
                }
                Job::ConnectBegun(result) => crate::plugins::connect::begun(self, ui, result),
                Job::ConnectPolled(result) => crate::plugins::connect::polled(self, ui, result),
                Job::Options { list } => {
                    let what = self.option_plugin().map(|(_, _, n)| n).unwrap_or_default();
                    match list {
                        Ok(l) if !l.is_empty() => {
                            self.view_asks.push(ViewAsk::OptionMenu { song: self.playback.song_version, values: l })
                        }
                        Ok(_) => self.toast(ui, ui.t("toast.no-options", &[("what", &what)])),
                        Err(e) => self.toast_error(ui, ui.t("toast.options-failed", &[("what", &what), ("e", &e)])),
                    }
                }
                Job::UserLists { plugin, lists, .. } => match lists {
                    Ok(l) if !l.is_empty() => {
                        let items = l
                            .into_iter()
                            .map(|u| {
                                let done = self.link_known(&u.url);
                                PickItem { on: !done, done, url: u.url, name: u.name, count: u.count, group: u.group }
                            })
                            .collect();
                        self.import.set_pick(Some(crate::views::import_dialog::ImportPick { plugin, items }));
                        self.view_asks.push(ViewAsk::ImportDialog { account: self.plugin.account_version });
                    }
                    Ok(_) => self.toast(ui, ui.t("toast.no-lists", &[])),
                    Err(e) => self.toast_error(ui, ui.t("toast.lists-failed", &[("e", &e)])),
                },
                Job::Rescanned(found) => {
                    let fresh = match found {
                        Some(Rescan { changed, gone, fresh }) => {
                            if !changed.is_empty() {
                                self.side_panel.invalidate_rows();
                                for i in changed {
                                    if let Some(song) = self.lists.shown_list.list.song_mut(i) {
                                        song.probed = false;
                                    }
                                }
                                self.lists.tag_fill = None;
                                self.fill_tags(ui);
                            }
                            // Missing files leave; the playing one stays (`remove` keeps it).
                            if !gone.is_empty() {
                                let q = &mut self.lists.shown_list;
                                for p in (0..q.len()).rev() {
                                    if gone.binary_search(&q.list.order[p]).is_ok() {
                                        q.remove(p);
                                    }
                                }
                                self.schedule_save();
                            }
                            fresh
                        }
                        // A source that does not answer (a network drive offline) changes
                        // nothing: the toast says nothing came.
                        None => Vec::new(),
                    };
                    self.on_added(ui, fresh, 0, None, false);
                }
                Job::Tags { tags, rest, total } => self.land_tags(ui, tags, rest, total),
                Job::RowSkipped { list, idx, version } => {
                    if self.rows_current(list, version) {
                        self.side_panel.row_skipped(idx);
                    }
                }
                Job::RowTags { list, idx, version, tags, thumb } => {
                    if self.rows_current(list, version) {
                        if let Some(s) = self.lists.shown_list.list.song_mut(idx) {
                            s.took_tags(tags);
                        }
                        let failed = thumb.is_none()
                            && self.lists.shown_list.list.songs.get(idx).is_some_and(|s| {
                                crate::plugins::client::parse(&s.path).is_some()
                                    && !crate::plugins::client::track_kept(&s.path)
                            });
                        self.side_panel.row_tags(idx, thumb);
                        if failed {
                            self.side_panel.retry_thumb(ui.now(), idx);
                        }
                        self.lists.shown_list.unsaved = true;
                    }
                }
            }
        }
    }
}
