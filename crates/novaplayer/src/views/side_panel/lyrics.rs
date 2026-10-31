//! Side panel: the lyrics page.
use super::*;
use novaui::dsl::{Profile, Sheet};

/// What a lyric style may use: looks only (the player places, sizes and wraps the lines).
pub const STYLE_PROFILE: Profile = Profile {
    kinds: &["Text"],
    props: &[
        "text",
        "fg",
        "fg-alpha",
        "weight",
        "opacity",
        "translate",
        "scale",
        "sweep",
        "sweep-fg",
        "sweep-alpha",
        "units",
        "timing",
        "time",
        "u-translate",
        "u-scale",
        "u-opacity",
        "u-fg",
        "u-rotate",
        "u-glow",
        "text-rotate",
        "text-origin",
    ],
    max_bytes: 16 * 1024,
    max_nodes: 16,
};

/// The lyric style contract v1: the values the player writes for each visible group (see
/// ui/lyric-style.nui and docs/extensions.md). `time` (v1) is the song's position held inside
/// the group's window (`TIME_BEFORE` before its first part to `TIME_AFTER` after its last); the
/// group's `timing` (v1, `Group::timing`) is set once. `offset` (v1) is the group's middle less
/// the side panel's focus, in dips, as it scrolls: a style may place its lines by it (an arc,
/// one line held at the focus). `middle` (v1) is the same against the window's middle: a style
/// that shows one line holds it there.
pub const STYLE_KEYS: [&str; 8] = ["light", "distance", "edge", "hover", "sweep", "time", "offset", "middle"];

/// A group's time window around its parts, in seconds.
pub const TIME_BEFORE: f32 = 0.5;
pub const TIME_AFTER: f32 = 1.0;

// The layer fold (docs/novaplayer-spec.md, lyrics layers). Three things move apart rather than
// one track: the space under a line opens (the lines below give way), the layer line slides
// down out from under the line above, and its ink comes in once there is room to read it (and
// on the way back goes first, before the space closes). A spring drives each line (a press
// mid-way turns it from where it is, at the speed it has), and the lines start in a ripple
// outward from the one at the side panel's focus.

/// The spring: about half a second to rest, settling without a bounce (stiffness, damping;
/// natural frequency 2π/0.5 s, damping ratio 0.86).
const LAYER_K: f32 = 157.9;
const LAYER_C: f32 = 21.6;
/// The ripple: seconds per line away from the focus, and the most any line waits.
const LAYER_RIPPLE: f32 = 0.03;
const LAYER_RIPPLE_MAX: f32 = 0.24;
/// The share of the fold before the ink starts to come in (and after which it is gone, folding).
const LAYER_INK_FROM: f32 = 0.3;

/// The built-in style.
const BUILTIN_STYLE: &str = include_str!("../../../ui/lyric-style.nui");

/// A group's contract values, in `STYLE_KEYS` order, before the player first places it.
const STYLE_REST: [f32; 8] = [0.0, 1.0, 1.0, 0.0, -1.0, 0.0, 1.0e4, 1.0e4];

/// A lyric style's `.nui` read under the style profile.
pub fn parse_style(src: &str) -> Result<Sheet, String> {
    novaui::dsl::parse_fragment(src, &STYLE_PROFILE, None).map_err(|e| e.to_string())
}

impl SidePanel {
    // ---- lyrics ---------------------------------------------------------------------------

    /// The layers switched off changed: those that turned fold in or out from where they stand.
    pub fn set_layers_off(&mut self, off: &std::collections::BTreeSet<String>) {
        for (i, id) in library::LAYER_ORDER.iter().enumerate() {
            let on = !off.contains(*id);
            if self.layer_on[i] != on {
                self.layer_on[i] = on;
                self.layer_kick[i] = true;
            }
        }
    }

