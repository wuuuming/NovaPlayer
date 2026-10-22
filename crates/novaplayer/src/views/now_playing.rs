//! The now-playing view: the artwork (cover + colour field, crossfading between two image
//! slots), the words under it (two blocks dissolving into each other) and the backdrop's drift.
//!
//! This is the only code that writes those elements. `Player` decides *what* stands there (a
//! song's face, the stand-in, "未在播放") and hands it over as `Words` / a cover; the view decides
//! how it gets there (which slot, which block, the shared crossfade clock).

use crate::library;
use novaui::dsl::Doc;
use novaui::*;

/// One of the two artwork slots: the backdrop's field and the cover.
#[derive(Clone, Default, PartialEq)]
struct Slot {
    field: Option<ImageHandle>,
    cover: Option<ImageHandle>,
}

/// Artwork scale while the user holds the song (paused).
const ART_HELD: f32 = 0.92;
/// The backdrop's drift steps at 10 Hz.
const FIELD_FRAME: f64 = 0.1;
/// The drift may step this early to share a wake with another timer (`Clock::within`).
const FIELD_EARLY: f64 = 0.075;
/// Artwork and words crossfade on one clock; the artwork follows the sound's crossfade instead
/// when the song came in under one (`FrameIn::sound_fade`).
pub const CROSSFADE: f32 = 0.267;

/// What the artwork's running crossfade follows, taken at its first frame.
#[derive(Clone, Copy, Default)]
enum Lead {
    #[default]
    Unknown,
    /// Its own clock (`CROSSFADE`).
    Clock,
    /// The sound's crossfade: the coming song's share of the sound when the artwork began to
    /// change. The artwork's mix rises with that share, from there to the whole.
    Sound(f32),
}

/// The coming song's share of the sound (its power) at `k` (0..1) through the crossfade: the
/// players' gains are cos and sin of a quarter turn, so the share is the sine squared.
fn sound_share(k: f32) -> f32 {
    (k.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).sin().powi(2)
}

/// What the words block shows. `res` is the resolution line as the face knows it; `res_main` /
/// `res_note` are its two styled halves (a plugin's `style.res` template filled in), `option`
/// the option value's name (part of what makes the line differ).
#[derive(Clone, Default, PartialEq)]
pub struct Words {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub res: String,
    pub res_main: String,
    pub res_note: String,
    pub option: String,
}

impl Words {
    fn is_empty(&self) -> bool {
        self.title.is_empty() && self.artist.is_empty() && self.album.is_empty() && self.res.is_empty()
    }
}

/// What the player asked of the view since the last present, applied in that order (the words'
/// dissolve shares the artwork's clock, and words set over empty ones appear instead of fading,
/// so the sequence matters, not only the last state).
enum Op {
    Words(Words, bool),
    Cover(Option<library::Cover>),
    ClearCover,
}

/// What the frame needs from the player.
pub struct FrameIn {
    pub playing: bool,
    /// A song is open (the artwork stands).
    pub opened: bool,
    /// The user wants it playing (the artwork is full size).
    pub wants_play: bool,
    /// Where the side panel's shade starts, and how open the panel is (0..1).
    pub shade_from: f32,
    pub panel_open: f32,
    /// How far the sound's crossfade into the song shown is (0..1), while it is under way
    /// (`Snapshot::fade_progress`): an artwork change it meets follows it.
    pub sound_fade: Option<f32>,
}

pub struct NowPlaying {
    /// Its frames and deadlines (`novaui::Clock`).
    pub clock: novaui::Clock,
    /// Which image slot holds the current artwork (the other is the one fading out).
    slot_b: bool,
    slots: [Slot; 2],
    /// The field under no cover.
    default_field: Option<ImageHandle>,
    /// Which words block holds the song (the other dissolves away), and what each holds.
    words_b: bool,
    words: [Words; 2],
    /// The running crossfade on its own clock (linear over `CROSSFADE`), and what it follows.
    fade: Option<novaui::anim::Tween>,
    lead: Lead,
    drift: f32,
    last_drift_at: f64,
    pub track: novaui::Model<Option<crate::app::NowPlayingTrack>>,
    /// The open (`version`) the words on screen belong to: its own, or the stand-in put up when
    /// they did not land within the change's window (a slow network). Smaller than `version`
    /// while the previous song's words still show.
    pub track_version: u64,
    /// The same for the artwork (and the backdrop that follows it): the words and the picture of
    /// an online song land apart.
    pub cover_version: u64,
    /// When the open (`version`) began: the change's window runs from here (`stand_in_track`).
    pub switched_at: Option<(u64, f64)>,
    /// The song the face is for, compared with the next one (`song_changed`).
    song: Option<crate::library::Song>,
    /// A cover can be chosen for the playing song: a file on disk without its own picture. Set
    /// when the song opens (false) and when its face job lands, never asked of the disk here.
    pub cover_editable: bool,
    /// Lyrics layers switched off, as `plugin/layer` (kept between runs; a layer is on at first).
    pub layers_off: novaui::Model<std::collections::BTreeSet<String>>,
    /// `layout_state` has written the face's facts since the face last changed (`Msg::Track`
    /// clears it).
    pub(crate) layout_current: bool,
    /// Asked for, not yet on screen (`present`).
    pending: Vec<Op>,
    /// The resolution line opens the option flyout (the song's plugin has an option); written
    /// to the line's elements at present when it was armed again.
    option_armed: bool,
    option_stale: bool,
}

