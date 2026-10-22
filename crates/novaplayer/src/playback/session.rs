//! Opening songs, the face, transport (spec, "Transport under the art" and "Queue page").
use crate::app::*;
use crate::library::queue::Queue;
use crate::library::{self, Lyrics};
use crate::playback::engine::{Cmd, NextSong, Snapshot};
use crate::player_core::MediaTags;
use crate::views::now_playing::Words;
use novaui::*;
use std::time::Duration;

/// Seconds a switched song's words and picture have to land (from the switch, whatever the
/// sound does) before the stand-in goes up in place of what is still standing: about the
/// crossfade's own length, so a face that lands within it fades once, old to new, with no
/// placeholder between.
pub(crate) const STAND_IN_WINDOW: f64 = 0.3;

impl Player {
    pub fn open_current(&mut self, ui: &mut Ui, at: Duration, play: bool) {
        let Some(song) = self.playing_queue().current().cloned() else { return };
        let version = self.playback.new_version();
        self.playback.snap = Snapshot { version, opening: true, ..Default::default() };
        self.show_song(ui, &song, version, at, play);
        let span = song.span;
        let duration = song.length();
        self.playback.engine.send(Cmd::Open {
            version,
            path: song.path.clone(),
            span,
            at,
            play,
            duration: duration.filter(|_| span.is_none()),
            rate: None,
        });
    }

    /// How long one song overlaps the next (the setting 淡入淡出); zero: gapless.
    fn crossfade(&self) -> Duration {
        crate::views::settings_page::crossfade_of(self.settings.get("crossfade"))
    }

    /// The sound went on into `next`, handed over at `pos` of the playing list (`hand_next`):
    /// the list moves there and the song shows as the one playing. No open: it plays already.
    pub(crate) fn show_followed(&mut self, ui: &mut Ui, pos: usize, next: NextSong) {
        self.tell_plugin("end", self.playback.snap.position);
        // The list may have moved since it was handed over: the song is found where it is now.
        let queue = self.playing_queue();
        let at = |p: usize| queue.song_at(p).is_some_and(|s| s.is(&next.path, next.span));
        let pos = if at(pos) { Some(pos) } else { (0..queue.len()).find(|&p| at(p)) };
        let Some(pos) = pos else {
            // Gone from the list: what it now plays next opens the ordinary way.
            match self.playing_queue().next_pos() {
                Some(p) => self.play_pos(ui, p, true),
                None => self.stop(ui),
            }
            return;
        };
        self.playing_queue_mut().set_pos(pos);
        if self.lists.playing_list.is_none() {
            self.side_panel.follow_current = true;
        }
        let Some(song) = self.playing_queue().current().cloned() else { return };
        self.show_song(ui, &song, next.version, Duration::ZERO, true);
    }

