//! The NovaPlayer app on NovaUI: state, the layout, event handling (novaplayer-spec.md).

use crate::library::queue::Queue;
use crate::library::store::Store;
use crate::library::Lyrics;
use crate::playback::engine::{Cmd, Engine, Snapshot};
use crate::player_core::MediaTags;
use crate::views::side_panel::{Page, SidePanel};
use novaui::dsl::Doc;
use novaui::App;
use novaui::*;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub mod command;
mod jobs;
pub mod kept;
mod msg;
pub mod query;
pub use msg::{Answer, Msg};
pub mod lyric_styles;
pub mod settings;
pub(crate) use crate::views::toast::TOAST_INFO;
pub use jobs::{Art, Job, JobSender, Landed, OnlineFace, Rescan, Tasks};
use settings::layer_setting;

use crate::views::now_playing::NowPlaying;

pub struct NowPlayingTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub res: String,
    pub tags: Option<MediaTags>,
    pub lyrics: Option<Lyrics>,
    /// The song's plugin and the layers its lyrics come with (docs/extensions.md, "Lyrics layers").
    pub layers: crate::library::LyricLayers,
}

/// What model and job code asks of views that do not follow state on their own (a flyout to
/// open or rebuild): carried out, in order, at `present`.
pub enum ViewAsk {
    /// The app menu is rebuilt (a sign-in moved on while it is up).
    RefreshPlugins,
    /// The option flyout opens with these values.
    OptionMenu { song: u64, values: Vec<crate::plugins::client::OptionValue> },
    /// The import dialog opens on `import.pick`, while the account is still current.
    ImportDialog { account: u64 },
}

/// What the views own and write: the document, each view's own state. With `Models` it is
/// `Player` split for presentation (`Player::split`): a view writes the tree and its own state
/// and reads the models, never `&mut Player`.
pub struct Views<'a> {
    pub doc: &'a mut Doc,
    /// The document's tweens (`Msg::Layout`).
    pub doc_clock: &'a novaui::Clock,
    pub side_panel: &'a mut SidePanel,
    pub now_playing: &'a mut NowPlaying,
    pub menus: &'a mut crate::views::menus::Menus,
    pub toast: &'a mut crate::views::toast::Toast,
    pub transport: &'a mut crate::views::transport::Transport,
    pub import: &'a mut crate::views::import_dialog::ImportDialog,
    pub settings_page: &'a mut crate::views::settings_page::SettingsPage,
}

impl Views<'_> {
    pub fn id(&self, n: &str) -> Id {
        self.doc.id(n)
    }
}

/// The models, read only (the other half of `Player::split`).
pub struct Models<'a> {
    pub playback: &'a crate::playback::Playback,
    pub lists: &'a crate::library::lists::Lists,
    pub plugin: &'a crate::plugins::Plugins,
    /// A press waiting for its second (`Player::pending`).
    pub pending: Option<&'a command::Confirm>,
    /// The settings page's choices (`settings.txt`).
    pub settings: &'a settings::SettingsFile,
}

impl Models<'_> {
    pub fn playing_queue(&self) -> &Queue {
        self.lists.playing()
    }

    /// A cover was chosen for the playing song.
    pub fn cover_chosen(&self) -> bool {
        self.playing_queue().current().is_some_and(|s| crate::library::covers::chosen().has(&s.path))
    }
}

/// A playlist the import dialog offers (a signed-in plugin account's own list).
pub struct PickItem {
    pub url: String,
    pub name: String,
    pub count: Option<u64>,
    /// The heading it is listed under.
    pub group: String,
    /// Already one of the playlists: shown, not offered again.
    pub done: bool,
    /// Picked for import.
    pub on: bool,
}