    /// The lyrics page's layer buttons: shown on the lyrics page for the layers its lyrics carry,
    /// lit while the layer is on.
    pub(super) fn present_layer_buttons(&self, ui: &mut Ui, doc: &mut Doc) {
        let page = self.shown.shows(Page::Lyrics);
        let carried = self.lyrics.as_ref().map(Lyrics::layer_ids).unwrap_or_default();
        for (i, id) in library::LAYER_ORDER.into_iter().enumerate() {
            let b = doc.id(&format!("layer-{id}"));
            let show = page && carried.contains(&id);
            let display = if show { Display::Flex } else { Display::None };
            if ui.tree.el(b).layout().display != display {
                ui.tree.set_layout(b, |l| l.display = display);
            }
            let on = self.layer_on[i];
            if ui.tree.el(b).lit() != on {
                ui.tree.set_lit(b, on);
            }
        }
    }

    /// The layer lines move toward their layer's state (before layout). Per line: the box opens
    /// to the line's share of its height (and of the gap above it), the line inside it rides at
    /// the box's foot so it slides down out from under the line above, and its ink follows the
    /// opening once `LAYER_INK_FROM` of it is there.
    pub(super) fn fold_layers(&mut self, ui: &mut Ui) {
        let now = ui.now();
        if self.layer_kick.iter().any(|k| *k) {
            // The ripple starts at the group at the side panel's focus (where the reader is).
            let at = self.lyric_scroll + self.browse;
            let focus = self.anchors.iter().rposition(|&a| a <= at + 0.5).or(self.sung).unwrap_or(0);
            for (i, g) in self.groups.iter_mut().enumerate() {
                let wait = (i.abs_diff(focus) as f32 * LAYER_RIPPLE).min(LAYER_RIPPLE_MAX);
                for l in g.layers.iter_mut().filter(|l| self.layer_kick[l.kind]) {
                    // A line already on its way turns at once; one at rest takes its turn.
                    l.wait = if l.v == 0.0 { wait } else { 0.0 };
                }
            }
            self.layer_kick = Default::default();
            if self.layers_placed {
                self.layers_placed = false;
                self.layer_clock = now;
            }
        }
        if self.layers_placed {
            return;
        }
        let dt = (now - self.layer_clock).clamp(0.0, 0.05) as f32;
        self.layer_clock = now;
        let mut moving = false;
        // The fold moves by whole pixels from rest: the side panel's column moves as a group in
        // whole pixels while the groups in it round where they stand, so a fraction left in the
        // heights would round apart from the column's offset and the sung line would step a
        // pixel back and forth as the spring settles. Whole pixels keep them together; the last
        // half pixel of the spring's tail is already rest.
        let s = ui.tree.scale().max(0.01);
        // `a` of `full` in whole pixels from the nearer end, so both ends are exact (a box need
        // not be whole pixels at rest); the fraction changes ends mid-way, while it moves fast.
        let part = |full: f32, a: f32| {
            let snap = |v: f32| (v * s).round() / s;
            if a >= 0.5 {
                full - snap(full * (1.0 - a))
            } else {
                snap(full * a)
            }
        };
        for g in &mut self.groups {
            let gap = ui.tree.el(g.root).layout().gap;
            for l in &mut g.layers {
                let to = if self.layer_on[l.kind] { 1.0 } else { 0.0 };
                let mut t = dt;
                let w = l.wait.min(t);
                l.wait -= w;
                t -= w;
                while t > 0.0 {
                    let h = t.min(1.0 / 240.0);
                    l.v += (LAYER_K * (to - l.x) - LAYER_C * l.v) * h;
                    l.x += l.v * h;
                    t -= h;
                }
                if l.wait <= 0.0 && (to - l.x).abs() < 1e-3 && l.v.abs() < 1e-2 {
                    l.x = to;
                    l.v = 0.0;
                } else {
                    moving = true;
                }
                let a = l.x.clamp(0.0, 1.0);
                let measured = ui.tree.size(l.line).1;
                if measured > 0.0 {
                    l.full = measured;
                }
                let h = l.full;
                let open = part(h, a);
                // Whole by its state, not by its measure: a line built this turn is not measured
                // yet (0 high), and a folded layer must not take its full height from that.
                let (height, top, dy) =
                    if a >= 1.0 { (Len::Auto, 0.0, 0.0) } else { (Len::Px(open), part(gap, a) - gap, open - h) };
                // Open by no whole pixel it leaves the group, its gap with it, as a line without
                // it: folded at rest, the spring's tail about it, a fold not started yet. A box
                // 0 high held in by a margin as tall as the gap is laid out a gap taller than no
                // box, so the group grew a gap for a moment and shrank again (the lines shook).
                // A line never laid out yet stays in while it opens, to be measured.
                let display =
                    if a < 1.0 && (h > 0.0 || a <= 0.0) && open * s < 0.5 { Display::None } else { Display::Flex };
                let ink = Ease::Smooth.at(((a - LAYER_INK_FROM) / (1.0 - LAYER_INK_FROM)).clamp(0.0, 1.0));
                ui.tree.set_layout(l.wrap, |s| {
                    s.height = height;
                    s.margin[0] = Len::Px(top);
                    s.display = display;
                });
                ui.tree.set_translate(l.line, 0.0, dy);
                ui.tree.set_opacity(l.wrap, ink);
            }
        }
        self.layers_placed = !moving;
        if moving {
            self.clock.frame();
        }
    }

