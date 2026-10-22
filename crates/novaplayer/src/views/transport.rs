//! The control bar and the volume tip.
//!
//! Nothing here keeps a copy of what it last wrote: the tree's setters and `Doc::set` drop a value
//! equal to the one they hold, so an unchanged transport costs the comparisons they do and marks
//! nothing. The bar's fills are percent lengths of their bars, so no size is read back.
use crate::app::*;
use crate::library;
use crate::library::queue::Order;
use crate::views::side_panel::Page;
use novaui::*;
use std::time::Duration;

pub(crate) fn update_transport(v: &mut Views, ui: &mut Ui, m: &Models) {
    let snap = &m.playback.snap;
    let pos = m.playback.shown_position();
    let dur = snap.duration.filter(|d| !d.is_zero());
    let empty = m.playing_queue().is_empty();
    let loading = snap.waiting() && !empty;
    let clock_running = snap.playing && !snap.buffering;
    let shown =
        v.transport.scrub.unwrap_or_else(|| dur.map_or(0.0, |d| (pos.as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0)));
    let elapsed = dur.map_or(pos, |d| d.mul_f32(shown));
    let active = dur.is_some() && shown > 0.0 && !loading;
    let (elapsed_text, remaining_text) = if empty {
        (String::new(), String::new())
    } else {
        let rem = if snap.opening {
            library::clock(Duration::ZERO)
        } else {
            dur.map(|d| format!("-{}", library::clock(d.saturating_sub(elapsed)))).unwrap_or_default()
        };
        (library::clock(elapsed), rem)
    };
    v.doc.set(ui, "elapsed", elapsed_text);
    v.doc.set(ui, "remaining", remaining_text);

    // Opening or waiting for audio: the same segment crossing the bar on the ease-in-out cubic.
    let sweep = v.id("track-sweep");
    if loading {
        // Placed once, just left of the bar; each frame moves it (a move, not a relayout) across
        // the bar's width, one sweep per period from the period's start (`sweep_*`).
        let (share, period) = (v.doc.num("sweep_share"), f64::from(v.doc.num("sweep_period")));
        ui.tree.set_layout(sweep, |l| {
            l.display = Display::Flex;
            l.width = Len::Pct(share * 100.0);
            l.inset[3] = Len::Pct(-share * 100.0);
        });
        let now = ui.now();
        let t0 = now - now % period;
        let e = novaui::anim::Tween::new(0.0, 1.0, t0, period, Ease::InOutCubic).at(now);
        let (w, _) = ui.tree.size(v.id("track-bar"));
        ui.tree.set_translate(sweep, (1.0 + share) * e * w, 0.0);
    } else {
        ui.tree.set_layout(sweep, |l| l.display = Display::None);
    }
    let fill = v.id("track-fill");
    // A thousandth of the bar is finer than a pixel at any real width, and coarse enough that
    // the 10 Hz wakes do not relayout the fill for a change nobody can see.
    let share = (shown * 1000.0).round() / 10.0;
    ui.tree.set_layout(fill, |l| {
        l.width = Len::Pct(if active { share } else { 0.0 });
        l.min_width = Len::Px(if active { 8.0 } else { 0.0 });
    });
    ui.tree
        .set_visual(fill, |v| v.bg = Some(if dur.is_some() && !loading { argb(0xE6FFFFFF) } else { argb(0x5CFFFFFF) }));

    // The sliders as a screen reader reads and sets them (`Event::Value`): seconds of the song,
    // the volume in percent; one arrow press is the keys' step (5 s, 5 %).
    let secs = dur.map_or(0.0, |d| d.as_secs_f64());
    let at = elapsed.as_secs_f64().floor().min(secs);
    ui.tree.set_semantics(v.id("track"), |a| a.range = Some(Range { value: at, min: 0.0, max: secs, step: 5.0 }));
    let vol = m.playback.volume;
    let pct = f64::from((vol * 100.0).round());
    ui.tree.set_semantics(v.id("gain"), |a| a.range = Some(Range { value: pct, min: 0.0, max: 100.0, step: 5.0 }));
    let gain = v.id("gain-fill");
    ui.tree.set_layout(gain, |l| {
        l.width = Len::Pct((vol * 1000.0).round() / 10.0);
        l.min_width = Len::Px(if vol > 0.0 { 6.0 } else { 0.0 });
    });
    ui.tree.set_icon(
        v.id("vol"),
        match vol {
            x if x <= 0.0 => Icon::SpeakerMuteFilled,
            x if x < 0.34 => Icon::Speaker0Filled,
            x if x < 0.67 => Icon::Speaker1Filled,
            _ => Icon::Speaker2Filled,
        },
    );

    label(ui, v.id("vol"), if vol <= 0.0 { "a11y.unmute" } else { "a11y.mute" });

    let playing = m.playback.opened && m.playback.wants_play;
    ui.tree.set_icon(v.id("play"), if playing { Icon::PauseFilled } else { Icon::PlayFilled });
    label(ui, v.id("play"), if playing { "a11y.pause" } else { "a11y.play" });
    ui.tree.set_disabled(v.id("play"), empty || snap.opening);
    ui.tree.set_disabled(v.id("prev"), empty);
    ui.tree.set_disabled(v.id("next"), m.playing_queue().next_pos().is_none());
    ui.tree.set_disabled(v.id("track"), dur.is_none() || snap.opening);
    let (order, order_off, said) = match m.playing_queue().order_mode {
        Order::RepeatOne => (Icon::ArrowRepeat1Filled, false, "a11y.order.one"),
        Order::RepeatAll => (Icon::ArrowRepeatAllFilled, false, "a11y.order.all"),
        Order::Off => (Icon::ArrowRepeatAllFilled, true, "a11y.order.off"),
        Order::Shuffle => (Icon::ArrowShuffleFilled, false, "a11y.order.shuffle"),
    };
    ui.tree.set_icon(v.id("order"), order);
    label(ui, v.id("order"), said);
    ui.tree.set_off(v.id("order"), order_off);
    ui.tree.set_disabled(v.id("q-sort"), !m.lists.shown_list.sortable());
    let words = v.now_playing.track.as_ref().is_some_and(|f| f.lyrics.is_some());
    ui.tree.set_layout(v.id("words"), |l| l.display = if words { Display::Flex } else { Display::None });
    ui.tree.set_lit(v.id("words"), v.side_panel.shown.page == Page::Lyrics && v.side_panel.shown.open);
    ui.tree.set_lit(v.id("queue"), v.side_panel.shown.page == Page::Queue && v.side_panel.shown.open);
    ui.tree.set_lit(v.id("info"), v.side_panel.shown.shows_info());
    let info = v.now_playing.track.is_some();
    ui.tree.set_layout(v.id("info"), |l| l.display = if info { Display::Flex } else { Display::None });
    let max = ui.window.maximized();
    ui.tree.set_icon(v.id("max"), if max { Icon::CaptionRestore } else { Icon::CaptionMaximize });
    label(ui, v.id("max"), if max { "a11y.restore" } else { "a11y.maximize" });

    // Wake obligations: the sweep moves every refresh, the clock on each second.
    if loading {
        v.transport.clock.frame();
    }
    if clock_running {
        // The label turns within 50 ms of the second: late enough to share the backdrop's
        // 10 Hz step most of the time, too early for anyone to see.
        let into = pos.as_millis() % 1000;
        let at = ui.now() + (1000 - into) as f64 / 1000.0 + 0.002;
        v.transport.clock.within(at, at + 0.05);
    }
    present_way_in(v, ui, m);
}

