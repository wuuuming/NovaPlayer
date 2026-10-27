//! Desktop lyrics (novaplayer-spec.md, "Desktop lyrics"): the sung line and the one under it on a
//! plate over every window, in a NovaUI overlay on a thread of its own (`spawn_overlay`).
//!
//! The player's side (`DesktopLyrics`) derives what the overlay shows from its owners (the
//! settings, the face's lyrics and the layers shown, the engine's clock) and hands it over when
//! it changed: a `Feed` behind a lock, and the overlay's waker. The overlay (`Overlay`) draws it,
//! keeps the song's time between feeds itself, and sends its presses back as the player's
//! commands (`Command`'s text) by the player's waker (`Msg::DesktopLyrics`).

use crate::app::{Models, Msg, Views};
use crate::library::Lyrics;
use crate::sync::Locked;
use novaui::dsl::Doc;
use novaui::platform::{OverlayOptions, Place};
use novaui::*;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The overlay's sheet.
const SHEET: &str = include_str!("../../ui/desktop-lyrics.nui");

/// The settings it reads (`settings.txt`), each offered on `设置 › 桌面歌词`.
pub const SHOWN: &str = "desktop-lyrics";
pub const LOCKED: &str = "desktop-lyrics-lock";
pub const SIZE: &str = "desktop-lyrics-size";
pub const INK: &str = "desktop-lyrics-ink";
pub const SUNG: &str = "desktop-lyrics-sung";
pub const PLATE: &str = "desktop-lyrics-plate";
/// Where it stands once moved (`Place`: `x y`); empty, the default.
pub const PLACE: &str = "desktop-lyrics-place";

/// The font sizes offered (dips), the default first; − and + step through them in order.
pub const SIZES: [u32; 7] = [32, 24, 28, 36, 40, 48, 56];
/// The text's colours offered, the default first.
pub const INKS: [(&str, u32); 3] = [("white", 0xFFFFFFFF), ("cream", 0xFFFFF3C4), ("sky", 0xFFD2ECFF)];
/// The sung part's colours offered, the default first.
pub const SUNGS: [(&str, u32); 5] =
    [("blue", 0xFF60CDFF), ("pink", 0xFFFF8FC8), ("green", 0xFF7EE08A), ("gold", 0xFFFFC83D), ("white", 0xFFFFFFFF)];
/// The plate's opacities offered (percent of black), the default first.
pub const PLATES: [u32; 5] = [60, 0, 20, 40, 80];

/// A line change: the old lines fade out, the new fade in, half of it each.
const LINE_FADE: f32 = 0.25;
/// Hovering: the bar shows (unlocked), or the plate fades to this (locked).
const HOVER_FADE: f32 = 0.15;
const LOCKED_HOVER: f32 = 0.7;
/// How far above the bottom of the main display's work area it opens: an eighth of its height.
const LIFT: f32 = 0.125;
/// The second line's ink: a translation or reading under the line, or the next line.
const LAYER_ALPHA: f32 = 0.85;
const NEXT_ALPHA: f32 = 0.55;

/// How it looks, as the settings say.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub size: f32,
    pub ink: Color,
    pub sung: Color,
    /// The plate's black, 0 to 1.
    pub plate: f32,
    pub locked: bool,
    /// The settings' font family (`None`: the default).
    pub font: Option<String>,
}

impl Look {
    /// The look the settings name; a value not offered is the default.
    pub fn of(get: impl Fn(&str) -> Option<String>) -> Look {
        let pick = |key: &str| get(key).filter(|v| !v.is_empty());
        let size = pick(SIZE).and_then(|v| v.parse().ok()).filter(|s| SIZES.contains(s)).unwrap_or(SIZES[0]);
        let colour = |key: &str, table: &[(&str, u32)]| {
            let v = pick(key);
            argb(table.iter().find(|(n, _)| Some(*n) == v.as_deref()).unwrap_or(&table[0]).1)
        };
        let plate = pick(PLATE).and_then(|v| v.parse().ok()).filter(|p| PLATES.contains(p)).unwrap_or(PLATES[0]);
        Look {
            size: size as f32,
            ink: colour(INK, &INKS),
            sung: colour(SUNG, &SUNGS),
            plate: plate as f32 / 100.0,
            locked: pick(LOCKED).as_deref() == Some("on"),
            font: pick("font"),
        }
    }
}