    /// The song to go on into without a gap when the playing one ends (`Queue::follow_pos`; none
    /// while the list does not play), handed to the engine whenever it changes.
    pub(crate) fn hand_next(&mut self) {
        let queue = self.playing_queue();
        let next = queue
            .follow_pos()
            .filter(|_| self.playback.opened && self.playback.wants_play && self.playback.snap.error.is_none())
            .and_then(|p| queue.song_at(p).map(|s| (p, s)));
        let live = self.playback.handed.last().filter(|_| self.playback.handing);
        // Consecutive songs of one album keep their joint: they follow gaplessly.
        let crossfade = match (queue.current(), next) {
            (Some(now), Some((_, s))) if !now.same_album(s) => self.crossfade(),
            _ => Duration::ZERO,
        };
        let same = match (next, live) {
            (Some((p, s)), Some((q, n))) => p == *q && s.is(&n.path, n.span) && n.crossfade == crossfade,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        // One taken back stays in `handed`: the sound may have gone on into it meanwhile.
        let Some((pos, song)) = next else {
            self.playback.engine.send(Cmd::Next(None));
            self.playback.handing = false;
            return;
        };
        let span = song.span;
        let duration = song.length();
        let mut next = NextSong { version: 0, path: song.path.clone(), span, duration, crossfade };
        next.version = self.playback.new_version();
        self.playback.engine.send(Cmd::Next(Some(next.clone())));
        self.playback.handed.push((pos, next));
        self.playback.handing = true;
    }

    /// Shows `song` as the one playing under `version`: the face, its words and picture, the
    /// window's title. Whether its sound is opened is the caller's matter.
    fn show_song(&mut self, ui: &mut Ui, song: &crate::library::Song, version: u64, at: Duration, play: bool) {
        self.now_playing.song_changed(song, self.playback.song_version, version, ui.now());
        self.playback.song_version = version;
        self.playback.handed.clear();
        self.playback.handing = false;
        // Known when the song's own face job lands (`Job::Track`); an online song never is.
        self.now_playing.cover_editable = false;
        self.playing_queue_mut().list.resume = true;
        self.playing_queue_mut().list.position = at;
        self.playback.open_as(play);
        self.playback.duration_hint = song.length();
        // What is up stays until the song's own words and picture land, or until the change's
        // window is out (`stand_in_track`); with nothing up, nothing stands under the artwork.
        self.playback.stand_in_clock.at(ui.now() + STAND_IN_WINDOW);
        if self.now_playing.track.is_none() {
            self.set_track_text("", "", "", "", false);
        }
        let path = song.path.clone();
        // A song of a plugin's list reports its plays (services that ask for them).
        crate::plugins::client::play_event(&self.plugin.accounts, &path, "start", at);
        let task = if crate::plugins::client::parse(&path).is_some() {
            let song = song.clone();
            // The words and lyrics land as soon as the plugin has them; the picture (a fetch of
            // its own) follows as a task of its own (`fetch_online_cover`).
            Some(ui.spawn_waiting(
                move |_| {
                    use crate::plugins::client::{face_cover, face_words, TrackInfo, Words};
                    let plugin = crate::plugins::client::parse(&song.path).map(|(p, _)| p).unwrap_or_default();
                    let face = |f: &TrackInfo, art: Art, fetch| {
                        Job::OnlineTrack(Box::new(OnlineFace {
                            path: song.path.clone(),
                            tags: Some(f.tags.clone()),
                            art,
                            lyrics: f.lyrics.as_deref().and_then(Lyrics::parse),
                            layers: crate::library::LyricLayers { plugin: plugin.clone(), layers: f.layers.clone() },
                            fetch,
                        }))
                    };
                    let picture = |f: &TrackInfo| f.cover.as_deref().and_then(|b| library::decode_cover(b, 1040));
                    match face_words(&song, false) {
                        Some(Words::Kept(f)) => face(&f, Art::Now(picture(&f)), None),
                        Some(Words::Asked { info, cover_at: None }) => {
                            let f = face_cover(&song.path, info, None);
                            face(&f, Art::Now(None), None)
                        }
                        Some(Words::Asked { info, cover_at }) => {
                            face(&info, Art::Later, Some((info.clone(), cover_at)))
                        }
                        None => Job::OnlineTrack(Box::new(OnlineFace {
                            path: song.path.clone(),
                            tags: None,
                            art: Art::Now(None),
                            lyrics: None,
                            layers: Default::default(),
                            fetch: None,
                        })),
                    }
                },
                Landed,
            ))
        } else {
            ui.spawn(
                novaui::Priority::Visible,
                move |_| {
                    let mut tags = crate::playback::engine::probe(&path);
                    let cover_editable = tags.as_ref().is_none_or(|t| t.cover.is_none()) && path.exists();
                    let bytes = crate::library::covers::resolve(&path, tags.as_mut().and_then(|t| t.cover.take()));
                    let cover = bytes.as_deref().and_then(|b| library::decode_cover(b, 1040));
                    let lyrics = tags
                        .as_ref()
                        .and_then(|t| t.lyrics.as_deref())
                        .and_then(Lyrics::parse)
                        .or_else(|| library::sidecar(&path));
                    Job::Track { tags, cover, lyrics, cover_editable }
                },
                Landed,
            )
        };
        // Another song's face, still on its way, is called off.
        self.playback.tasks.clear();
        self.playback.tasks.hold(task);
        let t = song.title().to_string();
        ui.window.set_title(format!("{t} - NovaPlayer"));
        self.schedule_save();
    }

    pub fn play_pos(&mut self, ui: &mut Ui, pos: usize, play: bool) {
        self.playing_queue_mut().set_pos(pos);
        self.open_current(ui, Duration::ZERO, play);
        if self.lists.playing_list.is_none() {
            self.side_panel.follow_current = true;
        }
    }

    /// The list the loaded song plays from (the shown one unless a switch detached it).
    pub fn playing_queue(&self) -> &Queue {
        self.lists.playing()
    }

    pub fn playing_queue_mut(&mut self) -> &mut Queue {
        self.lists.playing_mut()
    }

    /// A row of the shown list pressed: it plays, and the shown list becomes the one that
    /// plays (the list that played keeps where it was, to come back to).
    pub fn play_row(&mut self, ui: &mut Ui, pos: usize) {
        if self.lists.playing_list.is_some() {
            self.save_now();
            self.lists.playing_list = None;
            if let Some(id) = self.lists.shown_list.id {
                self.lists.store.set_active(Some(id));
            }
        } else if pos == self.lists.shown_list.pos && self.playback.opened {
            return;
        }
        self.play_pos(ui, pos, true);
    }

    /// Shows `list` (kept as playlist `id`) in the queue page. A loaded song from another list
    /// goes on playing from that list; with nothing loaded the shown list's own song is selected
    /// at `at`, not played (spec, "Scope and start"). Replacing the very list that plays (a re-import of it)
    /// stops it, as before.
    pub(crate) fn show_list(
        &mut self,
        ui: &mut Ui,
        list: crate::library::store::List,
        id: u32,
        at: Duration,
        select_first: bool,
    ) {
        self.save_now();
        let plays = self.playing_queue().id;
        let keep = self.playback.opened && plays != Some(id);
        if keep && self.lists.playing_list.is_none() {
            // The list that plays steps out of the page and goes on playing.
            let fresh = Queue {
                order_mode: self.lists.shown_list.order_mode,
                version: self.lists.shown_list.version + 1,
                order_version: self.lists.shown_list.order_version + 1,
                ..Default::default()
            };
            self.lists.playing_list = Some(std::mem::replace(&mut self.lists.shown_list, fresh));
        } else if !keep {
            self.stop(ui);
        }
        let version = self.lists.shown_list.version.max(self.lists.playing_list.as_ref().map_or(0, |p| p.version));
        self.lists.shown_list.adopt(list, Some(id));
        self.lists.shown_list.version = self.lists.shown_list.version.max(version + 1);
        if !keep {
            self.lists.store.set_active(Some(id));
        }
        self.shown_list_moved();
        self.fill_tags(ui);
        if !keep && self.lists.shown_list.current().is_some() {
            if select_first {
                self.lists.shown_list.set_pos(0);
            }
            self.open_current(ui, at, false);
        }
    }

    /// Puts the words under the artwork (see `NowPlaying::set_words`).
    /// With nothing open, the words under the artwork say so (`np.idle`).
    pub(crate) fn show_idle_words(&mut self, ui: &Ui, fade: bool) {
        self.set_track_text(&ui.t("np.idle", &[]), "", "", "", fade);
    }

    pub(crate) fn set_track_text(&mut self, title: &str, artist: &str, album: &str, res: &str, fade: bool) {
        let option = self.playback.snap.option.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
        let (res_main, res_note) = self.res_texts(res);
        let w = Words {
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            res: res.into(),
            res_main,
            res_note,
            option,
        };
        self.now_playing.set_words(w, fade);
    }

    /// The open song's cover (or its placeholder, `None`) is shown, stamped as the open song's.
    pub(crate) fn show_open_cover(&mut self, cover: Option<library::Cover>) {
        self.now_playing.cover_version = self.playback.song_version;
        self.show_cover(cover);
    }

    pub fn apply_now_playing(
        &mut self,
        tags: Option<MediaTags>,
        art: Art,
        lyrics: Option<Lyrics>,
        layers: crate::library::LyricLayers,
    ) {
        // The words shown are the open song's.
        self.now_playing.track_version = self.playback.song_version;
        let song = self.playing_queue().current().cloned();
        let title = tags
            .as_ref()
            .and_then(|t| t.title.clone())
            .filter(|t| !t.trim().is_empty())
            .or_else(|| song.as_ref().map(|s| s.title().to_string()))
            .unwrap_or_default();
        let (artist, album) = match (&tags, &song) {
            (_, Some(s)) if s.span.is_some() => {
                (s.tags.artist.clone().unwrap_or_default(), s.tags.album.clone().unwrap_or_default())
            }
            (Some(t), _) => (t.artist.clone().unwrap_or_default(), t.album.clone().unwrap_or_default()),
            _ => (String::new(), String::new()),
        };
        let res = tags.as_ref().and_then(|t| t.resolution.as_ref()).map(|r| r.label()).unwrap_or_default();
        if self.playback.duration_hint.is_none() && song.as_ref().is_some_and(|s| s.span.is_none()) {
            self.playback.duration_hint = tags.as_ref().and_then(|t| t.duration);
        }
        // The artwork's crossfade starts first (below), so the words share its clock.
        let words = (title.clone(), artist.clone(), album.clone(), res.clone());
        // The row learns what the file says.
        let shown = self.lists.playing_list.is_none();
        let pq = self.playing_queue_mut();
        if let (Some(t), Some(&i)) = (&tags, pq.list.order.get(pq.pos)) {
            std::sync::Arc::make_mut(&mut pq.list.songs_mut()[i]).took_tags(Some(crate::library::Tags::from(t)));
            if shown {
                self.side_panel.row_changed(i);
            }
        }
        if let Art::Now(cover) = art {
            self.show_open_cover(cover);
        }
        self.set_track_text(&words.0, &words.1, &words.2, &words.3, true);
        *self.now_playing.track.edit() = Some(NowPlayingTrack { title, artist, album, res, tags, lyrics, layers });

        let shown = self.shown_lyrics();
        self.side_panel.set_lyrics(shown);
        self.side_panel.set_info(self.now_playing.track.as_ref(), song.as_ref());
        self.arm_option();
        // An online song's stream may have been probed before its face landed.
        self.merge_stream();
    }

    /// A switch put the previous song's words and artwork on hold while the new song's own are
    /// fetched (words and lyrics first, then the picture; the sound is its own matter and does
    /// not decide this). Whatever lands within the change's window (`STAND_IN_WINDOW`, from the
    /// switch) crossfades in directly, old to new. What is still missing when the window is out
    /// gives way to its stand-in, so the old song's picture or words never stand over the new
    /// song: the list's own words, the placeholder art (the backdrop follows the artwork). The
    /// real one crossfades in when it lands. A song that failed to open stands in at once.
    pub(crate) fn stand_in_track(&mut self, ui: &mut Ui) {
        let version = self.playback.song_version;
        if !self.playback.opened {
            return;
        }
        let words_late = self.now_playing.track_version != version;
        let cover_late = self.now_playing.cover_version != version;
        if !words_late && !cover_late {
            return;
        }
        let failed = self.playback.snap.version == version && self.playback.snap.error.is_some();
        let since = match self.now_playing.switched_at {
            Some((s, t)) if s == version => t,
            _ => {
                self.now_playing.switched_at = Some((version, ui.now()));
                ui.now()
            }
        };
        if !failed && ui.now() < since + STAND_IN_WINDOW {
            self.playback.stand_in_clock.at(since + STAND_IN_WINDOW);
            return;
        }
        if cover_late {
            self.show_open_cover(None);
        }
        if !words_late {
            return;
        }
        let Some(song) = self.playing_queue().current().cloned() else { return };
        let tags = MediaTags::from(&song.tags);
        // Each stands in by itself: the words of a song whose picture is still on its way go
        // up with the picture's own placeholder only when it, too, is late.
        self.apply_now_playing(Some(tags), Art::Later, None, Default::default());
    }

    /// An online song's face learns what its stream is (codec, rate, depth, bit rate, size):
    /// the service says nothing of it, the stream itself does. Whichever lands second merges.
    pub(crate) fn merge_stream(&mut self) {
        if !self.playback.snap_is_open() {
            return;
        }
        let Some(st) = self.playback.snap.stream.clone() else { return };
        let Some(f) = self.now_playing.track.edit().as_mut() else { return };
        let t = f.tags.get_or_insert_with(|| MediaTags { has_audio: true, ..Default::default() });
        if t.resolution.is_some() {
            return;
        }
        t.resolution = st.resolution;
        t.container = t.container.take().or(st.container);
        t.file_size = t.file_size.or(st.file_size);
        f.res = t.resolution.as_ref().map(|r| r.label()).unwrap_or_default();
        self.words_again();
        let song = self.playing_queue().current().cloned();
        self.side_panel.set_info(self.now_playing.track.as_ref(), song.as_ref());
        self.arm_option();
    }

    /// The words under the artwork written again from the playing song's face (its resolution
    /// line learnt the stream, or the option it plays at moved): no dissolve.
    pub(crate) fn words_again(&mut self) {
        let Some(f) = self.now_playing.track.as_ref() else { return };
        let w = (f.title.clone(), f.artist.clone(), f.album.clone(), f.res.clone());
        self.set_track_text(&w.0, &w.1, &w.2, &w.3, false);
    }

    /// The playing song's plugin hears `event` at `at` (services that ask for plays; spec,
    /// plugins: `play_event`).
    fn tell_plugin(&self, event: &'static str, at: Duration) {
        if let Some(s) = self.playing_queue().current() {
            crate::plugins::client::play_event(&self.plugin.accounts, &s.path, event, at);
        }
    }

    /// Whether a song opened now plays: as the one before played, or was asked to (it may be
    /// still opening).
    fn keeps_playing(&self) -> bool {
        self.playback.snap.playing || self.playback.wants_play
    }

    /// The engine seeks: counted, so that a snapshot from before it does not move the bar back
    /// (`turn`).
    fn engine_seek(&mut self, to: Duration) {
        self.playback.engine.send(Cmd::Seek(to));
        self.playback.seeks_sent += 1;
    }

    /// The playing song's plugin, its URI, and the option it declares (its name: "音质").
    pub fn option_plugin(&self) -> Option<(String, String, String)> {
        let (plugin, uri) = self.playing_queue().current_plugin()?;
        let m = self.plugin.registry.manifest(&plugin)?;
        let name = m.option_name()?;
        Some((plugin, uri, name))
    }

    /// The line under the artwork: for a plugin's song, its `style.res` template filled with
    /// the stream's resolution and the option value's name, as two texts styled apart (`res`,
    /// `res-note` in ui/player.nui). It opens the option flyout when the plugin has an option.
    pub(crate) fn res_texts(&self, res: &str) -> (String, String) {
        let Some((plugin, _)) = self.playing_queue().current_plugin() else {
            return (res.to_owned(), String::new());
        };
        let tpl = self.plugin.registry.style_res(&plugin);
        let opt = self
            .playback
            .snap
            .option
            .as_ref()
            .filter(|_| self.playback.snap_is_open())
            .map(|(_, n)| n.as_str())
            .unwrap_or("");
        crate::plugins::client::res_line(tpl.as_deref(), res, opt)
    }

    /// The playing song resolved at another value of its plugin's option than the user chose
    /// (it does not come in it): said once, until the choice changes.
    pub(crate) fn tell_fallback(&mut self, ui: &mut Ui) {
        let version = self.playback.song_version;
        if self.playback.snap.error.is_some() || self.playback.fallback.seen(version) {
            return;
        }
        let Some((given, name)) = self.playback.snap.option.clone() else { return };
        let Some((plugin, _, what)) = self.option_plugin() else { return };
        let chosen = crate::plugins::client::option::get(&plugin);
        if self.playback.fallback.tell(version, &plugin, chosen.as_deref(), &given) {
            let name = if name.is_empty() { given } else { name };
            self.toast(ui, ui.t("toast.option-fallback", &[("what", &what), ("name", &name)]));
        }
    }

    pub(crate) fn arm_option(&mut self) {
        let on = self.option_plugin().is_some();
        self.now_playing.arm_option(on);
    }

    /// Asks the playing song's plugin which values of its option the song comes in.
    pub fn open_option(&mut self, ui: &mut Ui) {
        let Some((plugin, uri, _)) = self.option_plugin() else { return };
        let acc = self.plugin.accounts.clone();
        let task = ui.spawn_waiting(
            move |_| crate::plugins::client::options(&acc, &plugin, &uri),
            |list| Landed(Job::Options { list }),
        );
        self.playback.tasks.hold(Some(task));
    }

    /// A value picked: the plugin's songs resolve at it from now on, and the playing song opens
    /// again at it, where it was, playing or not as it was.
    pub fn set_option(&mut self, ui: &mut Ui, v: &str) {
        let Some((plugin, _, _)) = self.option_plugin() else { return };
        let same = self.playback.snap.option.as_ref().is_some_and(|(id, _)| id == v);
        crate::plugins::client::option::set(&plugin, v);
        self.keeping.changed(crate::app::kept::Kept::Options);
        self.playback.fallback.forget(&plugin);
        if same {
            return;
        }
        self.reopen(ui);
    }

    /// The playing song opens again where it is, playing as it was or was asked to (it may be
    /// still opening): an option changed, a plugin's command that changed its face.
    pub(crate) fn reopen(&mut self, ui: &mut Ui) {
        let at = self.position();
        self.open_current(ui, at, self.keeps_playing());
    }

    pub fn show_cover(&mut self, cover: Option<library::Cover>) {
        self.now_playing.set_cover(cover);
    }

    /// 4.11: change the cover of a local song without its own picture (kept for its folder).
    pub fn choose_cover(&mut self, ui: &mut Ui) {
        let Some(song) = self.playing_queue().current().cloned() else { return };
        let filters = vec![(ui.t("filter.images", &[]), crate::library::ext_filter(crate::library::covers::IMAGE_EXT))];
        ui.pick(novaui::FilePick::Open { multi: false, filters }, move |paths| crate::app::Answer::Cover {
            song: Box::new(song),
            image: paths.into_iter().next(),
        });
    }

    /// The picture the user chose for `song`'s folder (`choose_cover`), kept and shown.
    pub(crate) fn set_chosen_cover(&mut self, ui: &mut Ui, song: library::Song, img: std::path::PathBuf) {
        let version = self.playback.song_version;
        let task = ui.spawn_waiting(
            move |_| {
                let kept = crate::library::covers::chosen().choose(&song.path, &img).ok();
                crate::library::covers::folder_changed(&song.path);
                Job::Cover { song: version, cover: kept.as_deref().and_then(|b| library::decode_cover(b, 1040)) }
            },
            Landed,
        );
        // What is chosen is kept whatever plays by the time it is: the app's, not the song's.
        self.tasks.hold(Some(task));
    }

    pub fn restore_cover(&mut self, ui: &mut Ui) {
        let Some(song) = self.playing_queue().current().cloned() else { return };
        let version = self.playback.song_version;
        let task = ui.spawn_waiting(
            move |_| {
                let _ = crate::library::covers::chosen().remove(&song.path);
                crate::library::covers::folder_changed(&song.path);
                let cover = crate::library::covers::fallback(&song.path).and_then(|b| library::decode_cover(&b, 1040));
                Job::Cover { song: version, cover }
            },
            Landed,
        );
        self.tasks.hold(Some(task));
    }

    pub fn position(&self) -> Duration {
        self.playback.snap.position_now()
    }

    /// The info page describes the playing song again.
    pub fn fill_playing_info(&mut self) {
        self.side_panel.info_of = None;
        let song = self.playing_queue().current().cloned();
        self.side_panel.set_info(self.now_playing.track.as_ref(), song.as_ref());
    }

    /// 歌曲信息 (a queue row's menu): the row's song on the info page. The one that plays shows
    /// what playing it learnt (its stream, its option); another shows the list's words at once,
    /// and a local file's own once read off the UI thread.
    pub fn row_info(&mut self, ui: &mut Ui, pos: usize) {
        let now = ui.now();
        let Some(song) = self.lists.shown_list.song_at(pos).cloned() else { return };
        let playing = self.now_playing.track.is_some()
            && self.playing_queue().current().is_some_and(|c| c.is(&song.path, song.span));
        if playing {
            self.fill_playing_info();
        } else {
            self.side_panel.show_song_info(&song, None);
            let s = song.clone();
            let task = ui.spawn_waiting(
                move |_| {
                    let tags = s.path.is_file().then(|| crate::playback::engine::probe(&s.path)).flatten();
                    Job::SongInfo { song: s, tags: tags.map(Box::new) }
                },
                Landed,
            );
            self.tasks.hold(Some(task));
        }
        if !self.side_panel.shown.shows_info() {
            self.side_panel.toggle_info(now);
        }
    }

    pub fn play_pause(&mut self, ui: &mut Ui) {
        if self.playing_queue().is_empty() {
            return;
        }
        if !self.playback.opened {
            self.open_current(ui, Duration::ZERO, true);
            return;
        }
        if self.playback.snap.playing {
            self.playback.engine.send(Cmd::Pause);
            self.playback.rest();
            self.tell_plugin("pause", self.playback.snap.position);
        } else {
            if self.playback.snap.ended {
                // Another song follows the one that ended; the same one (repeat one) plays again.
                let queue = self.playing_queue();
                if let Some(p) = queue.follow_pos().filter(|&p| p != queue.pos) {
                    self.play_pos(ui, p, true);
                    return;
                }
            }
            self.playback.engine.send(Cmd::Play);
            self.playback.resume();
            self.tell_plugin("resume", self.playback.snap.position);
            self.playback.snap.at = Some(std::time::Instant::now());
        }
        self.schedule_save();
    }

    pub fn previous(&mut self, ui: &mut Ui) {
        let to = self.playing_queue().prev_pos();
        self.skip_to(ui, to);
    }

    pub fn next(&mut self, ui: &mut Ui) {
        let to = self.playing_queue().next_pos();
        self.skip_to(ui, to);
    }

    /// Previous and next: the song at `to` opens, playing as the one skipped did (none: the end of
    /// a list that does not wrap; the button is off).
    fn skip_to(&mut self, ui: &mut Ui, to: Option<usize>) {
        self.tell_plugin("skip", self.playback.snap.position);
        if let Some(p) = to {
            self.play_pos(ui, p, self.keeps_playing());
        }
    }

    pub fn seek(&mut self, to: Duration) {
        let to = self.playback.snap.duration.map_or(to, |d| to.min(d));
        self.engine_seek(to);
        self.playback.snap.position = to;
        self.playback.snap.at = Some(std::time::Instant::now());
        self.playback.snap.ended = false;
        self.side_panel.just_seeked = true;
        self.tell_plugin("seek", to);
        self.schedule_save();
    }

    /// The time the side panel last ticked (tests).
    #[cfg(all(feature = "test-support", windows))] // the screenshot suites (Windows only)
    pub fn side_panel_tick(&self) -> f64 {
        self.side_panel.last_tick()
    }

    pub fn set_volume(&mut self, v: f32) {
        self.playback.volume = v.clamp(0.0, 1.0);
        self.playback.engine.send(Cmd::Volume(self.playback.volume));
        self.playback.gain_touched = true;
    }

    /// 6.5: press mutes; again goes back to the last audible level (kept across runs).
    pub fn mute(&mut self) {
        if self.playback.volume > 0.0 {
            self.set_volume(0.0);
        } else {
            let v = self.playback.kept_gain.gain.unmuted;
            self.set_volume(if v > 0.0 { v } else { 1.0 });
        }
    }

    pub fn stop(&mut self, ui: &mut Ui) {
        self.playback.engine.send(Cmd::Stop);
        self.playback.close();
        self.playing_queue_mut().list.resume = false;
        self.playing_queue_mut().list.position = Duration::ZERO;
        // A list that played from outside the page is put back to rest, as it is.
        if let Some(p) = self.lists.playing_list.take() {
            if let Some(id) = p.id {
                self.lists.store.save(id, p.to_list());
            }
        }
        self.playback.snap = Snapshot::default();
        self.playback.state.notify();
        if self.now_playing.track.is_some() {
            *self.now_playing.track.edit() = None;
        }
        self.show_idle_words(ui, true);
        self.now_playing.clear_cover();
        self.side_panel.set_lyrics(None);
        self.side_panel.set_info(None, None);
        ui.window.set_title("NovaPlayer");
        self.schedule_save();
    }

    /// End of a song that nothing followed: what follows it (`Queue::follow_pos`; spec,
    /// "Transport under the art"). The same song again plays from the start where it is open.
    pub(crate) fn on_ended(&mut self, ui: &mut Ui) {
        self.tell_plugin("end", self.playback.snap.position);
        let queue = self.playing_queue();
        match queue.follow_pos() {
            Some(p) if p == queue.pos => {
                self.engine_seek(Duration::ZERO);
                self.playback.engine.send(Cmd::Play);
            }
            Some(p) => self.play_pos(ui, p, true),
            None => {
                // The list stops: rest at the end of the last song.
                self.playback.rest();
            }
        }
    }
}