pub struct Player {
    pub doc: Doc,
    /// Read-only image/metadata work shares one bounded decoder queue.
    pub assets: novaui::Queue,
    pub side_panel: SidePanel,
    /// The inbox: what other threads report on their own (`Msg::Jobs`).
    pub inbox: JobSender,
    pub jobs_rx: Receiver<Job>,
    /// The app's own requests, not of a song, a list or an account: a plugin command, an
    /// import, a link's sync, a chosen cover, a row's song read for the info page.
    pub tasks: Tasks,
    /// The artwork, the words under it and the backdrop (the only writer of those elements).
    pub now_playing: NowPlaying,
    pub playback: crate::playback::Playback,
    pub lists: crate::library::lists::Lists,
    pub menus: crate::views::menus::Menus,
    pub import: crate::views::import_dialog::ImportDialog,
    /// The settings page over the face (the gear in the title bar).
    pub settings_page: crate::views::settings_page::SettingsPage,
    /// The settings page's choices (`settings.txt`).
    pub settings: settings::SettingsFile,
    /// The kept files changed and not yet written (`kept`).
    pub keeping: kept::Keeping,
    pub toast: crate::views::toast::Toast,
    pub transport: crate::views::transport::Transport,
    pub plugin: crate::plugins::Plugins,
    /// A press waiting for its second (`command::run` forgets it at any other).
    pub pending: Option<command::Confirm>,
    /// View work asked for since the last present.
    pub view_asks: Vec<ViewAsk>,
    /// `ui.strings().version()` the views' texts were last written in (`Player::relabel`).
    pub strings_seen: u64,
    /// The document's tweens (`Msg::Layout`).
    pub doc_clock: novaui::Clock,
    /// Changed by every message but its own: the views present once, in the flush of the turn
    /// (`Msg::Present`), after everything the turn delivered.
    pub presentation: Model<()>,
    _present: Subscription,
    /// The face (`now_playing.track`) changed: `Msg::Track`.
    _track: Subscription,
    /// The plugin controls' context: `Msg::Controls` from each model it reads.
    _controls: [Subscription; 3],
}

/// Tests read the app's texts in zh-CN (the tables in the process's strings).
#[cfg(test)]
pub(crate) fn speak_zh_cn() {
    add_strings(novaui::i18n::global());
    novaui::i18n::global().set_locale("zh-CN");
}

/// The app's string tables (zh-CN, en the fallback) into `strings`.
pub(crate) fn add_strings(strings: &novaui::i18n::Strings) {
    strings.add("zh-CN", include_str!("../../ui/zh-CN.txt")).unwrap_or_else(|e| panic!("zh-CN.txt: {e}"));
    strings.add("en", include_str!("../../ui/en.txt")).unwrap_or_else(|e| panic!("en.txt: {e}"));
}

