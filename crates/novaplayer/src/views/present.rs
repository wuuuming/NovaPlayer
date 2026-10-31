//! Frame presentation. Jobs and session operations leave state / typed requests; this module
//! applies them before the layout, transport and time-driven views read the resulting tree.
use crate::app::*;
use novaui::*;

impl Player {
    pub(crate) fn init_view(&mut self, ui: &mut Ui) {
        ui.tree.set_clear(argb(0xFF202020));
        let root = ui.tree.root();
        self.doc.build(ui, root).unwrap_or_else(|e| panic!("player.nui: {e}"));
        // Debug builds: edits to the sheet apply while the player runs (F12 inspects).
        #[cfg(debug_assertions)]
        self.doc.watch(ui, crate::app::ui_file());
        self.show_idle_words(ui, false);
        self.doc.set(ui, "elapsed", "");
        self.doc.set(ui, "remaining", "");
        self.doc.set(ui, "toast", "");
        self.side_panel.init(ui, &mut self.doc);
        self.settings_page.init(ui, &mut self.doc);
        self.sync_plugin_controls(ui);
        self.show_route(ui);
        // The settings page's choices (the font ahead of every text) from the first frame.
        for s in crate::views::settings_page::SETTINGS {
            let value = self.settings.get(s.key).map(str::to_owned);
            self.views(|v, _| crate::views::menus::dropdown::apply(ui, v, s.key, value.as_deref()));
        }
    }

    /// Apply event-driven work before consumers of the words / lyric groups / image slots.
    /// Also used on initial restore before the first forced queue sync.
    pub(crate) fn present_pending(&mut self, ui: &mut Ui) {
        if self.strings_seen != ui.strings().version() {
            self.strings_seen = ui.strings().version();
            self.relabel(ui);
        }
        let (v, m) = self.split();
        v.now_playing.present(ui, v.doc);
        v.side_panel.present(ui, v.doc);
        let mut v = v;
        crate::views::menus::close_stale(&mut v, ui, &m);
        for ask in std::mem::take(&mut self.view_asks) {
            match ask {
                ViewAsk::RefreshPlugins => {
                    // After `plugins_changed` the controls mount again from the registry.
                    self.sync_plugin_controls(ui);
                    if self.views(|v, _| crate::views::menus::is_open(v, |k| *k == crate::views::menus::Kind::App)) {
                        self.views(|v, m| crate::views::menus::app_menu::open_app_menu(v, ui, m));
                    }
                    self.views(|v, md| crate::views::settings_page::refresh(v, ui, md));
                }
                // Recheck at presentation too: EOF may have advanced the song after drain.
                ViewAsk::OptionMenu { song, values } if song == self.playback.song_version => {
                    self.views(|v, m| crate::views::menus::option::open_option_menu(v, ui, m, values));
                }
                ViewAsk::ImportDialog { account } if account == self.plugin.account_version => {
                    self.views(|v, m| crate::views::import_dialog::open_import_dialog(v, ui, m));
                }
                _ => {}
            }
        }
    }

    /// The language changed (`设置 › 外观 › 语言`): every text a view wrote speaks it again. The
    /// sheet's `@key` texts already do (`Doc::update`); a toast up keeps its words, a flyout
    /// closes (its rows were written in the old language).
    fn relabel(&mut self, ui: &mut Ui) {
        if self.now_playing.track.is_none() && !self.now_playing.words().title.is_empty() {
            self.show_idle_words(ui, false);
        }
        if self.now_playing.track.is_some() {
            let song = self.playing_queue().current().cloned();
            self.side_panel.set_info(self.now_playing.track.as_ref(), song.as_ref());
        }
        self.settings_page.init(ui, &mut self.doc);
        self.views(|v, m| {
            crate::views::menus::close_menus(v, ui);
            for s in crate::views::settings_page::SETTINGS {
                crate::views::menus::dropdown::left_box(v, ui, m, s.key);
            }
            crate::views::settings_page::refresh(v, ui, m);
        });
    }