/// The next size offered after `size` toward larger (`up`) or smaller ones, if there is one.
pub fn step_size(size: f32, up: bool) -> Option<u32> {
    let mut sizes = SIZES;
    sizes.sort_unstable();
    let size = size.round() as u32;
    if up {
        sizes.into_iter().find(|s| *s > size)
    } else {
        sizes.into_iter().rev().find(|s| *s < size)
    }
}

/// The song's time as the engine last said: `position` at `at`, going on while it plays.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SongClock {
    pub position: Duration,
    pub at: Option<Instant>,
    pub playing: bool,
}

impl SongClock {
    fn now(&self, now: Instant) -> Duration {
        match (self.playing, self.at) {
            (true, Some(at)) => self.position + now.saturating_duration_since(at),
            _ => self.position,
        }
    }
}

/// What the overlay shows, handed over whole when any of it changed.
#[derive(Clone, Debug, Default)]
pub struct Feed {
    /// The lyrics with the layers the player knows laid under their lines.
    pub lyrics: Option<Arc<Lyrics>>,
    /// The face's words, shown while there are no lyrics.
    pub title: String,
    pub artist: String,
    /// The layer shown under the lines (a translation, else a reading) when the lyrics carry
    /// one and the lyrics page shows it; `None`: the group's own second line, else the next.
    pub layer: Option<String>,
    pub clock: SongClock,
    pub look: Option<Look>,
    /// The overlay stays up; false: it closes.
    pub open: bool,
}

impl PartialEq for Feed {
    fn eq(&self, o: &Self) -> bool {
        let same_lyrics = match (&self.lyrics, &o.lyrics) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same_lyrics
            && self.title == o.title
            && self.artist == o.artist
            && self.layer == o.layer
            && self.clock == o.clock
            && self.look == o.look
            && self.open == o.open
    }
}

/// What the overlay tells the player.
pub enum FromOverlay {
    /// A press: the command's text (`Command`).
    Command(String),
    /// It could not open (no transparent window on this system).
    Failed(String),
}

/// A running overlay: what it is fed and its waker, once it has made one.
struct Link {
    feed: Arc<Mutex<Feed>>,
    waker: Arc<OnceLock<Waker>>,
}

impl Link {
    fn send(&self, feed: Feed) {
        *self.feed.locked() = feed;
        if let Some(w) = self.waker.get() {
            w.wake();
        }
    }
}

/// The player's side.
pub struct DesktopLyrics {
    link: Option<Link>,
    sent: Feed,
    /// The lyrics handed over, and the track's version they were made from.
    lyrics: (u64, Option<Arc<Lyrics>>),
    to_player: Sender<FromOverlay>,
    pub from_overlay: Receiver<FromOverlay>,
    wake: Waker,
}

impl DesktopLyrics {
    /// Its presses come as `Msg::DesktopLyrics`.
    pub fn new(ui: &mut Ui) -> Self {
        let (to_player, from_overlay) = channel();
        DesktopLyrics {
            link: None,
            sent: Feed::default(),
            lyrics: (u64::MAX, None),
            to_player,
            from_overlay,
            wake: ui.waker(Msg::DesktopLyrics),
        }
    }
}

/// The kept place (`PLACE`: `x y`), if one is.
fn kept_place(v: Option<&str>) -> Option<Place> {
    let (x, y) = v?.split_once(' ')?;
    Some(Place { x: x.trim().parse().ok()?, y: y.trim().parse().ok()? })
}

/// The text of a place, as `PLACE` keeps it.
pub fn place_text(p: Place) -> String {
    format!("{} {}", p.x, p.y)
}