impl Player {
    pub fn new(ui: &mut Ui) -> Self {
        let src = ui_source().unwrap_or_else(|e| panic!("{e}"));
        let sheet = novaui::dsl::parse(&src).unwrap_or_else(|e| panic!("player.nui: {e}"));
        // The app's strings (novaui `i18n`, en the fallback), in the kept language before any
        // text is made: the workers' too (`novaui::i18n::t`).
        add_strings(ui.strings());
        let settings = settings::SettingsFile::load();
        crate::views::settings_page::set_locale(ui, settings.get("locale"));
        let w2 = ui.waker(Msg::Engine);
        let accounts: Arc<crate::plugins::accounts::Accounts> = Default::default();
        let effects = crate::playback::effects::EffectsModel::player();
        let changed = ui.waker(Msg::Effects);
        effects.chain.watch(move || changed.wake());
        let engine = Engine::new(Arc::new(move || w2.wake()), accounts.clone(), effects.chain.clone());
        // The kept volume goes to the engine before any sound opens: no blip at another level.
        let kept_gain = crate::playback::gain::KeptGain::player();
        let (output, output_note) = crate::playback::output::Output::install();
        engine.send(Cmd::Volume(kept_gain.gain.level));
        engine.send(Cmd::ReplayGain(crate::views::settings_page::replay_gain_of(settings.get("replay-gain"))));
        let (jobs_tx, jobs_rx) = channel();
        let inbox = JobSender::new(jobs_tx, ui.waker(Msg::Jobs));
        let mut presentation = Model::new(());
        let present = presentation.subscribe(Msg::Present);
        let mut now_playing = NowPlaying::new(ui);
        now_playing.layers_off = novaui::Model::new(layer_setting::get());
        let track = now_playing.track.subscribe(Msg::Track);
        let mut side_panel = SidePanel::new(ui, layer_setting::get());
        let mut play_state = Model::new(());
        let controls = [
            side_panel.shown.subscribe(Msg::Controls),
            play_state.subscribe(Msg::Controls),
            now_playing.track.subscribe(Msg::Controls),
        ];
        let assets = ui.queue(32);
        Player {
            doc: Doc::new(sheet),
            assets,
            side_panel,
            inbox,
            jobs_rx,
            tasks: Default::default(),
            playback: crate::playback::Playback {
                stand_in_clock: ui.clock(Msg::StandIn),
                tasks: Default::default(),
                engine,
                snap: Snapshot::default(),
                song_version: 0,
                last_version: 0,
                handed: Vec::new(),
                handing: false,
                opened: false,
                state: play_state,
                wants_play: false,
                duration_hint: None,
                seeks_sent: 0,
                error_shown: None,
                fallback: Default::default(),
                volume: kept_gain.gain.level,
                kept_gain,
                effects,
                gain_touched: false,
                output,
                output_note,
                no_device_told: 0,
                held_seen: 0,
                devices_listed: 0,
                #[cfg(any(test, feature = "test-support"))]
                pinned_position: None,
            },
            lists: crate::library::lists::Lists {
                store: Store::open(),
                shown_list: Queue::default(),
                playing_list: None,
                list_version: 1,
                sort_waiting: None,
                tag_fill: None,
                tasks: Default::default(),
                switching: None,
                adding: Default::default(),
                synced: Default::default(),
                import: None,
                importing: Default::default(),
            },
            now_playing,
            menus: crate::views::menus::Menus::new(ui),
            import: Default::default(),
            settings_page: crate::views::settings_page::SettingsPage::new(ui),
            settings,
            keeping: kept::Keeping::new(ui.clock(Msg::Keep)),
            toast: crate::views::toast::Toast::new(ui),
            transport: crate::views::transport::Transport::new(ui),
            plugin: crate::plugins::Plugins {
                registry: crate::plugins::registry::Registry::open_watched(
                    crate::data_dir().map(|d| d.join("plugins")),
                    {
                        let checked = ui.waker(Msg::PluginsChecked);
                        move || checked.wake()
                    },
                ),
                plugin_controls: Default::default(),
                prefetch: None,
                prefetch_waiting: None,
                account_tasks: Default::default(),
                errands: Default::default(),
                net_busy: Default::default(),
                connection: None,
                form: Default::default(),
                account_version: 0,
                accounts,
            },
            pending: None,
            view_asks: Vec::new(),
            strings_seen: ui.strings().version(),
            doc_clock: ui.clock(Msg::Layout),
            presentation,
            _present: present,
            _track: track,
            _controls: controls,
        }
    }

    pub fn id(&self, n: &str) -> Id {
        self.doc.id(n)
    }

    /// Changes the output choice (`f`), tells why it was not kept, and shows the route taken
    /// (the `asio` mark beside the volume).
    pub fn choose_output(&mut self, ui: &mut Ui, f: impl FnOnce(&mut crate::playback::output::Output) -> bool) {
        if f(&mut self.playback.output) {
            self.keeping.changed(kept::Kept::Output);
        }
        self.show_route(ui);
    }

    /// The route the outputs were sent to, into the layout (written where it changes).
    pub(crate) fn show_route(&mut self, ui: &mut Ui) {
        self.doc.set(ui, "asio", self.playback.output.asio());
    }

    /// Runs a view with its half of the split (a controller opening a flyout).
    pub fn views<R>(&mut self, f: impl FnOnce(&mut Views, &Models) -> R) -> R {
        let (mut v, m) = self.split();
        f(&mut v, &m)
    }