/// What a screen reader calls a button whose icon says its state: the string `key`.
fn label(ui: &mut Ui, id: Id, key: &str) {
    let text = ui.t(key, &[]);
    ui.tree.set_semantics(id, |a| a.label = Some(text));
}

/// The pointer reveal is time-driven: it follows the pointer and a deadline, not the model.
fn present_way_in(v: &mut Views, ui: &mut Ui, m: &Models) {
    let (cover_id, way) = (v.id("cover"), v.id("way-in"));
    let over = ui.tree.el(cover_id).hovered() || ui.tree.el(way).hovered();
    let linger = f64::from(v.doc.num("way_in_linger"));
    if over && (v.transport.way_in_until < ui.now() + linger - 0.1 || ui.pointer() != v.transport.last_pointer) {
        v.transport.way_in_until = ui.now() + linger;
    }
    v.transport.last_pointer = ui.pointer();
    let offered = v.side_panel.has_lyrics() || m.lists.shown_list.len() > 1;
    let want = if offered
        && !v.side_panel.shown.open
        && (ui.tree.el(way).hovered() || (over && ui.now() < v.transport.way_in_until))
    {
        1.0
    } else {
        0.0
    };
    let cur = ui.tree.prop(way, Prop::Opacity);
    let heading = ui.tree.is_animating(way, Prop::Opacity);
    if (cur - want).abs() > 1e-3 && (!heading || v.transport.way_in_to != want) {
        // In or out (`way_in_show`, `way_in_hide`): its time and curve.
        let motion = |k: &str| {
            let d = v.doc.num(&format!("way_in_{k}"));
            (d, crate::views::sheet_ease(v.doc, &format!("way_in_{k}_")))
        };
        let (d, e) = motion(if want > 0.0 { "show" } else { "hide" });
        ui.tree.animate(way, Prop::Opacity, want, d, e, ui.now());
        v.transport.way_in_to = want;
    }
    ui.tree.set_disabled(way, want == 0.0);
    if want > 0.0 && !ui.tree.el(way).hovered() {
        v.transport.clock.at(v.transport.way_in_until);
    }
}