/// Every present: the overlay opens or closes as the setting says, and is fed what it shows
/// when that changed; the face's button is lit while it is up.
pub fn present(v: &mut Views, ui: &mut Ui, m: &Models) {
    let want = m.settings.get(SHOWN) == Some("on");
    let d = &mut *v.desktop_lyrics;
    let button = v.doc.id("desk");
    if ui.tree.el(button).lit() != want {
        ui.tree.set_lit(button, want);
    }
    if !want {
        if let Some(link) = d.link.take() {
            link.send(Feed { open: false, ..Feed::default() });
            d.sent = Feed::default();
        }
        return;
    }
    let look = Look::of(|k| m.settings.get(k).map(str::to_owned));
    if d.link.is_none() {
        d.link = Some(open(d, &look, kept_place(m.settings.get(PLACE))));
    }
    // The lyrics are laid with their layers again only when the face changed.
    let track = &v.now_playing.track;
    if d.lyrics.0 != track.version() {
        let lyrics = track.as_ref().and_then(|f| f.lyrics.clone().map(|l| l.with_layers(&f.layers.layers)));
        d.lyrics = (track.version(), lyrics.map(Arc::new));
    }
    let off = &*v.now_playing.layers_off;
    let carried = d.lyrics.1.as_ref().map(|l| l.layer_ids()).unwrap_or_default();
    let layer = ["trans", "roma"].into_iter().find(|id| carried.contains(id) && !off.contains(*id)).map(str::to_owned);
    let words = v.now_playing.words();
    let feed = Feed {
        lyrics: d.lyrics.1.clone(),
        title: words.title.clone(),
        artist: words.artist.clone(),
        layer,
        clock: song_clock(m),
        look: Some(look),
        open: true,
    };
    if feed != d.sent {
        if let Some(link) = &d.link {
            link.send(feed.clone());
        }
        d.sent = feed;
    }
}

/// The song's time as the engine last said (a test's pinned position stands still).
fn song_clock(m: &Models) -> SongClock {
    let snap = &m.playback.snap;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(position) = m.playback.pinned_position.filter(|_| m.playback.opened) {
        return SongClock { position, at: None, playing: false };
    }
    SongClock {
        position: snap.position,
        at: snap.at,
        playing: snap.playing && !snap.buffering && m.playback.snap_is_open(),
    }
}

/// Starts the overlay at `place` (or the default), locked as the look says.
fn open(d: &DesktopLyrics, look: &Look, place: Option<Place>) -> Link {
    let feed = Arc::new(Mutex::new(Feed::default()));
    let waker: Arc<OnceLock<Waker>> = Arc::new(OnceLock::new());
    let opts = OverlayOptions {
        title: "NovaPlayer".into(),
        width: 600.0,
        height: 140.0,
        place,
        lift: LIFT,
        click_through: look.locked,
        backend: Default::default(),
    };
    let (f, w, out, wake) = (feed.clone(), waker.clone(), d.to_player.clone(), d.wake.clone());
    let (fail, fail_wake) = (d.to_player.clone(), d.wake.clone());
    let started = novaui::platform::spawn_overlay(opts, move |ui| {
        let fed = ui.waker(Fed);
        let _ = w.set(fed.clone());
        let mut o = Overlay::new(ui, f, out, wake);
        o.fed = Some(fed);
        Box::new(o)
    });
    // It could not open: the player hears why (it turns the setting off).
    let tell = move |why: String| {
        let _ = fail.send(FromOverlay::Failed(why));
        fail_wake.wake();
    };
    match started {
        Ok(thread) => {
            let _ = std::thread::Builder::new().name("novaplayer-desktop-lyrics".into()).spawn(move || {
                if let Ok(Err(e)) = thread.join() {
                    tell(e.to_string());
                }
            });
        }
        Err(e) => tell(e.to_string()),
    }
    Link { feed, waker }
}

// ---- the overlay (its own thread) -----------------------------------------------------------