    /// The views, to write, and the models they read (see `Views`).
    pub fn split(&mut self) -> (Views<'_>, Models<'_>) {
        (
            Views {
                doc: &mut self.doc,
                doc_clock: &self.doc_clock,
                side_panel: &mut self.side_panel,
                now_playing: &mut self.now_playing,
                menus: &mut self.menus,
                toast: &mut self.toast,
                transport: &mut self.transport,
                import: &mut self.import,
                settings_page: &mut self.settings_page,
            },
            Models {
                playback: &self.playback,
                lists: &self.lists,
                plugin: &self.plugin,
                pending: self.pending.as_ref(),
                settings: &self.settings,
            },
        )
    }

    pub(crate) fn restore(&mut self, ui: &mut Ui) {
        let inbox = self.inbox.clone();
        self.lists.store.watch(move |files| {
            inbox.send(Job::StoreChanged(files));
        });
        match self.lists.store.active.filter(|id| self.lists.store.entry(*id).is_some()) {
            // Read on the store's thread; shown when it lands (`restored`).
            Some(id) => {
                let inbox = self.inbox.clone();
                self.lists.store.read(id, move |list| {
                    let then = Box::new(crate::library::lists::ListFor::Restore(id));
                    inbox.send(Job::ListRead { list: list.map(Box::new), then });
                });
            }
            None => self.restored(ui, None),
        }
    }

    /// The list kept active as read (none: there was none, or it could not be read): shown, and
    /// its song opened paused where it was.
    pub(crate) fn restored(&mut self, ui: &mut Ui, loaded: Option<(u32, crate::library::store::List)>) {
        match loaded {
            Some((id, list)) => {
                let resume = list.resume.then_some(list.position);
                self.lists.shown_list.adopt(list, Some(id));
                if let Some(position) = resume {
                    self.open_current(ui, position, false);
                }
            }
            None => {
                self.lists.show_default(ui);
            }
        }
        self.shown_list_moved();
        self.fill_tags(ui);
        self.side_panel.open_queue_if_empty(&self.lists.shown_list);
        self.sync_link(ui, false);
    }

    /// The lyrics the side panel shows, with the layers the player knows laid under their lines
    /// (shown or hidden by the side panel: `layers_off`).
    pub fn shown_lyrics(&self) -> Option<Lyrics> {
        let f = self.now_playing.track.as_ref()?;
        f.lyrics.clone().map(|l| l.with_layers(&f.layers.layers))
    }

    /// The lyrics page's layer buttons: a layer (`trans`, `roma`) shows or goes, under every
    /// line at once (the side panel eases it), and the choice is kept.
    pub fn toggle_layer(&mut self, layer: &str) {
        if !self.now_playing.layers_off.edit().remove(layer) {
            self.now_playing.layers_off.edit().insert(layer.to_owned());
        }
        self.keeping.changed(kept::Kept::Layers);
        self.side_panel.set_layers_off(&self.now_playing.layers_off);
    }

    /// `plugin.<name>` from a control: the plugin's `command` call with the context, off the
    /// frame; its answer may say a toast and that the song's face is to be asked again.
    pub fn plugin_command(&mut self, ui: &mut Ui, plugin: &str, name: &str) {
        let (plugin, name) = (plugin.to_owned(), name.to_owned());
        let uri = self.playing_queue().current_plugin().map(|(_, u)| u).unwrap_or_default();
        let path = self.playing_queue().current().map(|s| s.path.clone()).unwrap_or_default();
        let (acc, song) = (self.plugin.accounts.clone(), self.playback.song_version);
        let task = ui.spawn_waiting(
            move |_| crate::plugins::client::command(&acc, &plugin, &name, &uri),
            move |result| Landed(Job::PluginCommandDone { path, song, result }),
        );
        self.tasks.hold(Some(task));
    }