    /// One frame's presentation, after jobs, engine snapshot and session decisions. Static
    /// views observe their changed inputs; motion and deadlines still receive clock ticks. Every
    /// view gets its own state and the tree to write and the models to read (`Player::split`).
    pub(crate) fn present(&mut self, ui: &mut Ui) {
        self.present_pending(ui);
        {
            let (mut v, m) = self.split();
            present_search(&mut v, ui);
            crate::views::menus::dropdown::present_boxes(&mut v, ui);
            if v.settings_page.open {
                crate::views::menus::output::present_drops(&mut v, ui, &m);
            }
            crate::views::import_dialog::present_import(&mut v, ui);
            crate::views::layout::layout_state(&mut v, ui);
            crate::views::transport::update_transport(&mut v, ui, &m);
        }
        // The field's shade seats the two-column layout's right column: none in one column, and
        // it fades out under the settings page (one column over the whole field).
        self.settings_page.tick(ui, &self.doc);
        let seat = if self.doc.flag("two") { 1.0 - self.settings_page.amount } else { 0.0 };
        let (mut v, m) = self.split();
        let f = crate::views::now_playing::FrameIn {
            playing: m.playback.snap.playing,
            opened: m.playback.opened,
            wants_play: m.playback.wants_play,
            shade_from: v.side_panel.shade_from(ui, v.doc),
            panel_open: seat * v.side_panel.open_amount,
            sound_fade: Some(&m.playback.snap).filter(|_| m.playback.snap_is_open()).and_then(|s| s.fade_progress()),
        };
        v.now_playing.frame(ui, v.doc, f);
        crate::views::toast::present_toast(&mut v, ui);
        v.side_panel.tick(ui, v.doc);
        crate::views::transport::present_vol_tip(&mut v, ui);
        crate::views::desktop_lyrics::present(&mut v, ui, &m);
        crate::views::menus::drag::drag_tick(self, ui);
    }

    /// The post-layout phase (`App::after_layout`): everything placed against another element's
    /// laid-out box, read once the host has laid out this turn's writes.
    /// The queue's rows follow the shown list (the side panel learns whether it is the one
    /// playing); `force`: listed again whatever moved.
    pub(crate) fn sync_queue(&mut self, ui: &mut Ui, force: bool) {
        self.side_panel.shows_playing = self.lists.playing_list.is_none();
        self.side_panel.sync(
            ui,
            &mut self.doc,
            &self.lists.shown_list,
            self.lists.list_version,
            &self.lists.store,
            &self.inbox,
            &self.assets,
            force,
        );
    }

    pub(crate) fn after_layout_phase(&mut self, ui: &mut Ui) {
        // The queue's rows are placed against the list's laid-out height (which rows are in
        // view, how far it can scroll): this turn's height, not last frame's.
        self.sync_queue(ui, false);
        let pos = self.playback.shown_position();
        self.side_panel.after_layout(ui, &mut self.doc, pos, self.playback.snap.playing);
        self.views(|v, _| crate::views::transport::place_vol_tip(v, ui));
        self.views(|v, _| crate::views::menus::after_layout(v, ui));
        if self.settings_page.open {
            self.settings_page.place(ui, &mut self.doc);
        }
        // Bindings written after the layout's `doc.update` (the queue's caption and rows, the
        // lyric keys) reach their expressions in this turn's frame, not on whatever wake comes
        // next. A no-op when nothing was written.
        crate::views::update_doc(&mut self.doc, &self.doc_clock, ui);
    }
}

/// Search is product styling / page state, not a generic editor feature.
fn present_search(v: &mut Views, ui: &mut Ui) {
    let search = v.id("search");
    // Any route that dropped the narrowing leaves no words in the box.
    if v.side_panel.narrow.as_ref().is_none_or(|n| n.text.is_empty()) && !ui.edit_text(search).is_empty() {
        ui.set_edit_text(search, "");
        if ui.focus() == Some(search) {
            ui.set_focus(None);
        }
    }
    let focused = ui.focus() == Some(search);
    v.doc.set(ui, "search-focused", focused);
    v.doc.set(ui, "search-clearable", focused && !ui.edit_text(search).is_empty());
}