/// The overlay's waker: a feed came.
struct Fed;
/// The overlay's clock: the sweep's frames, the next line, the fades.
struct Tick;

/// What the lines show: the group they are of (`None`: no lyrics, or before the first), and
/// their texts.
#[derive(Clone, Debug, PartialEq)]
struct Lines {
    group: Option<usize>,
    first: String,
    second: String,
    second_alpha: f32,
}

struct Overlay {
    doc: Doc,
    /// Its own waker's registration, held while it runs: the player's copy goes with the
    /// player's link the moment it closes, and the last feed (close) must still arrive.
    fed: Option<Waker>,
    feed: Arc<Mutex<Feed>>,
    /// The feed as last taken.
    now: Feed,
    to_player: Sender<FromOverlay>,
    player: Waker,
    clock: novaui::Clock,
    /// The lines shown, and those coming in while the old fade out (from when).
    shown: Option<Lines>,
    coming: Option<(Lines, f64)>,
    hovered: bool,
    /// What the window was last told: its click-through, and whether the look and hover stood
    /// so (`look`).
    through: bool,
    look: Option<(Look, bool)>,
}

impl Overlay {
    fn new(ui: &mut Ui, feed: Arc<Mutex<Feed>>, to_player: Sender<FromOverlay>, player: Waker) -> Overlay {
        let sheet = novaui::dsl::parse(SHEET).unwrap_or_else(|e| panic!("desktop-lyrics.nui: {e}"));
        Overlay {
            doc: Doc::new(sheet),
            fed: None,
            feed,
            now: Feed::default(),
            to_player,
            player,
            clock: ui.clock(Tick),
            shown: None,
            coming: None,
            hovered: false,
            through: false,
            look: None,
        }
    }

    fn tell(&self, c: String) {
        let _ = self.to_player.send(FromOverlay::Command(c));
        self.player.wake();
    }

    /// Everything shown follows the feed and the time: the look, the lines, the sweep.
    fn present(&mut self, ui: &mut Ui) {
        if !self.now.open {
            ui.window.close();
            return;
        }
        let Some(look) = self.now.look.clone() else { return };
        self.apply_look(ui, &look);
        let pos = self.now.clock.now(Instant::now());
        let (lines, sweep, next) = lines_at(&self.now, pos);
        let t = ui.now();
        // A change of lines fades the old out, then the new in (each half of the fade).
        match &self.shown {
            None => self.show_lines(ui, &lines),
            Some(s) if *s == lines => self.coming = None,
            Some(_) => {
                let start = match &self.coming {
                    Some((_, at)) => *at,
                    None => {
                        ui.tree.animate(self.doc.id("lines"), Prop::Opacity, 0.0, LINE_FADE / 2.0, Ease::Linear, t);
                        t
                    }
                };
                self.coming = Some((lines.clone(), start));
            }
        }
        if let Some((coming, start)) = self.coming.clone() {
            let half = f64::from(LINE_FADE / 2.0);
            if t >= start + half {
                self.show_lines(ui, &coming);
                self.coming = None;
                ui.tree.animate(self.doc.id("lines"), Prop::Opacity, 1.0, LINE_FADE / 2.0, Ease::Linear, t);
            } else {
                self.clock.at(start + half);
            }
        }
        let shown_group = self.shown.as_ref().and_then(|s| s.group);
        let sweep = if shown_group == lines.group { sweep } else { -1.0 };
        self.doc.set(ui, "sweep", sweep);
        // Frames while the line is being sung; then nothing until the next line.
        if self.now.clock.playing {
            if sweep >= 0.0 && sweep < lines.first.chars().count() as f32 {
                self.clock.frame();
            } else if let Some(next) = next {
                let wait = next.saturating_sub(pos).as_secs_f64();
                self.clock.at(t + wait + 0.005);
            }
        }
    }