    /// The installed plugins changed (on, off, installed, deleted, reloaded, checked; the
    /// registry has synced the host and the face cache, `Registry::scan`): their controls mount
    /// again, their accounts are read, the views that show them follow.
    pub fn plugins_changed(&mut self, ui: &mut Ui) {
        self.plugin.plugin_controls.clear();
        self.read_accounts(ui);
        self.view_asks.push(ViewAsk::RefreshPlugins);
    }

    /// The controls plugins add, mounted in their slots while their conditions hold. Runs on its
    /// causes: `Msg::Controls` (the page shown, what plays, the face), the plugins changing
    /// (`plugins_changed`), and once at the start (`init_view`).
    pub fn sync_plugin_controls(&mut self, ui: &mut Ui) {
        let shown = *self.side_panel.shown;
        let page = match (shown.open, shown.info_up, shown.page) {
            (false, ..) => "",
            (true, true, _) => "info",
            (true, false, Page::Lyrics) => "lyrics",
            (true, false, Page::Queue) => "queue",
        };
        // The playing song (docs/extensions.md, "Context keys"): the open one, none before a
        // song opens. It moves only when a song opens or closes (`playback.state`).
        let song = self.playback.opened.then(|| self.playing_queue().current()).flatten();
        let song_plugin = song.and_then(|s| crate::plugins::client::parse(&s.path)).map(|(p, _)| p).unwrap_or_default();
        let layers = self
            .now_playing
            .track
            .as_ref()
            .filter(|f| f.layers.plugin == song_plugin)
            .map_or(Vec::new(), |f| f.layers.layers.iter().map(|l| l.id.clone()).collect());
        let ctx = crate::plugins::controls::Context { page, playing: self.playback.snap.playing, song_plugin, layers };
        let slot = self.doc.get("slot-lyrics-tools");
        let plugin = &mut self.plugin;
        plugin.plugin_controls.sync(ui, &mut self.doc, &|| plugin.registry.controls(), &ctx, &|n| {
            (n == "lyrics-tools").then_some(slot).flatten()
        });
    }
}

/// The sheet's file a debug build watches and reloads while running (novaui `Doc::watch`):
/// `NOVAPLAYER_UI_FILE`, else the checkout's `ui/player.nui` it was built from.
#[cfg(debug_assertions)]
pub(crate) fn ui_file() -> std::path::PathBuf {
    std::env::var_os("NOVAPLAYER_UI_FILE")
        .map_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/ui/player.nui").into(), Into::into)
}

fn ui_source() -> Result<String, String> {
    // A release is self-contained. Never silently load a newer checkout into an older executable.
    // Live source editing is an explicit developer opt-in, not a fallback based on build-machine paths.
    match std::env::var_os("NOVAPLAYER_UI_FILE") {
        Some(path) => {
            std::fs::read_to_string(&path).map_err(|e| format!("cannot read NOVAPLAYER_UI_FILE {:?}: {e}", path))
        }
        None => Ok(include_str!("../../ui/player.nui").to_string()),
    }
}

impl App for Player {
    fn init(&mut self, ui: &mut Ui) {
        self.init_view(ui);
        self.read_accounts(ui);
        self.restore(ui);
        self.present_pending(ui);
        self.sync_queue(ui, true);
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        // A task's result: delivered once, owned here.
        let ev = match ev {
            Event::Message(m) if m.is::<Landed>() => {
                if let Some(Landed(job)) = m.downcast::<Landed>().ok().and_then(|r| std::rc::Rc::try_unwrap(r).ok()) {
                    self.land(ui, job);
                }
                return self.presentation.notify();
            }
            ev => ev,
        };
        if let Event::Message(m) = &ev {
            match m.downcast_ref::<Msg>() {
                Some(Msg::Present) => return self.turn(ui),
                Some(Msg::Rescue) => {
                    self.rescue();
                    return crate::crash::rescued();
                }
                Some(Msg::Track) => self.now_playing.layout_current = false,
                // The controls follow here, not in the present: no second present for them.
                Some(Msg::Controls) => return self.sync_plugin_controls(ui),
                Some(Msg::Connect) => crate::plugins::connect::due(self, ui),
                // A plugin that was being checked is now on, or changed.
                Some(Msg::PluginsChecked) => self.rescan_plugins(ui),
                _ => {}
            }
            if let Some(a) = m.downcast_ref::<crate::app::Answer>() {
                crate::app::command::answered(self, ui, a.clone());
            }
        }
        // Whatever came changes what the views show: they present once, in the flush.
        self.presentation.notify();
        crate::views::input::handle(self, ui, ev);
    }