/// Shows the volume number when asked, and takes it away when its second is up (present:
/// no geometry is read here).
pub(crate) fn present_vol_tip(v: &mut Views, ui: &mut Ui) {
    let t = v.id("vol-tip");
    if let Some(text) = v.transport.vol_tip.take() {
        v.doc.set(ui, "vol-tip", text);
        ui.tree.set_layout(t, |l| l.display = Display::Flex);
        v.transport.clock.at(v.transport.vol_tip_until);
    }
    if ui.tree.el(t).layout().display != Display::None && ui.now() >= v.transport.vol_tip_until {
        v.transport.vol_tip_until = 0.0;
        ui.tree.set_layout(t, |l| l.display = Display::None);
    }
}

/// Keeps the volume number centred over the button (post-layout phase: the button's box and
/// the tip's size are this turn's). Runs only while the tip is up.
pub(crate) fn place_vol_tip(v: &mut Views, ui: &mut Ui) {
    let t = v.id("vol-tip");
    if ui.tree.el(t).layout().display == Display::None {
        return;
    }
    let s = ui.scale();
    let r = ui.tree.rect(v.id("vol"));
    let (tw, th) = ui.tree.size(t);
    let x = (r.x as f32 + r.w as f32 / 2.0) / s - tw / 2.0;
    let y = r.y as f32 / s - th - v.doc.num("vol_tip_gap");
    let margin = v.doc.num("vol_tip_margin");
    ui.tree.set_layout(t, |l| l.inset = [Len::Px(y.max(0.0)), Len::Auto, Len::Auto, Len::Px(x.max(margin))]);
}

impl Player {
    /// The wheel turned the volume: state only; the tip is presented in the same host turn.
    pub fn show_vol_tip(&mut self, ui: &Ui) {
        self.transport.vol_tip = Some(format!("{}", (self.playback.volume * 100.0).round() as u32));
        self.transport.vol_tip_until = ui.now() + f64::from(self.doc.num("vol_tip_time"));
    }
}

pub struct Transport {
    /// Its frames and deadlines (`novaui::Clock`).
    pub clock: novaui::Clock,
    pub scrub: Option<f32>,
    pub gain_drag: bool,
    pub way_in_until: f64,
    pub way_in_to: f32,
    pub last_pointer: Option<(i32, i32)>,
    vol_tip: Option<String>,
    /// The volume number over its button shows until then (the wheel turned it).
    pub vol_tip_until: f64,
}

impl Transport {
    /// Its clock delivers `Msg::Transport`.
    pub fn new(ui: &mut novaui::Ui) -> Self {
        Transport {
            clock: ui.clock(crate::app::Msg::Transport),
            scrub: Default::default(),
            gain_drag: Default::default(),
            way_in_until: Default::default(),
            way_in_to: Default::default(),
            last_pointer: Default::default(),
            vol_tip: Default::default(),
            vol_tip_until: Default::default(),
        }
    }
}