    fn show_lines(&mut self, ui: &mut Ui, l: &Lines) {
        self.doc.set(ui, "line", l.first.as_str());
        self.doc.set(ui, "second", l.second.as_str());
        self.doc.set(ui, "has-second", !l.second.is_empty());
        self.doc.set(ui, "second-alpha", l.second_alpha);
        self.shown = Some(l.clone());
    }

    /// The look, the click-through and the hover's motion, when they changed.
    fn apply_look(&mut self, ui: &mut Ui, look: &Look) {
        if self.look.as_ref() == Some(&(look.clone(), self.hovered)) {
            return;
        }
        let was = self.look.replace((look.clone(), self.hovered));
        if was.as_ref().is_none_or(|(l, _)| l != look) {
            self.doc.set(ui, "size", look.size);
            self.doc.set(ui, "ink", look.ink);
            self.doc.set(ui, "sung", look.sung);
            self.doc.set(ui, "plate", [0.0, 0.0, 0.0, look.plate]);
            let stage = ui.window.work_area().map_or(1600.0, |(w, _)| w * 0.9);
            self.doc.set(ui, "stage", stage);
            if was.as_ref().is_none_or(|(l, _)| l.font != look.font) {
                ui.tree.set_font(look.font.as_deref());
            }
        }
        if self.through != look.locked || was.is_none() {
            self.through = look.locked;
            ui.window.set_click_through(look.locked);
        }
        let t = ui.now();
        let bar = if self.hovered && !look.locked { 1.0 } else { 0.0 };
        let plate = if self.hovered && look.locked { LOCKED_HOVER } else { 1.0 };
        let (b, p) = (self.doc.id("bar"), self.doc.id("plate"));
        ui.tree.animate(b, Prop::Opacity, bar, HOVER_FADE, Ease::Linear, t);
        ui.tree.animate(p, Prop::Opacity, plate, HOVER_FADE, Ease::Linear, t);
    }

    /// The play button shows what a press does.
    fn play_icon(&self, ui: &mut Ui) {
        let icon = if self.now.clock.playing { Icon::PauseFilled } else { Icon::PlayFilled };
        ui.tree.set_icon(self.doc.id("play"), icon);
    }
}

/// The lines at `pos`, how far the first is sung (chars; -1: not yet), and when the next line
/// starts.
fn lines_at(feed: &Feed, pos: Duration) -> (Lines, f32, Option<Duration>) {
    let none = |first: &str, second: &str| Lines {
        group: None,
        first: first.to_owned(),
        second: second.to_owned(),
        second_alpha: LAYER_ALPHA,
    };
    let Some(lyrics) = feed.lyrics.as_ref().filter(|l| !l.groups.is_empty()) else {
        return (none(&feed.title, &feed.artist), -1.0, None);
    };
    let groups = &lyrics.groups;
    // Before the first line: it stands, not yet sung, until it starts.
    let started = lyrics.current(pos);
    let i = started.unwrap_or(0);
    let g = &groups[i];
    let next = groups.get(i + 1);
    let first = g.lines.first().cloned().unwrap_or_else(|| "\u{266A}".to_owned());
    let (second, alpha) = match feed.layer.as_deref().and_then(|id| g.layers.iter().find(|(l, _)| l == id)) {
        Some((_, text)) => (text.clone(), LAYER_ALPHA),
        None => match g.lines.get(1) {
            Some(text) => (text.clone(), LAYER_ALPHA),
            None => (next.and_then(|n| n.lines.first()).cloned().unwrap_or_default(), NEXT_ALPHA),
        },
    };
    let lines_from = |sung: f32, next: Option<Duration>| {
        (Lines { group: Some(i), first: first.clone(), second: second.clone(), second_alpha: alpha }, sung, next)
    };
    if started.is_none() {
        return lines_from(-1.0, Some(g.at));
    }
    let sung = sung(g, pos, next.map(|n| n.at), first.chars().count());
    lines_from(sung, next.map(|n| n.at).filter(|at| *at > pos))
}