    fn after_layout(&mut self, ui: &mut Ui) {
        self.after_layout_phase(ui);
    }

    fn nc_hit(&self, ui: &Ui, x: i32, y: i32) -> NcHit {
        for name in ["queue-rail", "lyrics-rail"] {
            let id = self.id(name);
            if ui.tree.el(id).visual().visible && ui.tree.rect(id).contains(x, y) {
                return NcHit::ClientControl;
            }
        }
        let s = ui.scale();
        if y as f32 >= 32.0 * s || ui.window.fullscreen() {
            return NcHit::Client;
        }
        for (n, h) in [("close", NcHit::Close), ("max", NcHit::Maximize), ("min", NcHit::Minimize)] {
            if ui.tree.rect(self.doc.id(n)).contains(x, y) {
                return h;
            }
        }
        if ui.tree.rect(self.doc.id("app-menu")).contains(x, y) {
            return NcHit::Client;
        }
        // Menus opened over the title bar keep their clicks.
        for m in ["flyout", "plugin-flyout"] {
            let id = self.doc.id(m);
            if ui.tree.el(id).layout().display != Display::None && ui.tree.rect(id).contains(x, y) {
                return NcHit::Client;
            }
        }
        NcHit::Caption
    }
}

impl Player {
    /// The player's work for one turn: what jobs and the engine sent, then the views' present
    /// and the deadlines it leaves. Runs once per turn that delivered the player anything, in
    /// the flush (`Msg::Present`), after everything the turn delivered.
    fn turn(&mut self, ui: &mut Ui) {
        self.drain_jobs(ui);
        let (was_ended, was_playing) = (self.playback.snap.ended, self.playback.snap.playing);
        let fresh = self.playback.engine.snapshot();
        if fresh.version != self.playback.song_version {
            // The sound went on into the song handed over to follow: it is the one playing now.
            let followed = self.playback.handed.iter().find(|(_, n)| n.version == fresh.version).cloned();
            if let Some((pos, next)) = followed {
                self.show_followed(ui, pos, next);
            }
        }
        if fresh.version == self.playback.song_version {
            // What changed is read off the two snapshots before the old one goes: no copy of it.
            let stream_landed = self.playback.snap.stream.is_none() && fresh.stream.is_some();
            let option_moved = self.playback.snap.option != fresh.option;
            // A snapshot taken before the engine carried out the latest seek still has the old
            // position: the bar would flash back to it. Keep the asked position until it has.
            let stale = fresh.seeks < self.playback.seeks_sent;
            let (pos, at) = (self.playback.snap.position, self.playback.snap.at);
            self.playback.snap = fresh;
            if stale {
                self.playback.snap.position = pos;
                self.playback.snap.at = at;
                self.playback.snap.ended = false;
            }
            if self.playback.snap.duration.is_none() {
                self.playback.snap.duration = self.playback.duration_hint;
            }
            if stream_landed {
                self.merge_stream();
            } else if option_moved {
                self.words_again();
            }
        }
        // Failure wins over EOF, even if a backend reports both. Advancing first used to
        // overwrite the failed snapshot with the next song's opening state, lose the error,
        // and spin through the whole list. Keep the error in the snapshot; only its toast is
        // deduplicated. A restored paused selection must not auto-start another song either.
        if self.playback.snap_is_open() {
            self.tell_fallback(ui);
            if let Some(e) = self.playback.snap.error.clone() {
                self.playback.snap.ended = false;
                self.playback.snap.buffering = false;
                self.playback.snap.at = None;
                self.playback.close();
                if self.playback.error_shown != Some(self.playback.song_version) {
                    self.playback.error_shown = Some(self.playback.song_version);
                    self.playback.engine.send(Cmd::Stop);
                    self.toast_error(ui, ui.t("toast.play-failed", &[("e", &e.to_string())]));
                }
            } else if self.playback.snap.ended
                && !was_ended
                && self.playback.opened
                && !self.playback.snap.opening
                && self.playback.wants_play
            {
                self.on_ended(ui);
            }
        }
        self.hand_next();
        // The engine paused the song by itself (its device went, or there was none): the play
        // button and the kept state follow.
        if self.playback.snap.held > self.playback.held_seen {
            self.playback.held_seen = self.playback.snap.held;
            if self.playback.wants_play {
                self.playback.rest();
                self.schedule_save();
            }
        }
        // A play that found no sound device: the song stays held where it was.
        if self.playback.snap.no_device > self.playback.no_device_told {
            self.playback.no_device_told = self.playback.snap.no_device;
            self.playback.rest();
            self.toast_error(ui, ui.t("toast.no-device", &[]));
        }
        // A device came or went: the output flyout, if up, lists them again.
        let devices = self.playback.engine.devices.load(std::sync::atomic::Ordering::Relaxed);
        if devices != self.playback.devices_listed {
            self.playback.devices_listed = devices;
            if self.views(|v, _| crate::views::menus::output::is_up(v)) {
                self.list_outputs(ui);
            }
        }
        self.stand_in_track(ui);
        if self.playback.snap.playing != was_playing {
            self.playback.state.notify();
            self.schedule_save();
        }
        // The network is the player's while a song opens or buffers.
        let busy = u32::from(self.playback.snap.waiting());
        self.plugin.net_busy.store(busy, std::sync::atomic::Ordering::Relaxed);
        self.prefetch_thumbs(ui);
        if let Some(n) = self.playback.output_note.take() {
            self.toast(ui, n);
        }
        self.present(ui);
        self.effects_turn(ui);
        self.note_volume();
        for what in self.keeping.due(ui.now()) {
            self.keep(what);
        }
    }

