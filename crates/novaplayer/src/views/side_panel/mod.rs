//! The side panel: queue (virtualized rows), lyrics, song info; its pages and motion (spec, "Now-playing face",
//! "Lyrics", "Queue page" and "Song info page").

mod info;
mod lyrics;
pub use lyrics::parse_style;
mod queue;

use crate::app::{Job, NowPlayingTrack};
use crate::library::queue::Queue;
use crate::library::store::Store;
use crate::library::{self, Lyrics, Song};
use novaui::dsl::{Doc, Val};
use novaui::*;
use std::collections::HashMap;
use std::time::Duration;

pub const ROW_H: f32 = 64.0;
const LYRIC_FOCUS: f32 = 0.35;
const LYRIC_MOTION: f32 = 0.30;
const LYRIC_EDGE: f32 = 96.0;
const BROWSE_SECS: f64 = 3.0;
const PANEL_MOTION: f32 = 0.30;
/// Thumbnails kept for rows (the least recently shown goes past this).
const THUMB_SLOTS: usize = 96;
/// A page of the side panel giving way to another (b7b33f5 lane_page.rs): 0.3 s on the smoothstep,
/// each page travelling 48 dips the way the turn goes.
const TURN_SECS: f32 = 0.30;
const TURN_DIST: f32 = 48.0;
/// A new search's or narrowing's rows rise this far into place over the turn.
const RISE_DIST: f32 = 24.0;
/// Its curve: WinUI's fast-out.
const SHIFT_EASE: Ease = Ease::Bezier(0.1, 0.9, 0.2, 1.0);

/// A page turning in, a level deeper or back out (`deeper`): from 48 dips (`chrome_scale`: the chrome's
/// factor) the way the turn goes, on the smoothstep. The queue's narrowings and the info page
/// turn this way, and so do the settings page's levels.
pub(crate) fn turn_in(ui: &mut Ui, id: Id, deeper: bool, c: f32, now: f64) {
    let sign = if deeper { c } else { -c };
    ui.tree.enter(id, Pose::clear_x(-sign * TURN_DIST), TURN_SECS, Ease::Smooth, now);
}

/// What the side panel shows: open or folded, the page (kept while folded), and the info page
/// over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Shown {
    pub open: bool,
    pub page: Page,
    pub info_up: bool,
}

impl Shown {
    /// The panel is open with the song's info over its page.
    pub fn shows_info(&self) -> bool {
        self.open && self.info_up
    }

    /// The panel is open on `page`, nothing over it.
    pub fn shows(&self, page: Page) -> bool {
        self.open && !self.info_up && self.page == page
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Queue,
    Lyrics,
}

/// What the queue's rows are of: the list's version, its order's version and the narrowing.
pub type Listed = (u64, u64, Option<Narrow>);

/// A search's result, landed from its worker (`SidePanel::found`).
pub struct Found {
    pub of: Listed,
    pub positions: Vec<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Narrow {
    pub text: String,
    pub artist: Option<String>,
    pub album: Option<String>,
}

impl Narrow {
    /// It is a search: words only, no artist or album.
    pub fn is_search(&self) -> bool {
        self.artist.is_none() && self.album.is_none()
    }