    /// The lyrics page shows `l`: its state now, its elements at the next present.
    /// The opening song is known to have lyrics (`index::lyrics_known`), or what it has is
    /// known now (false): while they are expected, a song with none yet keeps the lyrics page
    /// (no turn to the queue and back while its file is read).
    pub fn expect_lyrics(&mut self, on: bool) {
        self.lyrics_expected = on;
    }

    pub fn set_lyrics(&mut self, l: Option<Lyrics>) {
        self.lyrics = l;
        self.lyric_key = (0, 0);
        self.sung = None;
        self.leaving = None;
        self.lyric_started = false;
        self.lyric_active = false;
        self.lyrics_rail.cancel();
        self.browse = 0.0;
        self.browse_glide.jump(0.0);
        // A song without lyrics (or whose lyrics are still on their way: a song's face lands
        // first without them) shows the queue; the lyrics page comes back when lyrics come,
        // unless another page was chosen meanwhile.
        if self.lyrics.is_none() && self.shown.page == Page::Lyrics && !self.lyrics_expected {
            self.shown.edit().page = Page::Queue;
            self.lyrics_wanted = true;
            self.relayout = true;
        } else if self.lyrics.is_some() && std::mem::take(&mut self.lyrics_wanted) && self.shown.page == Page::Queue {
            self.shown.edit().page = Page::Lyrics;
            self.turn_key = None;
            self.relayout = true;
        }
        self.lyrics_stale = true;
    }

    /// Draws the lines with the style `src` (a `.nui` the style profile admits; `None`: the
    /// built-in one) from the next present on. False: `src` is not admitted (the built-in is
    /// used).
    pub fn set_lyric_style(&mut self, src: Option<String>) -> bool {
        let ok = src.as_deref().is_none_or(|s| parse_style(s).is_ok());
        let src = src.filter(|_| ok);
        if src != self.lyric_style_src {
            self.lyric_style_src = src;
            self.restyle = true;
            self.lyrics_stale = true;
        }
        ok
    }