    /// The volume moved (`gain_touched`): kept.
    fn note_volume(&mut self) {
        if std::mem::take(&mut self.playback.gain_touched) {
            self.playback.kept_gain.set(self.playback.volume);
            self.keeping.changed(kept::Kept::Volume);
        }
    }

    /// The effects chain changed (a parameter, an effect's state, a failure: from here, its
    /// window, the chain's thread): an effect's level shown reads its parameters again; the chain
    /// is kept (`kept::Kept::Effects`).
    fn effects_turn(&mut self, ui: &mut Ui) {
        let effects = &mut self.playback.effects;
        let changes = effects.chain.changes();
        if changes != effects.seen {
            effects.seen = changes;
            self.keeping.changed(kept::Kept::Effects);
            // An effect came or went (one added lands from the plugin host thread): the
            // Effects page lists the chain again.
            let keys: Vec<_> = effects.chain.list().iter().map(|e| e.key).collect();
            if keys != effects.listed {
                effects.listed = keys;
                self.views(|v, md| crate::views::settings_page::refresh(v, ui, md));
            }
            self.read_effect_params();
        }
    }
}

impl Player {
    /// Saves what must outlive the process: every kept file changed and not yet written, the
    /// queue and its position, the volume and the effects chain as they are, then waits for the
    /// store's thread to have written them. On close (`Drop`) and on a panic (`crate::crash`).
    pub fn rescue(&mut self) {
        self.note_volume();
        self.keeping.changed(kept::Kept::Lists);
        self.keeping.changed(kept::Kept::Volume);
        self.keeping.changed(kept::Kept::Effects);
        for what in self.keeping.all() {
            self.keep(what);
        }
        self.lists.store.flush();
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        crate::crash::forget(self);
        self.rescue();
    }
}