impl NowPlaying {
    /// Its clock delivers `Msg::NowPlaying`.
    pub fn new(ui: &mut novaui::Ui) -> Self {
        NowPlaying {
            clock: ui.clock(crate::app::Msg::NowPlaying),
            slot_b: Default::default(),
            slots: Default::default(),
            default_field: Default::default(),
            words_b: Default::default(),
            words: Default::default(),
            fade: Default::default(),
            lead: Default::default(),
            drift: Default::default(),
            last_drift_at: Default::default(),
            track: Default::default(),
            track_version: Default::default(),
            cover_version: Default::default(),
            switched_at: Default::default(),
            song: Default::default(),
            cover_editable: Default::default(),
            layers_off: Default::default(),
            layout_current: Default::default(),
            pending: Default::default(),
            option_armed: Default::default(),
            option_stale: Default::default(),
        }
    }
}

impl NowPlaying {
    /// The words block standing now: element names carry `-b` for block B.
    pub fn block(&self, n: &str) -> String {
        if self.words_b {
            format!("{n}-b")
        } else {
            n.to_owned()
        }
    }

    /// Puts `w` under the artwork at the next present. `fade`: the standing block dissolves into
    /// these (words that do not change start nothing; the first words after empty ones just
    /// appear).
    pub fn set_words(&mut self, w: Words, fade: bool) {
        self.pending.push(Op::Words(w, fade));
    }

    /// The words standing under the artwork (as presented).
    pub fn words(&self) -> &Words {
        &self.words[self.words_b as usize]
    }

    /// Crossfades the artwork to `cover` at the next present (none: the placeholder note over
    /// the default field).
    pub fn set_cover(&mut self, cover: Option<library::Cover>) {
        self.pending.push(Op::Cover(cover));
    }

    /// A song becomes the one playing under `version` (an open, or the sound going on into the
    /// song handed over), after the open `was`: the one place that decides whether the face
    /// changes. The same song again (repeat one, the playing song opened anew) changes nothing,
    /// so nothing moves: what of its face was up for `was` is up for `version`, nothing stands in
    /// while it is read again, and what lands for it is equal and starts nothing. Another song's
    /// change starts its window (`stand_in_track`).
    pub(crate) fn song_changed(&mut self, song: &crate::library::Song, was: u64, version: u64, now: f64) {
        self.switched_at = Some((version, now));
        if self.song.as_ref().is_some_and(|s| s.is(&song.path, song.span)) {
            if self.track_version == was {
                self.track_version = version;
            }
            if self.cover_version == was {
                self.cover_version = version;
            }
        } else {
            self.song = Some(song.clone());
        }
    }

    /// Nothing open: the note stands, the current cover goes (no crossfade).
    pub fn clear_cover(&mut self) {
        self.song = None;
        self.pending.push(Op::ClearCover);
    }

    /// The resolution line opens the option flyout (`on`) or is plain text.
    pub fn arm_option(&mut self, on: bool) {
        self.option_armed = on;
        self.option_stale = true;
    }

    /// Writes what was asked since the last present, in order.
    pub fn present(&mut self, ui: &mut Ui, doc: &mut Doc) {
        for op in std::mem::take(&mut self.pending) {
            match op {
                Op::Words(w, fade) => self.write_words(ui, doc, w, fade),
                Op::Cover(c) => self.write_cover(ui, doc, c),
                Op::ClearCover => self.write_clear_cover(ui, doc),
            }
        }
        if std::mem::take(&mut self.option_stale) {
            let on = self.option_armed;
            for n in ["res-line", "res-line-b"] {
                let id = doc.id(n);
                ui.tree.set_action(id, if on { Some("option-menu") } else { None });
                ui.tree.set_visual(id, |v| {
                    v.hit = on;
                    v.cursor = if on { Cursor::Hand } else { Cursor::Arrow };
                });
            }
        }
    }