    /// The lyric groups' elements follow the lyrics set since the last present, once the lyrics
    /// page is up: a hidden page builds nothing (a song's ~50 groups took ~1.2 ms of the turn
    /// that opened it), and its old groups stay out of sight until then.
    pub(super) fn present_lyrics(&mut self, ui: &mut Ui, doc: &mut Doc) {
        let up = self.shown.shows(Page::Lyrics);
        if !up || !std::mem::take(&mut self.lyrics_stale) {
            return;
        }
        let col = doc.id("lyrics");
        doc.clear(ui, col);
        doc.forget_scope("ly");
        if std::mem::take(&mut self.restyle) {
            if let Some(a) = self.lyric_style.take() {
                doc.drop_adopted(&a);
            }
        }
        self.groups.clear();
        // New lines take the player's typography in this turn's tick.
        self.lyric_key = (0, 0);
        self.layers_placed = false;
        let Some(ly) = &self.lyrics else { return };
        let style = match &self.lyric_style {
            Some(style) => style.clone(),
            None => {
                // The chosen style, else (none chosen, or it no longer reads) the built-in one.
                let chosen = self.lyric_style_src.as_deref().and_then(|s| parse_style(s).ok());
                let sheet = chosen
                    .unwrap_or_else(|| parse_style(BUILTIN_STYLE).unwrap_or_else(|e| panic!("lyric-style.nui: {e}")));
                let style = doc.adopt(&sheet, &[], "lyric.");
                self.lyric_style = Some(style.clone());
                // A style that never reads `time` asks for no frames of its own.
                self.lyric_timed = sheet.reads().iter().any(|b| b == "time");
                style
            }
        };
        let duet = ly.duet();
        for (gi, g) in ly.groups.iter().enumerate() {
            let Ok(root) = doc.instantiate(ui, col, None, "LyricGroup", &[], &format!("lg{gi}")) else { continue };
            ui.tree.set_action(root, Some("lyric"));
            ui.tree.set_tag(root, gi as u64);
            // The group's contract values exist before its lines read them.
            let scope = format!("ly/{gi}");
            let keys = STYLE_KEYS.map(|k| format!("{scope}/{k}"));
            for (k, v) in keys.iter().zip(STYLE_REST) {
                doc.set(ui, k, v);
            }
            let timing = g.timing(ly.groups.get(gi + 1).map(|n| n.at));
            let window =
                (timing.first().map_or(0.0, |s| s.1) - TIME_BEFORE, timing.last().map_or(0.0, |s| s.2) + TIME_AFTER);
            let flat = timing.iter().flat_map(|&(c, a, b)| [Val::Num(c as f32), Val::Num(a), Val::Num(b)]);
            doc.set(ui, &format!("{scope}/timing"), Val::List(flat.collect()));
            doc.set(ui, &keys[5], window.0);
            let mut lines = Vec::with_capacity(g.lines.len().max(1));
            let texts: Vec<&str> =
                if g.lines.is_empty() { vec![" "] } else { g.lines.iter().map(String::as_str).collect() };
            for (k, line) in texts.into_iter().enumerate() {
                let comp = if k == 0 { "LyricLine" } else { "LyricReading" };
                match doc.instantiate_scoped(ui, root, None, &style, comp, &[Val::Str(line.to_owned())], &scope) {
                    Ok(l) => {
                        ui.tree.set_layout(l, |l| l.width = Len::Pct(100.0));
                        lines.push(l);
                    }
                    Err(e) => eprintln!("lyric style: {comp}: {e}"),
                }
            }
            let mut layers = Vec::with_capacity(g.layers.len());
            for (k, (id, text)) in g.layers.iter().enumerate() {
                let Some(li) = library::LAYER_ORDER.iter().position(|l| l == id) else { continue };
                let Ok(wrap) = doc.instantiate(ui, root, None, "LyricLayer", &[], &format!("lg{gi}x{k}")) else {
                    continue;
                };
                match doc.instantiate_scoped(ui, wrap, None, &style, "LyricReading", &[Val::Str(text.clone())], &scope)
                {
                    Ok(l) => {
                        ui.tree.set_layout(l, |l| {
                            l.width = Len::Pct(100.0);
                            l.shrink = 0.0;
                        });
                        let x = if self.layer_on[li] { 1.0 } else { 0.0 };
                        layers.push(LayerEl { kind: li, wrap, line: l, x, v: 0.0, wait: 0.0, full: 0.0 });
                    }
                    Err(e) => eprintln!("lyric style: LyricReading: {e}"),
                }
            }
            let far = duet && g.voice >= 2;
            self.groups.push(GroupEl { root, lines, keys, words: !g.words.is_empty(), window, layers, far });
        }
    }