    /// Something is typed: the rows are ranked by how well it is found (`Queue::filter`), not in
    /// play order, so they cannot be dragged into another place.
    pub fn ranked(&self) -> bool {
        !self.text.trim().is_empty()
    }
}

struct RowEl {
    root: Id,
    name: Id,
    artist: Id,
    sep: Id,
    album: Id,
    len: Id,
    remove: Id,
    grip: Id,
    thumb: Id,
    rule: Id,
    idx: usize,
}

struct GroupEl {
    root: Id,
    lines: Vec<Id>,
    /// The doc names of its lyric style contract values (`lyrics::STYLE_KEYS` order).
    keys: [String; 8],
    /// Its first line has word timing (it sweeps while sung).
    words: bool,
    /// Its time window (`lyrics::TIME_BEFORE`, `TIME_AFTER`), seconds.
    window: (f32, f32),
    /// Its layer lines (a romanisation, a translation).
    layers: Vec<LayerEl>,
}

/// A layer line under a group, and where its fold stands: `x` 0 folded away under the group's
/// line, 1 unfolded, moved by a spring (`v` its speed) once `wait` (its place in the ripple) is
/// over.
struct LayerEl {
    /// Index in `library::LAYER_ORDER`.
    kind: usize,
    /// The clipping box; the line in it.
    wrap: Id,
    line: Id,
    x: f32,
    v: f32,
    wait: f32,
}

/// A middle-button autoscroll in progress.
#[derive(Clone, Copy, Debug)]
pub struct Autoscroll {
    pub anchor: i32,
    pub pointer: i32,
    /// Moved further than the dead zone while the button was held: releasing ends it.
    pub dragged: bool,
    /// Released without moving: the scroll runs on until the next press.
    pub latched: bool,
    pub last: f64,
}

pub struct SidePanel {
    /// Its frames and deadlines (`novaui::Clock`).
    pub clock: novaui::Clock,
    /// The wake for the sung lyric's next line: one deadline, replaced as the song plays, seeks
    /// or pauses (`Clock::cancel`), so a stale one never fires.
    pub line_clock: novaui::Clock,
    /// What the panel shows: its one owner. Changing it notifies whoever follows it (the plugin
    /// controls' context, `Msg::Controls`).
    pub shown: novaui::Model<Shown>,
    /// The song the info page describes when it is not the one that plays (a queue row's
    /// 歌曲信息); `None`: the playing one, followed as it changes.
    pub info_of: Option<Song>,
    pub narrow: Option<Narrow>,
    /// The page last shown, as (pane, depth, narrowing): a change is a turn.
    turn_key: Option<(&'static str, u8, Option<Narrow>)>,
    pub open_amount: f32,
    /// The open amount's motion (`open_amount` is its value at the last present).
    open: novaui::anim::Tween,
    /// The panel was opened (true) or closed since the last present: its slide and fade start.
    panel_turn: Option<bool>,
    pub relayout: bool,
    pub follow_current: bool,
    /// The locate button: ease to the playing song.
    pub locate: bool,
    /// The shown list is the one the loaded song plays from (its row is marked as playing).
    pub shows_playing: bool,
    pub just_seeked: bool,
    // geometry (dips)
    rect: Option<(f32, f32, f32, f32)>,
    one_column: bool,
    /// The layout's text factor.
    pub scale: crate::views::layout::Scale,
    // queue
    rows: HashMap<Id, RowEl>,
    /// The queue's rows: which places are live, the carried one, those giving way.
    list: Option<novaui::widgets::VirtualList>,
    version: u64,
    /// Shared so a present holds it without copying 20 000 entries.
    positions: std::rc::Rc<Vec<usize>>,
    /// What `positions` are of: the list's version and order version, and the narrowing.
    positions_key: Listed,
    /// A search running on a worker for the narrowing typed (`queue::filter`), and what it is
    /// for; dropped (and so called off) when another is typed. The rows shown stay until it
    /// lands (`found`).
    searching: Option<(Listed, novaui::Task)>,
    pub scroll: f32,
    target: f32,
    /// The row pitch and list height the scroll was last laid out at: a resize keeps the same
    /// row at the top (the scroll is rescaled with the pitch, not left in dips).
    last_rh: f32,
    last_lh: f32,
    /// The list is easing toward a follow target (not a resize's clamp).
    easing: bool,
    /// The scroll's motion toward the list's goal, and the wheel's speed (`Glide`).
    glide: novaui::anim::Glide,
    /// Middle-button autoscroll (browsers'): the anchor's y (px), the pointer's, and whether the
    /// button was let go without moving (then it stays on until the next press).
    auto: Option<Autoscroll>,
    listed_version: u64,
    row_gen: u64,
    pub drag: Option<(usize, f32, f32)>,
    /// The narrowing the rows were last listed for: a new one snaps the scroll and the rows rise
    /// into place instead of the list sliding across to them.
    listed_narrow: Option<Option<Narrow>>,
    thumbs: HashMap<usize, novaui::ImageHandle>,
    thumb_lru: Vec<usize>,
    requested: std::collections::HashSet<usize>,
    row_version: u64,
    wanted: std::sync::Arc<std::sync::Mutex<(u64, u64, std::collections::HashSet<usize>)>>,
    /// Songs whose file has no picture to make a row cover of (asked once).
    no_thumb: std::collections::HashSet<usize>,
    /// Rows whose online song's face could not be had (a timeout, a dropped connection): asked
    /// again at the time, a few times (idx → (when, tries)).
    thumb_retry: std::collections::HashMap<usize, (f64, u8)>,
    last_pos: usize,
    queue_rail: novaui::widgets::scroll::Scrollbar,
    lyrics_rail: novaui::widgets::scroll::Scrollbar,
    // lyrics
    lyrics: Option<Lyrics>,
    groups: Vec<GroupEl>,
    /// The lyric style the lines are drawn with, as adopted: `None` until the lines are next
    /// built (`present_lyrics`).
    lyric_style: Option<novaui::dsl::Adopted>,
    /// The chosen style's `.nui` (the setting `lyric-style`); `None`: the built-in one.
    lyric_style_src: Option<String>,
    /// The style changed: the adopted one goes once the lines drawn with it are gone.
    restyle: bool,
    /// The lyric style reads `time` (contract v1): visible groups in their window want frames.
    lyric_timed: bool,
    heights: Vec<f32>,
    anchors: Vec<f32>,
    lyric_key: (i32, i32),
    sung: Option<usize>,
    leaving: Option<usize>,
    lyric_scroll: f32,
    lyric_scroll_from: f32,
    lyric_motion: novaui::anim::Tween,
    /// How far the reader has scrolled the lyrics off the sung line (dips), as shown, and its
    /// glide; it glides back once `browse_until` passes.
    browse: f32,
    browse_glide: novaui::anim::Glide,
    browse_until: f64,
    lyric_started: bool,
    lyric_active: bool,
    /// The lyrics page began to show since the last post-layout phase (its scroll jumps).
    lyric_entering: bool,
    last_tick: f64,
    // info
    info_rows: Vec<Id>,
    /// How far the info page is scrolled (dips from its top) when it does not fit.
    info_scroll: novaui::anim::Glide,
    /// The info page's rows (group → label, value), made into elements at present.
    info_groups: Vec<Vec<(&'static str, String)>>,
    info_stale: bool,
    /// The lyrics changed since their elements were made.
    lyrics_stale: bool,
    /// Each layer of `library::LAYER_ORDER` is on (its lines unfold; off, they fold away): the
    /// player's choice (`now_playing.layers_off`), as this panel shows it.
    layer_on: [bool; library::LAYER_ORDER.len()],
    /// Switched since the last tick: its lines get their places in the ripple.
    layer_kick: [bool; library::LAYER_ORDER.len()],
    /// The layer lines are at rest where `layer_on` puts them (nothing to write).
    layers_placed: bool,
    /// When the layer springs last stepped (while they move).
    layer_clock: f64,
}

impl SidePanel {
    /// Its clocks deliver `Msg::SidePanel`; `off`: the lyric layers switched off (kept).
    pub fn new(ui: &mut novaui::Ui, off: std::collections::BTreeSet<String>) -> Self {
        SidePanel {
            clock: ui.clock(crate::app::Msg::SidePanel),
            line_clock: ui.clock(crate::app::Msg::SidePanel),
            shown: Default::default(),
            info_of: Default::default(),
            narrow: Default::default(),
            turn_key: Default::default(),
            open_amount: Default::default(),
            open: Default::default(),
            panel_turn: Default::default(),
            relayout: Default::default(),
            follow_current: Default::default(),
            locate: Default::default(),
            shows_playing: Default::default(),
            just_seeked: Default::default(),
            rect: Default::default(),
            one_column: Default::default(),
            scale: Default::default(),
            rows: Default::default(),
            list: Default::default(),
            version: Default::default(),
            positions: Default::default(),
            positions_key: Default::default(),
            searching: None,
            scroll: Default::default(),
            target: Default::default(),
            last_rh: Default::default(),
            last_lh: Default::default(),
            easing: Default::default(),
            glide: Default::default(),
            auto: Default::default(),
            listed_version: Default::default(),
            row_gen: Default::default(),
            drag: Default::default(),
            listed_narrow: Default::default(),
            thumbs: Default::default(),
            thumb_lru: Default::default(),
            requested: Default::default(),
            row_version: Default::default(),
            wanted: Default::default(),
            no_thumb: Default::default(),
            thumb_retry: Default::default(),
            last_pos: Default::default(),
            queue_rail: Default::default(),
            lyrics_rail: Default::default(),
            lyrics: Default::default(),
            groups: Default::default(),
            lyric_style: Default::default(),
            lyric_style_src: Default::default(),
            restyle: Default::default(),
            lyric_timed: Default::default(),
            heights: Default::default(),
            anchors: Default::default(),
            lyric_key: Default::default(),
            sung: Default::default(),
            leaving: Default::default(),
            lyric_scroll: Default::default(),
            lyric_scroll_from: Default::default(),
            lyric_motion: novaui::anim::Tween::rest(1.0),
            browse: Default::default(),
            browse_glide: Default::default(),
            browse_until: Default::default(),
            lyric_started: Default::default(),
            lyric_active: Default::default(),
            lyric_entering: Default::default(),
            last_tick: Default::default(),
            info_rows: Default::default(),
            info_scroll: Default::default(),
            info_groups: Default::default(),
            info_stale: Default::default(),
            lyrics_stale: Default::default(),
            layer_on: library::LAYER_ORDER.map(|id| !off.contains(id)),
            layer_kick: Default::default(),
            layers_placed: Default::default(),
            layer_clock: Default::default(),
        }
    }
}

impl SidePanel {
    /// Writes what the panel's state changed since the last present: the lyric groups, the info
    /// rows. Run once a frame before `sync` / `tick`.
    pub fn present(&mut self, ui: &mut Ui, doc: &mut Doc) {
        self.present_lyrics(ui, doc);
        self.present_layer_buttons(ui, doc);
        self.present_info(ui, doc);
    }
}

fn opt(s: &Option<String>) -> String {
    s.clone().unwrap_or_else(|| library::MISSING.to_string())
}

impl SidePanel {
    /// The queue's narrowing changed (a search, an artist or album, none): the queue follows
    /// the current song again.
    pub fn set_narrow(&mut self, narrow: Option<Narrow>) {
        self.narrow = narrow;
        self.follow_current = true;
    }

    pub fn init(&mut self, ui: &mut Ui, doc: &mut Doc) {
        for p in ["queue-pane", "lyrics-pane", "info-pane", "queue-rail", "lyrics-rail"] {
            let id = doc.id(p);
            ui.tree.set_visible(id, false);
        }
        doc.set(ui, "list-name", ui.t("list.default", &[]));
        doc.set(ui, "queue-caption", "");
    }

    pub fn open_queue_if_empty(&mut self, q: &Queue) {
        if q.is_empty() {
            self.shown.edit().open = true;
            self.shown.edit().page = Page::Queue;
            self.open_amount = 1.0;
            self.open = novaui::anim::Tween::rest(1.0);
        }
    }

    pub fn toggle(&mut self, now: f64, page: Page) {
        if self.shown.shows(page) {
            self.set_open(now, false);
        } else {
            self.shown.edit().page = page;
            self.shown.edit().info_up = false;
            self.narrow = None;
            self.set_open(now, true);
        }
        self.relayout = true;
    }

    pub fn toggle_info(&mut self, now: f64) {
        if self.shown.shows_info() {
            self.shown.edit().info_up = false;
        } else {
            self.shown.edit().info_up = true;
            self.set_open(now, true);
        }
        self.relayout = true;
    }

    pub fn set_open(&mut self, now: f64, on: bool) {
        if self.shown.open != on {
            self.shown.edit().open = on;
            let to = if on { 1.0 } else { 0.0 };
            self.open = novaui::anim::Tween::new(self.open_amount, to, now, PANEL_MOTION.into(), Ease::OutCubic);
            self.panel_turn = Some(on);
            self.relayout = true;
            if on {
                // A folded side panel reveals its final destination directly, whether remembered or
                // explicitly chosen. Never turn through the page that was hidden before it.
                self.turn_key = None;
                if self.shown.page == Page::Queue {
                    self.follow_current = true;
                }
            }
        }
    }

    /// Back from a second-level page (info, narrowed queue).
    pub fn back(&mut self) -> bool {
        if self.shown.info_up {
            self.shown.edit().info_up = false;
        } else if self.narrow.is_some() {
            self.set_narrow(None);
        } else {
            return false;
        }
        self.relayout = true;
        true
    }

    pub fn second_level(&self) -> bool {
        self.shown.open && (self.shown.info_up || (self.shown.page == Page::Queue && self.narrow.is_some()))
    }

    pub fn list_changed(&mut self) {
        self.listed_version = u64::MAX;
        self.follow_current = true;
    }

    pub fn row_changed(&mut self, idx: usize) {
        if let Some(r) = self.rows.values_mut().find(|r| r.idx == idx) {
            r.idx = usize::MAX; // re-fill on next sync
        }
    }

    /// Timed lyrics for the current song.
    pub fn has_lyrics(&self) -> bool {
        self.lyrics.as_ref().is_some_and(|l| !l.groups.is_empty())
    }

    /// A queue row's pitch in dips: 64 at the layout's text factor.
    pub fn row_h(&self) -> f32 {
        ROW_H * self.scale.text
    }

    /// Reads where the layout put the side panel (the layout is ui/player.nui's).
    pub fn place(&mut self, _ui: &mut Ui, doc: &Doc) {
        let r = (doc.num("panel_x"), doc.num("safe_y"), doc.num("panel_w"), doc.num("panel_h"));
        if Some(r) != self.rect {
            self.rect = Some(r);
            self.lyric_key = (0, 0);
        }
        self.one_column = !doc.flag("two");
        self.scale = crate::views::layout::Scale::of(doc);
    }

    /// Window x (dips) where the field darkens: the side panel's left edge.
    pub fn shade_from(&self, _ui: &Ui, _doc: &Doc) -> f32 {
        self.rect.map_or(0.0, |r| r.0)
    }

    /// The panel's present (every player turn): motion, page turns, typography. What reads
    /// laid-out boxes waits for `after_layout`.
    pub fn tick(&mut self, ui: &mut Ui, doc: &mut Doc) {
        let now = ui.now();
        let lyric_active = self.shown.shows(Page::Lyrics);
        let lyric_pane = doc.id("lyrics-pane");
        let lyric_entering = lyric_active
            && !self.lyric_active
            && (!ui.tree.el(lyric_pane).visual().visible || ui.tree.prop(lyric_pane, Prop::Opacity) <= 0.0);
        self.lyric_active = lyric_active;
        self.lyric_entering |= lyric_entering;
        // Prepare typography even while this page is hidden. Revealing a page must not first
        // lay out default-sized text and then resize it during the dissolve.
        if !self.groups.is_empty() && doc.num("panel_w") > 0.0 {
            self.style_lyrics(ui, doc, !doc.flag("two"));
        }
        self.fold_layers(ui);
        self.last_tick = now;
        // SidePanel open/close.
        // 7.3: the side panel opens over 0.30 s eased out (cubic); the artwork's column gives it room by
        // the same amount (the layout reads `panel-open`). It slides in 48 dips from the right.
        self.open_amount = self.open.at(now);
        if !self.open.done(now) {
            self.clock.frame();
        }
        let panel = doc.id("side-panel");
        let c = crate::views::layout::Scale::of(doc).chrome;
        // Coming in (7.3, transition lane_in): 48 dips to 0 on the control knob curve over
        // 333 ms, the opacity on the standard curve over 250 ms. Going out follows the column.
        // The tree runs both motions; at rest the panel stands in or is hidden.
        match self.panel_turn.take() {
            Some(true) => {
                if !ui.tree.el(panel).visual().visible {
                    ui.tree.set_prop(panel, Prop::X, 48.0 * c);
                    ui.tree.set_opacity(panel, 0.0);
                    ui.tree.set_visible(panel, true);
                }
                ui.tree.animate(panel, Prop::X, 0.0, 0.333, Ease::Bezier(0.0, 0.0, 0.0, 1.0), now);
                ui.tree.animate(panel, Prop::Opacity, 1.0, 0.25, Ease::Bezier(0.1, 0.9, 0.2, 1.0), now);
            }
            Some(false) => {
                ui.tree.animate(panel, Prop::X, 48.0 * c, PANEL_MOTION, Ease::OutCubic, now);
                ui.tree.animate(panel, Prop::Opacity, 0.0, PANEL_MOTION, Ease::OutCubic, now);
            }
            // Closed once the column has given its room back (the same motion's end).
            None if !self.shown.open => {
                if self.open.done(now) {
                    ui.tree.stop(panel, Prop::X);
                    ui.tree.stop(panel, Prop::Opacity);
                    ui.tree.set_visible(panel, false);
                }
            }
            None if ui.tree.is_animating(panel, Prop::X) || ui.tree.is_animating(panel, Prop::Opacity) => {}
            // Open without a motion (`open_queue_if_empty` at the start).
            None => {
                ui.tree.set_visible(panel, true);
                ui.tree.set_opacity(panel, 1.0);
                ui.tree.set_prop(panel, Prop::X, 0.0);
            }
        }
        // Page turns (b7b33f5 lane_page.rs; layers: composite only). Going in (a second-level
        // page: the info page, the queue narrowed to an artist or album, a search), the page
        // that was there dissolves toward the right and the new one comes in behind it from
        // the left, both moving right; going back is the same toward the left; between the two
        // first-level pages, a dissolve in place.
        let pane: &'static str = if self.shown.info_up {
            "info-pane"
        } else if self.shown.page == Page::Queue {
            "queue-pane"
        } else {
            "lyrics-pane"
        };
        let narrowing = if pane == "queue-pane" { self.narrow.clone() } else { None };
        let depth = u8::from(self.shown.info_up || narrowing.is_some());
        let key = (pane, depth, narrowing);
        if self.turn_key.as_ref() != Some(&key) {
            let first = self.turn_key.is_none();
            let (old_pane, old_depth) = self.turn_key.as_ref().map(|k| (k.0, k.1)).unwrap_or((pane, depth));
            // A search typed into, or narrowed further, is the same page: no turn.
            let same_search = old_pane == pane
                && old_depth == depth
                && pane == "queue-pane"
                && self.turn_key.as_ref().and_then(|k| k.2.as_ref()).is_some_and(|n| n.is_search())
                && key.2.as_ref().is_some_and(|n| n.is_search());
            self.turn_key = Some(key);
            // One rule for every way back to the whole queue (back button or key, the other
            // first-level page, the info page, a cleared search, the side panel opening): the list
            // comes back at the playing song.
            if pane == "queue-pane" && depth == 0 {
                self.follow_current = true;
                self.clock.frame();
            }
            if first {
                // Initial display or an explicit destination chosen while folded: cancel any
                // interrupted page turn before revealing the target. The side panel's own opening
                // motion and the pages' vertical scroll positions remain untouched.
                for name in ["queue-pane", "lyrics-pane", "info-pane"] {
                    let id = doc.id(name);
                    ui.tree.stop(id, Prop::Opacity);
                    ui.tree.stop(id, Prop::X);
                    let active = name == pane;
                    ui.tree.set_visible(id, active);
                    ui.tree.set_opacity(id, if active { 1.0 } else { 0.0 });
                    ui.tree.set_prop(id, Prop::X, 0.0);
                }
                // Queue narrowing/back transitions animate the list rather than its pane.
                let list = doc.id("q-list");
                ui.tree.stop(list, Prop::Opacity);
                ui.tree.stop(list, Prop::X);
                ui.tree.set_opacity(list, 1.0);
                ui.tree.set_prop(list, Prop::X, 0.0);
            } else if !same_search {
                let sign = (depth as f32 - old_depth as f32).signum() * c;
                let ease = Ease::Smooth;
                if old_pane != pane {
                    let old = doc.id(old_pane);
                    ui.tree.exit(old, Pose::clear_x(sign * TURN_DIST), TURN_SECS, ease, now, Then::Stay);
                }
                // Within the queue (a narrowing, a search, back out): the heading stands and the
                // rows turn (8.7: `rekey`, the rows coming in behind).
                let id = if old_pane == pane { doc.id("q-list") } else { doc.id(pane) };
                let reversing =
                    old_pane != pane && ui.tree.el(id).visual().visible && ui.tree.prop(id, Prop::Opacity) > 0.0;
                ui.tree.set_visible(id, true);
                if reversing {
                    ui.tree.reenter(id, Pose::clear_x(0.0), TURN_SECS, ease, now);
                } else {
                    turn_in(ui, id, depth > old_depth, c, now);
                }
                self.clock.frame();
            }
        }
        for p in ["queue-pane", "lyrics-pane", "info-pane"] {
            let id = doc.id(p);
            let up = p == pane;
            if ui.tree.is_animating(id, Prop::Opacity) {
                continue;
            }
            if up {
                if !ui.tree.el(id).visual().visible || ui.tree.prop(id, Prop::Opacity) < 1.0 {
                    ui.tree.set_visible(id, true);
                    ui.tree.set_opacity(id, 1.0);
                    ui.tree.set_translate(id, 0.0, 0.0);
                }
            } else if ui.tree.el(id).visual().visible {
                ui.tree.set_visible(id, false);
                ui.tree.set_opacity(id, 0.0);
                ui.tree.set_translate(id, 0.0, 0.0);
            }
        }
        let back = doc.id("panel-back");
        let sl = self.second_level();
        ui.tree.set_layout(back, |l| l.display = if sl { Display::Flex } else { Display::None });
        ui.tree.set_opacity(back, if sl { self.open_amount } else { 0.0 });
    }

    /// The panel's post-layout phase (`App::after_layout`): what reads this turn's laid-out
    /// boxes. The lyric groups' heights and the side panel's height place the sung line; the info
    /// rows centre in their pane. No layout barrier of its own: the host laid out first.
    pub fn after_layout(&mut self, ui: &mut Ui, doc: &mut Doc, pos: Duration, playing: bool) {
        let now = ui.now();
        let lyric_entering = std::mem::take(&mut self.lyric_entering);
        self.place_info(ui, doc);
        if self.lyric_active && !self.groups.is_empty() {
            let pane = doc.id("lyrics-pane");
            let (w, h) = ui.tree.size(pane);
            if w > 0.0 {
                self.lyric_layout(ui);
            }
            if let Some(ly) = self.lyrics.as_ref().filter(|_| w > 0.0) {
                let current = ly.current(pos);
                // b7b33f5's shipped intro (Intro::Clamp): the first group high in the side panel, just
                // under the band the lines fade in (96 dips times the text's factor), and the sung
                // group down to the focus as the song goes on: the side panel is never half empty over
                // the first words.
                let fade = LYRIC_EDGE * self.scale.text;
                let anchor = self.anchors.get(current.unwrap_or(0)).copied().unwrap_or(0.0);
                let target = anchor.max(h * LYRIC_FOCUS - fade);
                if !self.lyric_started || lyric_entering {
                    self.lyric_started = true;
                    self.lyric_scroll = target;
                    self.lyric_scroll_from = target;
                    self.sung = current;
                    self.leaving = None;
                    self.lyric_motion = novaui::anim::Tween::rest(1.0);
                    self.browse = 0.0;
                    self.browse_glide.jump(0.0);
                    self.browse_until = 0.0;
                } else if current != self.sung {
                    self.leaving = self.sung;
                    self.sung = current;
                    self.lyric_scroll_from = self.lyric_scroll;
                    self.lyric_motion = if self.just_seeked {
                        novaui::anim::Tween::rest(1.0)
                    } else {
                        novaui::anim::Tween::new(0.0, 1.0, now, LYRIC_MOTION.into(), Ease::OutCubic)
                    };
                }
                self.just_seeked = false;
                // How far the motion has gone (the sung line's light, the scroll's share).
                let mut light = self.lyric_motion.at(now);
                if light < 1.0 {
                    // Arrived once what is left is below what the screen shows: under one 8-bit
                    // step of the light, under half a pixel of the scroll. The ease's last ~15 %
                    // of its time is spent there, in frames that would draw nothing.
                    let left = 1.0 - light;
                    let px = (target - self.lyric_scroll_from).abs() * left * ui.tree.scale();
                    if left < 1.0 / 255.0 && px < 0.5 {
                        self.lyric_motion = novaui::anim::Tween::rest(1.0);
                        light = 1.0;
                    }
                }
                if light < 1.0 {
                    self.lyric_scroll = self.lyric_scroll_from + (target - self.lyric_scroll_from) * light;
                    self.clock.frame();
                } else {
                    self.lyric_scroll = target;
                    self.leaving = None;
                    // A paused song's lines do not move: no wake for the next one.
                    self.line_clock.cancel();
                    if let Some(wait) = ly.until_next(pos).filter(|_| playing) {
                        self.line_clock.at(now + wait.as_secs_f64() + 0.005);
                    }
                }
                if self.lyrics_rail.grabbed() {
                    self.browse_glide.jump(self.lyrics_rail.offset() - self.lyric_scroll);
                    self.browse_until = now + BROWSE_SECS;
                }
                // Browsing over, the lyrics glide back to the sung line as any scroll moves.
                if now >= self.browse_until {
                    self.browse_glide.to(0.0, now, crate::views::SCROLL);
                }
                self.browse = self.browse_glide.at(now);
                if !self.browse_glide.done(now) {
                    self.clock.frame();
                } else if self.browse != 0.0 {
                    self.clock.at(self.browse_until);
                }
                let focus = h * LYRIC_FOCUS;
                // The window's middle, in the side panel's dips.
                let s = ui.tree.scale().max(0.01);
                let centre = (ui.tree.window_size().1 as f32 * 0.5 - ui.tree.rect(pane).y as f32) / s;
                let scroll = self.lyric_scroll + self.browse;
                let col = doc.id("lyrics");
                ui.tree.set_translate(col, 0.0, focus - scroll);
                // The sung group's words: how far its first line is sung, at the display's rate
                // while it plays (one line; the rest keep the group's own cadence).
                let sweep = self.sung.filter(|&i| self.groups.get(i).is_some_and(|g| g.words)).and_then(|i| {
                    let g = &ly.groups[i];
                    let at = g.sung(pos, ly.groups.get(i + 1).map(|n| n.at))?;
                    let total = g.lines.first().map_or(0, |l| l.chars().count()) as f32;
                    if playing && at < total.min(g.words.last().map_or(0.0, |w| w.1.end as f32)) {
                        self.clock.frame();
                    }
                    Some((i, at))
                });
                let sung_at = self.sung.map_or(-1.0, |l| l as f32);
                let secs = pos.as_secs_f32();
                for (i, g) in self.groups.iter().enumerate() {
                    let top = focus + self.anchors[i] - scroll;
                    let height = self.heights[i];
                    if top + height < -10.0 || top > h + 10.0 {
                        continue;
                    }
                    let mid = top + height * 0.5;
                    let edge = (mid / fade).clamp(0.0, 1.0) * ((h - mid) / fade).clamp(0.0, 1.0);
                    let light = if Some(i) == self.sung {
                        light
                    } else if Some(i) == self.leaving {
                        1.0 - light
                    } else {
                        0.0
                    };
                    let hover = if ui.tree.el(g.root).hovered() { 1.0 } else { 0.0 };
                    let swept = sweep.filter(|s| s.0 == i).map_or(-1.0, |s| s.1);
                    // Held inside the group's window: a group outside it does no work.
                    let time = secs.clamp(g.window.0, g.window.1);
                    if self.lyric_timed && playing && secs > g.window.0 && secs < g.window.1 {
                        self.clock.frame();
                    }
                    // `lyrics::STYLE_KEYS` order.
                    let vals = [light, i as f32 - sung_at, edge, hover, swept, time, mid - focus, mid - centre];
                    for (k, v) in g.keys.iter().zip(vals) {
                        doc.set(ui, k, v);
                    }
                }
                // The style's expressions follow now, not a frame late.
                if doc.update(ui) {
                    self.clock.frame();
                }
            }
        }
        self.sync_rails(ui, doc);
    }
}

/// Autoscroll's dead zone in dips before the doc is at hand (player.nui autoscroll_dead).
fn doc_dead(scale: f32) -> f32 {
    let _ = scale;
    10.0
}