    fn write_words(&mut self, ui: &mut Ui, doc: &mut Doc, w: Words, fade: bool) {
        let cur = usize::from(self.words_b);
        if self.words[cur] == w {
            return;
        }
        let b = if fade && !self.words[cur].is_empty() { !self.words_b } else { self.words_b };
        let sfx = if b { "-b" } else { "" };
        for (n, v) in [("title", &w.title), ("artist", &w.artist), ("album", &w.album)] {
            doc.set(ui, &format!("{n}{sfx}"), v.as_str());
        }
        doc.set(ui, &format!("res{sfx}"), w.res_main.as_str());
        doc.set(ui, &format!("res-note{sfx}"), w.res_note.as_str());
        for (n, shown) in [
            ("artist", !w.artist.is_empty()),
            ("album", !w.album.is_empty()),
            ("res-line", !w.res.is_empty() || !w.res_note.is_empty()),
        ] {
            let id = doc.id(&format!("{n}{sfx}"));
            ui.tree.set_layout(id, |l| l.display = if shown { Display::Flex } else { Display::None });
        }
        self.words[usize::from(b)] = w;
        let (up, away) = (doc.id(if b { "words-b" } else { "words-a" }), doc.id(if b { "words-a" } else { "words-b" }));
        if b != self.words_b {
            // One dissolve, linear, on the artwork's clock (whatever is left of it).
            let t0 = self.fade.map_or(ui.now(), |f| f.t0.max(ui.now() - 0.016));
            let left = (CROSSFADE - (ui.now() - t0) as f32).max(0.05);
            ui.tree.enter(up, Pose::CLEAR, left, Ease::Linear, ui.now());
            ui.tree.exit(away, Pose::CLEAR, left, Ease::Linear, ui.now(), Then::Stay);
            self.words_b = b;
        } else if !ui.tree.is_animating(up, Prop::Opacity) {
            ui.tree.set_opacity(up, 1.0);
            ui.tree.set_opacity(away, 0.0);
        }
    }

    fn write_cover(&mut self, ui: &mut Ui, doc: &Doc, cover: Option<library::Cover>) {
        let has_cover = cover.is_some();
        let slot = match cover {
            Some(c) => Slot { field: Some(c.field), cover: Some(c.image) },
            None => Slot { field: Some(library::default_field()), cover: None },
        };
        ui.tree.set_visible(doc.id("note"), !has_cover);
        // The same picture again (the next song of one album): nothing changes, so nothing fades.
        // Equal pixels are one handle, so this compares content.
        if self.slots[usize::from(self.slot_b)] == slot {
            return;
        }
        let to_b = !self.slot_b;
        self.slots[usize::from(to_b)] = slot;
        self.slot_b = to_b;
        self.fade = Some(novaui::anim::Tween::new(0.0, 1.0, ui.now(), CROSSFADE.into(), Ease::Linear));
        self.lead = Lead::Unknown;
    }

    fn write_clear_cover(&mut self, ui: &mut Ui, doc: &Doc) {
        ui.tree.set_visible(doc.id("note"), true);
        self.slots[usize::from(self.slot_b)].cover = None;
    }