    pub(super) fn style_lyrics(&mut self, ui: &mut Ui, doc: &novaui::dsl::Doc, single: bool) {
        // 7.x: the sung line's size and its reading's, from the side panel's width (`lyric_*`). In
        // one column the panel is the whole window and the lyrics stand centred in it; beside the
        // face, they start at its edge.
        let (size, line) = (doc.num("lyric_size"), doc.num("lyric_line"));
        let (rsize, rline) = (doc.num("lyric_reading"), doc.num("lyric_reading_line"));
        // A duet's voices stand apart, the first at the near side and the next at the far one,
        // in one column too (centred, they could not be told apart).
        let duet = self.groups.iter().any(|g| g.far);
        let base = if single && !duet { Align::Center } else { Align::Start };
        let key = (i32::from(single) + 2, (size * 16.0) as i32);
        if key == self.lyric_key {
            return;
        }
        self.lyric_key = key;
        for g in &self.groups {
            let align = if g.far { Align::End } else { base };
            let layer_lines = g.layers.iter().map(|l| l.line);
            for (j, l) in g.lines.iter().copied().chain(layer_lines).enumerate() {
                let t = match &ui.tree.el(l).content() {
                    Content::Text(t) => t.clone(),
                    _ => continue,
                };
                // Size, line height and wrap are the player's; the weight is the style's.
                let (s, lh) = if j == 0 { (size, line) } else { (rsize, rline) };
                ui.tree
                    .set_content(l, Content::Text(TextContent { size: s, line: lh, lines: 3, wrap: true, align, ..t }));
            }
        }
    }

    pub(super) fn lyric_layout(&mut self, ui: &Ui) {
        self.heights = self.groups.iter().map(|g| ui.tree.size(g.root).1).collect();
        let mut top = 0.0;
        self.anchors = self
            .heights
            .iter()
            .map(|h| {
                let a = top;
                top += h;
                a
            })
            .collect();
    }

    pub fn wheel_lyrics(&mut self, dy: f32, now: f64) {
        let last = self.anchors.last().copied().unwrap_or(0.0);
        let (lo, hi) = (-self.lyric_scroll, last - self.lyric_scroll);
        self.browse_glide.wheel(dy, now, lo, hi, crate::views::SCROLL);
        self.browse_until = now + BROWSE_SECS;
    }

    pub fn lyric_at(&self, g: usize) -> Option<Duration> {
        self.lyrics.as_ref().and_then(|l| l.groups.get(g)).map(|g| g.at)
    }

    /// A line was clicked to seek to it: the lyrics glide from where they stand to it, as a
    /// list glides to the playing song, not jump there (`after_layout`).
    pub fn lyric_pressed(&mut self) {
        self.seek_glides = true;
        self.browse_until = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A song known to have lyrics (`index::lyrics_known`) keeps the lyrics page while they
    /// are read: it does not turn to the queue and back with every song (a share's file, read
    /// cold, took longer than the face's stand-in). One not known as such turns as before.
    #[test]
    fn a_song_known_to_have_lyrics_keeps_the_lyrics_page_while_they_come() {
        let mut ui = novaui::Ui::new(400, 400, 1.0);
        let mut p = SidePanel::new(&mut ui, Default::default());
        let lyrics = || Lyrics::parse("[00:01.00]one\n[00:02.00]two\n");
        p.shown.edit().open = true;
        p.shown.edit().page = Page::Lyrics;
        p.set_lyrics(lyrics());
        // The next song: its lyrics on their way, known to come.
        p.expect_lyrics(true);
        p.set_lyrics(None);
        assert_eq!(p.shown.page, Page::Lyrics, "known to have lyrics: the page stays");
        p.set_lyrics(lyrics());
        assert_eq!(p.shown.page, Page::Lyrics);
        // Its file read, it has none after all: the queue.
        p.expect_lyrics(true);
        p.set_lyrics(None);
        p.expect_lyrics(false);
        p.set_lyrics(None);
        assert_eq!(p.shown.page, Page::Queue, "none after all");
        p.set_lyrics(lyrics());
        assert_eq!(p.shown.page, Page::Lyrics, "the page comes back with the next lyrics");
        // Not known: the queue while they come, then back.
        p.set_lyrics(None);
        assert_eq!(p.shown.page, Page::Queue);
        p.set_lyrics(lyrics());
        assert_eq!(p.shown.page, Page::Lyrics);
    }
}