/// Chars of a group's first line sung at `pos`: by its words where they are stamped, else
/// evenly over its time (`Group::timing`).
fn sung(g: &crate::library::Group, pos: Duration, end: Option<Duration>, chars: usize) -> f32 {
    if let Some(s) = g.sung(pos, end) {
        return s;
    }
    let Some(&(_, from, to)) = g.timing(end).first() else { return chars as f32 };
    let k = if to > from { ((pos.as_secs_f32() - from) / (to - from)).clamp(0.0, 1.0) } else { 1.0 };
    k * chars as f32
}

impl App for Overlay {
    fn init(&mut self, ui: &mut Ui) {
        ui.tree.set_clear([0.0; 4]);
        let root = ui.tree.root();
        self.doc.build(ui, root).unwrap_or_else(|e| panic!("desktop-lyrics.nui: {e}"));
        for (k, v) in [("line", ""), ("second", "")] {
            self.doc.set(ui, k, v);
        }
        self.doc.set(ui, "has-second", false);
        self.doc.set(ui, "second-alpha", 1.0);
        self.doc.set(ui, "sweep", -1.0);
        self.now = self.feed.locked().clone();
        self.present(ui);
        self.play_icon(ui);
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        match ev {
            Event::Message(m) if m.is::<Fed>() => {
                self.now = self.feed.locked().clone();
                self.play_icon(ui);
            }
            Event::Message(_) => {}
            Event::Click { action: Some(a), .. } => {
                let size = self.now.look.as_ref().map_or(SIZES[0] as f32, |l| l.size);
                match a.name.as_str() {
                    "smaller" | "larger" => {
                        if let Some(s) = step_size(size, a.name == "larger") {
                            self.tell(format!("set:{SIZE}:{s}"));
                        }
                    }
                    _ => self.tell(a.to_string()),
                }
            }
            Event::Hover { id, on } if id == self.doc.id("content") => self.hovered = on,
            Event::Moved => {
                if let Some(p) = ui.window.place() {
                    self.tell(format!("set:{PLACE}:{}", place_text(p)));
                }
            }
            _ => return,
        }
        self.present(ui);
    }

    /// The window follows its content's laid-out size.
    fn after_layout(&mut self, ui: &mut Ui) {
        self.doc.update(ui);
        let r = ui.tree.rect(self.doc.id("content"));
        let k = ui.scale();
        ui.window.set_size(r.w as f32 / k, r.h as f32 / k);
    }

    /// The plate moves it; the bar's buttons are buttons.
    fn nc_hit(&self, ui: &Ui, x: i32, y: i32) -> NcHit {
        if ui.tree.rect(self.doc.id("plate")).contains(x, y) {
            NcHit::Caption
        } else {
            NcHit::Client
        }
    }
}

/// The overlay as it draws `feed` at `pos`, headless: for tests and screenshots.
#[cfg(any(test, feature = "test-support"))]
pub mod headless {
    use super::*;