    /// Per frame: the backdrop's drift, the crossfade's progress, the artwork's breathing.
    pub fn frame(&mut self, ui: &mut Ui, doc: &Doc, f: FrameIn) {
        let now = ui.now();
        if f.playing {
            // A step every 75..100 ms: the drift advances by the time that passed, so a step
            // taken early (sharing the clock's or a lyric's wake) moves it no faster.
            if now - self.last_drift_at >= FIELD_EARLY {
                self.drift += (now - self.last_drift_at).min(0.2) as f32;
                self.last_drift_at = now;
            }
            self.clock.within(self.last_drift_at + FIELD_EARLY, self.last_drift_at + FIELD_FRAME);
        } else {
            self.last_drift_at = now;
        }
        let (cur, old) = (&self.slots[usize::from(self.slot_b)], &self.slots[usize::from(!self.slot_b)]);
        // Only an old backdrop or cover gives the crossfade something to mix (the first artwork
        // has none): no frames for a fade that draws the same thing.
        let mixes = old.field.is_some() || (f.opened && old.cover.is_some() && cur.cover.is_some());
        let fade = match self.fade {
            Some(clock) => {
                if let Lead::Unknown = self.lead {
                    self.lead = f.sound_fade.map_or(Lead::Clock, |k| Lead::Sound(sound_share(k)));
                }
                let k = match self.lead {
                    // In step with the sound: it stands while the sound is held, and a seek that
                    // ends the sound's crossfade ends this one.
                    Lead::Sound(from) => {
                        f.sound_fade.filter(|_| from < 0.999).map_or(1.0, |k| (sound_share(k) - from) / (1.0 - from))
                    }
                    _ => clock.at(now),
                }
                .clamp(0.0, 1.0);
                let runs = f.playing || !matches!(self.lead, Lead::Sound(_));
                if k >= 1.0 {
                    self.fade = None;
                } else if mixes && runs {
                    self.clock.frame();
                }
                k
            }
            None => 1.0,
        };
        let shade = 0.45 * f.panel_open;
        // The darkening starts at the side panel's left edge: physical px from the field's left.
        let params = [self.drift, f.shade_from * ui.tree.scale(), shade, 0.0];
        let fc = |image: &ImageHandle, opacity| EffectContent {
            effect: field_effect(),
            image: image.clone(),
            params,
            opacity,
        };
        let base = match &cur.field {
            Some(h) => h,
            None => self.default_field.get_or_insert_with(library::default_field),
        };
        let field = match &old.field {
            Some(of) if fade < 1.0 => Content::Effect(fc(of, 1.0), Some(fc(base, fade))),
            _ => Content::Effect(fc(base, 1.0), None),
        };
        ui.tree.set_content(doc.id("field"), field);
        let cover = doc.id("cover");
        // The cover is the one picture that may go past SDR white on an HDR display (a gain-map
        // JPEG's highlights); the field made from it, and every other image, stay SDR.
        let ic =
            |image: &ImageHandle, opacity| ImageContent { image: image.clone(), opacity, fit_square: true, hdr: true };
        let cc = match (&cur.cover, &old.cover) {
            (Some(cc), Some(oc)) if f.opened && fade < 1.0 => {
                Content::ImageCrossfade(ImageCrossfade { from: ic(oc, 1.0), to: ic(cc, 1.0), mix: fade })
            }
            (Some(cc), _) if f.opened => Content::Image(ic(cc, 1.0)),
            _ => Content::None,
        };
        ui.tree.set_content(cover, cc);
        ui.tree.set_visual(cover, |v| v.bg = Some(argb(0x0FFFFFFF)));
        // Artwork breathes: smaller while held (0.25 s). It follows the user's intent alone, so
        // no path between songs (next, repeat one, a list wrapping, a reopen) can dip it.
        let target = if f.opened && f.wants_play { 1.0 } else { ART_HELD };
        if (ui.tree.prop(cover, Prop::Scale) - target).abs() > 0.001 && !ui.tree.is_animating(cover, Prop::Scale) {
            ui.tree.animate(cover, Prop::Scale, target, 0.25, Ease::Linear, now);
        }
    }
}

/// The backdrop: the cover's colour lattice (a small texture) sampled bilinearly through a
/// flowing warp, darkened right of the side panel's edge. An image effect: params are the drift
/// (seconds), where the darkening starts (physical px from the field's left), how dark (0..1).
/// The GPU program is `field.wgsl`, compiled by build.rs.
static FIELD: novaui::image::effect::EffectProgram = novaui::image::effect::EffectProgram {
    name: "novaplayer.field",
    gpu: novaui::effect_shaders!("field"),
    cpu: field_cpu,
};

fn field_cpu(p: &novaui::image::effect::EffectPixel) -> [f32; 4] {
    let [drift, shade_from, shade, _] = p.params;
    let (nx, ny) = (p.size.0 - 1.0, p.size.1 - 1.0);
    let cell = p.w / nx;
    let gx = p.x / p.w * nx;
    let gy = p.y / p.h * ny;
    let rim = gx.min(nx - gx).min(gy.min(ny - gy)).clamp(0.0, 1.0);
    let dx = 0.34 * (drift * 0.113 + gy * 1.3 + gx * 0.7).sin() * rim;
    let dy = 0.34 * (drift * 0.097 + gy * 0.9 + gx * 1.9).cos() * rim;
    let c = (p.sample)((gx - dx).clamp(0.0, nx) + 0.5, (gy - dy).clamp(0.0, ny) + 0.5);
    let keep = 1.0 - shade * ((p.x - (shade_from - cell)) / cell.max(1.0)).clamp(0.0, 1.0);
    [c[0] * keep, c[1] * keep, c[2] * keep, c[3]]
}

fn field_effect() -> novaui::image::effect::Effect {
    static E: std::sync::OnceLock<novaui::image::effect::Effect> = std::sync::OnceLock::new();
    *E.get_or_init(|| novaui::image::effect::register(&FIELD))
}