    /// An overlay in `ui` fed `feed`, its first turn run and laid out: what its window shows.
    pub fn overlay(ui: &mut Ui, feed: Feed) -> Box<dyn App> {
        let (tx, _rx) = channel();
        let player = ui.waker(Fed);
        let mut o = Overlay::new(ui, Arc::new(Mutex::new(feed)), tx, player);
        o.init(ui);
        novaui::app::settle(&mut o, ui, 0);
        Box::new(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lrc(text: &str) -> Option<Arc<Lyrics>> {
        Lyrics::parse(text).map(Arc::new)
    }

    /// The second line is the layer shown, else the group's own second line, else the next
    /// line; before the first line, the first stands unsung.
    #[test]
    fn the_second_line_is_the_translation_else_the_next_line() {
        let s = Duration::from_secs;
        let mut feed =
            Feed { lyrics: lrc("[00:01.00]one\n[00:03.00]two\n[00:03.00]deux\n[00:05.00]three"), ..Feed::default() };
        let (l, sweep, next) = lines_at(&feed, Duration::from_millis(500));
        assert_eq!((l.first.as_str(), l.second.as_str(), sweep, next), ("one", "two", -1.0, Some(s(1))));
        let (l, ..) = lines_at(&feed, s(1));
        assert_eq!((l.first.as_str(), l.second.as_str(), l.second_alpha), ("one", "two", NEXT_ALPHA));
        let (l, ..) = lines_at(&feed, s(4));
        assert_eq!((l.first.as_str(), l.second.as_str(), l.second_alpha), ("two", "deux", LAYER_ALPHA));
        let layers = [crate::library::Layer { id: "trans".into(), name: "t".into(), lrc: "[00:01.00]un".into() }];
        feed.lyrics = feed.lyrics.map(|l| Arc::new((*l).clone().with_layers(&layers)));
        feed.layer = Some("trans".into());
        let (l, ..) = lines_at(&feed, s(2));
        assert_eq!((l.first.as_str(), l.second.as_str()), ("one", "un"));
        let (l, _, next) = lines_at(&feed, s(6));
        assert_eq!((l.first.as_str(), l.second.as_str(), next), ("three", "", None));
    }

    /// A line without word stamps is swept evenly to the next line; with them, by its words.
    #[test]
    fn a_line_is_swept_by_its_words_else_evenly() {
        let feed = Feed { lyrics: lrc("[00:00.00]abcd\n[00:04.00]x"), ..Feed::default() };
        let (_, sweep, _) = lines_at(&feed, Duration::from_secs(1));
        assert!((sweep - 1.0).abs() < 1e-4, "{sweep}");
        let feed = Feed { lyrics: lrc("[00:00.00]<00:00.00>ab<00:01.00>cd<00:03.00>\n[00:04.00]x"), ..Feed::default() };
        let (_, sweep, _) = lines_at(&feed, Duration::from_millis(2000));
        assert!((sweep - 3.0).abs() < 1e-4, "{sweep}");
    }

    /// − and + step through the sizes offered, and stop at the ends.
    #[test]
    fn the_size_steps_through_the_sizes_offered() {
        assert_eq!(step_size(32.0, true), Some(36));
        assert_eq!(step_size(32.0, false), Some(28));
        assert_eq!(step_size(56.0, true), None);
        assert_eq!(step_size(24.0, false), None);
        let look = Look::of(|k| (k == SIZE).then(|| "31".to_owned()));
        assert_eq!(look.size, SIZES[0] as f32, "a size not offered is the default");
    }

    /// The overlay paints its plate and nothing around it: the window is transparent past the
    /// plate's corners, the plate the settings' black where no text is.
    #[test]
    fn the_overlay_paints_only_its_plate() {
        let (w, h) = (600, 160);
        let mut ui = Ui::new(w, h, 1.0);
        let feed = Feed {
            title: "Title".into(),
            artist: "Artist".into(),
            look: Some(Look::of(|_| None)),
            open: true,
            ..Feed::default()
        };
        let _app = headless::overlay(&mut ui, feed);
        let mut be = novaui::backend::Software::new(w, h);
        novaui::backend::frame(&mut ui.tree, &mut be, 0.0);
        let px = novaui::backend::Backend::readback(&mut be);
        let at = |x: i32, y: i32| px[(y * w + x) as usize];
        assert_eq!(at(0, 0) >> 24, 0, "transparent where nothing paints");
        // The plate's left edge, half way down its lines: the default 60% black.
        let row = h - 30;
        let first = (0..w).find(|x| at(*x, row) >> 24 != 0).expect("a plate");
        let a = at(first + 4, row) >> 24;
        assert!((150..=156).contains(&a), "the plate's alpha {a}");
    }

    /// The kept place reads back as it was written.
    #[test]
    fn the_place_is_kept_as_written() {
        let p = Place { x: -1280, y: 1225 };
        assert_eq!(kept_place(Some(&place_text(p))), Some(p));
        assert_eq!(kept_place(Some("")), None);
        assert_eq!(kept_place(None), None);
    }
}
