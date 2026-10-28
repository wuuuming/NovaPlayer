//! The settings page (novaplayer-spec.md, "Settings page"): in the face's place over the same backdrop,
//! under the title bar. The app menu's last row opens it; the menu button, turned into a back
//! button while it is up, and Esc return to the face, which stood as it was. Its first level is
//! laid out in `player.nui` (`#settings-root`); a deeper one (the plugins, a plugin, the
//! effects, an effect's parameters) is built here into `#settings-level`. The title is a breadcrumb of the levels. This view writes what
//! the sheet cannot say itself and scrolls the page.
use crate::app::command::Command;
use crate::app::{Models, Views};
use crate::views::menus::dropdown::Choice;
use crate::views::plugin_view::{Conn, PluginView};
use novaui::dsl::{Doc, Val};
use novaui::widgets::switch;
use novaui::*;

/// The crossfade between the face and the page (WinUI's page crossfade).
const FADE: f32 = 0.25;

/// Where on the page the user is. No history: each level's parent is fixed.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Level {
    #[default]
    Root,
    /// `设置 › 插件`: the installed plugins.
    Plugins,
    /// `设置 › 音效`: the CLAP effects the sound plays through.
    Effects,
    /// `设置 › 桌面歌词`: the desktop lyrics' choices (laid out in the sheet).
    DesktopLyrics,
    /// `设置 › 插件 › <name>`: one plugin (its id).
    Plugin(String),
    /// `设置 › 音效 › <name>`: the parameters of the effect at this place in the chain.
    Effect(usize),
}

impl Level {
    /// The sheet's container a level is in: the first level's own (`#settings-root`), the desktop
    /// lyrics' (`#settings-desktop-lyrics`, laid out in the sheet), a level built here
    /// (`#settings-level`).
    pub fn container(&self) -> &'static str {
        match self {
            Level::Root => "settings-root",
            Level::DesktopLyrics => "settings-desktop-lyrics",
            _ => "settings-level",
        }
    }
}

impl Level {
    fn depth(&self) -> u8 {
        match self {
            Level::Root => 0,
            Level::Plugins | Level::Effects | Level::DesktopLyrics => 1,
            Level::Plugin(_) | Level::Effect(_) => 2,
        }
    }

    fn parent(&self) -> Option<Level> {
        match self {
            Level::Root => None,
            Level::Plugins | Level::Effects | Level::DesktopLyrics => Some(Level::Root),
            Level::Plugin(_) => Some(Level::Plugins),
            Level::Effect(_) => Some(Level::Effects),
        }
    }
}

pub struct SettingsPage {
    pub open: bool,
    /// How far the page has come in over the face (0: the face, 1: the page).
    pub amount: f32,
    /// Its motion toward the page or the face.
    fade: novaui::anim::Tween,
    /// The face and the page stand as `amount` says, the motion over.
    at_rest: bool,
    pub clock: novaui::Clock,
    /// How far the page is scrolled (dips from its top), gliding as every scroll does.
    scroll: novaui::anim::Glide,
    pub level: Level,
    /// The breadcrumb's segments, the page first, and as built: for this level, at this width (px).
    trail: Vec<(String, Level)>,
    crumbs: Option<(Level, i32)>,
    /// The sliders of an effect's parameters as built.
    sliders: Vec<Slider>,
    /// The slider being dragged (its bar); `stale`: the parameters changed meanwhile in a way
    /// only building the level again shows, done when it is let go.
    pub dragging: Option<Id>,
    stale: bool,
    /// The switches as built (`Toggle`, instance name; on), and the level they were built for:
    /// one built again in its place, on that level, moves from how the one before stood.
    switches: Vec<(String, bool, switch::Parts)>,
    switches_for: Option<Level>,
    was: Vec<(String, switch::Was)>,
}

impl SettingsPage {
    /// Its clock delivers `Msg::Settings`.
    pub fn new(ui: &mut novaui::Ui) -> Self {
        SettingsPage {
            open: Default::default(),
            amount: Default::default(),
            fade: Default::default(),
            at_rest: Default::default(),
            clock: ui.clock(crate::app::Msg::Settings),
            scroll: Default::default(),
            level: Default::default(),
            trail: Default::default(),
            crumbs: Default::default(),
            sliders: Default::default(),
            dragging: Default::default(),
            stale: Default::default(),
            switches: Default::default(),
            switches_for: Default::default(),
            was: Default::default(),
        }
    }
}

/// A slider of an effect's parameter (`ParamSlider` in `player.nui`, instance `name`):
/// parameter `id`, `min` to `max` (whole steps only: `stepped`), of the effect at `at`.
#[derive(Clone, Debug)]
struct Slider {
    name: String,
    bar: Id,
    at: usize,
    id: u32,
    min: f64,
    max: f64,
    stepped: bool,
}

/// What About names: this build's version, its commit and the Player-Core commit it pins
/// (`build.rs`).
#[cfg(not(any(test, feature = "test-support")))]
fn build_revs() -> (&'static str, &'static str, &'static str) {
    (env!("CARGO_PKG_VERSION"), env!("NOVA_COMMIT"), env!("NOVA_PLAYER_CORE_REV"))
}

/// Tests see a fixed version and commits: a screenshot must not change with every commit.
#[cfg(any(test, feature = "test-support"))]
pub fn build_revs() -> (&'static str, &'static str, &'static str) {
    ("0.0.1", "0000000", "0000000")
}

impl SettingsPage {
    /// What the About section names: this build's version and commit, the playback core, and
    /// the terms of what the Windows build ships (TAK's decoder; ASIO's SDK, under GPL-3.0).
    pub fn init(&mut self, ui: &mut Ui, doc: &mut Doc) {
        let (version, commit, core_rev) = build_revs();
        let line = if commit.is_empty() {
            ui.t("about.build", &[("version", version)])
        } else {
            ui.t("about.build.commit", &[("version", version), ("commit", commit)])
        };
        doc.set(ui, "about-tak", cfg!(windows));
        doc.set(ui, "about-asio", cfg!(all(windows, feature = "asio")));
        doc.set(ui, "about-version", line);
        let core =
            if cfg!(windows) { ui.t("about.core.rev", &[("rev", core_rev)]) } else { ui.t("about.core.linux", &[]) };
        doc.set(ui, "about-core", core);
    }

    /// Opens the page at its top, or closes it.
    pub fn show(&mut self, ui: &mut Ui, doc: &mut Doc, open: bool) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.scroll.jump(0.0);
        let to = if open { 1.0 } else { 0.0 };
        self.fade = novaui::anim::Tween::new(self.amount, to, ui.now(), FADE.into(), Ease::Bezier(0.1, 0.9, 0.2, 1.0));
        self.at_rest = false;
        // The menu button is the page's way back while it is up.
        let icon = if open { Icon::ArrowLeft } else { Icon::Navigation };
        ui.tree.set_icon(doc.id("app-menu"), icon);
        // A field on the face keeps no keys while the page is over it.
        ui.set_focus(None);
        self.tick(ui, doc);
        self.place(ui, doc);
    }

    /// The crossfade (every present): the page comes in over the face, or goes; each is shown
    /// while any of it is, on its own layer while they mix.
    pub fn tick(&mut self, ui: &mut Ui, doc: &Doc) {
        if self.at_rest {
            return;
        }
        let now = ui.now();
        self.amount = self.fade.at(now);
        let mixing = !self.fade.done(now);
        if mixing {
            self.clock.frame();
        } else {
            self.at_rest = true;
        }
        let (face, page) = (doc.id("face"), doc.id("settings-page"));
        ui.tree.set_promoted(face, mixing);
        ui.tree.set_opacity(face, 1.0 - self.amount);
        ui.tree.set_visible(face, self.amount < 1.0);
        ui.tree.set_opacity(page, self.amount);
        ui.tree.set_visible(page, self.amount > 0.0);
    }

    /// The wheel over the page (positive: the content moves up).
    pub fn wheel(&mut self, ui: &mut Ui, doc: &mut Doc, dy: f32) {
        let reach = self.reach(ui, doc);
        self.scroll.wheel(dy, ui.now(), 0.0, reach, crate::views::SCROLL);
        self.place(ui, doc);
    }

    /// Scrolls the page so `id` (an element on it) is in view: the keyboard focus came to it, or
    /// a screen reader asked to see it. Nothing for an element elsewhere.
    pub fn reveal(&mut self, ui: &mut Ui, doc: &mut Doc, id: Id) {
        let page = doc.id("settings-page");
        if !ui.tree.is_descendant(id, page) {
            return;
        }
        let (r, view, s) = (ui.tree.rect(id), ui.tree.rect(page), ui.scale());
        let margin = doc.num("settings_reveal_margin");
        // From where the page stands now (the rects are its), toward the element.
        let at = self.scroll.at(ui.now());
        let to = if r.y < view.y {
            at - ((view.y - r.y) as f32 / s + margin)
        } else if r.b() > view.b() {
            at + (r.b() - view.b()) as f32 / s + margin
        } else {
            return;
        };
        let reach = self.reach(ui, doc);
        self.scroll.to(to.clamp(0.0, reach), ui.now(), crate::views::SCROLL);
        self.place(ui, doc);
    }

    /// How far the page's content reaches past its bottom (dips): its scroll's end.
    fn reach(&self, ui: &Ui, doc: &Doc) -> f32 {
        let (_, view) = ui.tree.size(doc.id("settings-page"));
        let (_, content) = ui.tree.size(doc.id("settings-body"));
        (content - view).max(0.0)
    }

    /// Holds the scroll within the page's content as laid out (after a resize too), stands the
    /// page where its glide is now, and fits the breadcrumb to the width it has.
    pub fn place(&mut self, ui: &mut Ui, doc: &mut Doc) {
        let body = doc.id("settings-body");
        let (now, reach) = (ui.now(), self.reach(ui, doc));
        self.scroll.hold(0.0, reach, now, crate::views::SCROLL);
        if !self.scroll.done(now) {
            self.clock.frame();
        }
        let y = -self.scroll.at(now);
        if (ui.tree.prop(body, Prop::Y) - y).abs() > 0.01 {
            ui.tree.set_translate(body, 0.0, y);
        }
        let w = ui.tree.rect(doc.id("settings-crumbs")).w;
        if self.crumbs.as_ref() != Some(&(self.level.clone(), w)) {
            self.crumbs = Some((self.level.clone(), w));
            self.build_crumbs(ui, doc, w);
        }
    }

    /// The title: every level down to this one, an earlier one a link to it. When they do not
    /// fit `w` (px), the earliest go, and `\u{2026}` in their place goes up one level.
    fn build_crumbs(&self, ui: &mut Ui, doc: &mut Doc, w: i32) {
        let row = doc.id("settings-crumbs");
        doc.clear(ui, row);
        if self.trail.is_empty() {
            return;
        }
        // The whole trail first (it mostly fits); the crumbs' type and the separator's room are
        // read off it as the sheet laid them (`style crumb`, `CrumbSep`, the row's gap).
        self.fill_crumbs(ui, doc, 0);
        let Some((size, weight)) = doc.get("crumb0.t").and_then(|t| match &ui.tree.el(t).content() {
            Content::Text(c) => Some((c.size, c.weight)),
            _ => None,
        }) else {
            return;
        };
        let icon = doc.get("crumb0-sep.sep").map_or(0.0, |s| match &ui.tree.el(s).content() {
            Content::Icon(i) => i.size,
            _ => 0.0,
        });
        let sep = (icon + 2.0 * ui.tree.el(row).layout().gap) * ui.scale();
        let widths: Vec<f32> = self.trail.iter().map(|(t, _)| ui.text_width(t, size, weight)).collect();
        let more = ui.text_width("\u{2026}", size, weight) + sep;
        let fits = |from: usize| {
            let lead = if from > 0 { more } else { 0.0 };
            lead + widths[from..].iter().sum::<f32>() + sep * (widths.len() - from - 1) as f32 <= w as f32
        };
        let last = self.trail.len() - 1;
        let from = (0..last).find(|&k| fits(k)).unwrap_or(last);
        if from > 0 {
            doc.clear(ui, row);
            self.fill_crumbs(ui, doc, from);
        }
        // The chevrons centre on the words' capitals (macOS centres a symbol in text so), not on
        // their line's middle, which Noto Sans' short descent leaves above the capitals'.
        let dy = ui.font_metrics(size, weight).cap_middle();
        for k in self
            .trail
            .iter()
            .enumerate()
            .map(|(k, _)| format!("crumb{k}-sep.sep"))
            .chain(["crumb-up-sep.sep".to_owned()])
        {
            if let Some(sep) = doc.get(&k) {
                ui.tree.set_translate(sep, 0.0, dy);
            }
        }
    }

    /// The trail from level `from` on (an earlier one: `\u{2026}`, up one level, before it).
    fn fill_crumbs(&self, ui: &mut Ui, doc: &mut Doc, from: usize) {
        let row = doc.id("settings-crumbs");
        let last = self.trail.len() - 1;
        if from > 0 {
            let up = Command::Settings(self.level.parent().unwrap_or_default()).to_string();
            let _ = doc.instantiate(ui, row, None, "Crumb", &[Val::Str("\u{2026}".into()), Val::Str(up)], "crumb-up");
            let _ = doc.instantiate(ui, row, None, "CrumbSep", &[], "crumb-up-sep");
        }
        for (k, (text, level)) in self.trail.iter().enumerate().skip(from) {
            let name = format!("crumb{k}");
            if k == last {
                let _ = doc.instantiate(ui, row, None, "CrumbHere", &[Val::Str(text.clone())], &name);
            } else {
                let _ = doc.instantiate(
                    ui,
                    row,
                    None,
                    "Crumb",
                    &[Val::Str(text.clone()), Val::Str(Command::Settings(level.clone()).to_string())],
                    &name,
                );
                let _ = doc.instantiate(ui, row, None, "CrumbSep", &[], &format!("{name}-sep"));
            }
        }
    }
}

/// Opens the page at `level` (the app menu's 设置: its first).
pub fn open(v: &mut Views, ui: &mut Ui, m: &Models, level: Level) {
    v.settings_page.level = level;
    v.settings_page.switches_for = None;
    build(v, ui, m);
    v.settings_page.show(ui, v.doc, true);
}

/// Shows `level`, at its top; its content turns in as the queue's second level does (the title
/// stands).
pub fn go(v: &mut Views, ui: &mut Ui, m: &Models, level: Level) {
    let page = &mut v.settings_page;
    let deeper = level.depth() > page.level.depth();
    page.level = level;
    page.scroll.jump(0.0);
    ui.set_focus(None);
    build(v, ui, m);
    let shown = v.settings_page.level.container();
    let c = crate::views::layout::Scale::of(v.doc).chrome;
    crate::views::side_panel::turn_in(ui, v.doc.id(shown), deeper, c, ui.now());
    v.settings_page.clock.frame();
}

/// Up one level (the mouse's back button): from the first level, to playback.
pub fn up(v: &mut Views, ui: &mut Ui, m: &Models) {
    match v.settings_page.level.parent() {
        Some(level) => go(v, ui, m, level),
        None => v.settings_page.show(ui, v.doc, false),
    }
}

/// Builds what the page shows again: after a plugin changed (its state, its connection).
pub fn refresh(v: &mut Views, ui: &mut Ui, m: &Models) {
    if v.settings_page.open {
        build(v, ui, m);
    }
}

/// Builds the level the page is on: the first is the sheet's (only its plugin count is
/// written); a deeper one is made into `#settings-level`.
fn build(v: &mut Views, ui: &mut Ui, m: &Models) {
    let (root, deep) = (v.doc.id("settings-root"), v.doc.id("settings-level"));
    let page = &mut v.settings_page;
    let switches = std::mem::take(&mut page.switches);
    page.was = if page.switches_for.as_ref() == Some(&page.level) {
        switches
            .into_iter()
            .map(|(name, on, parts)| (name, switch::Was { on, feel: switch::Feel::of(ui, parts) }))
            .collect()
    } else {
        Vec::new()
    };
    v.doc.clear(ui, deep);
    let plugins = &m.plugin.registry.list;
    let on = m.plugin.registry.on_count();
    let desc = ui.t("plugins.desc", &[("count", &plugins.len().to_string()), ("on", &on.to_string())]);
    v.doc.set(ui, "plugins-desc", desc);
    let effects = m.playback.effects.chain.list();
    let desc = match effects.len() {
        0 => ui.t("effects.desc.none", &[]),
        n => ui.t("effects.desc", &[("count", &n.to_string())]),
    };
    v.doc.set(ui, "effects-desc", desc);
    // A plugin deleted (or gone in a reload) while its page was up: the list. So for an effect.
    if let Level::Plugin(id) = &v.settings_page.level {
        if m.plugin.registry.get(id).is_none() {
            v.settings_page.level = Level::Plugins;
        }
    }
    if let Level::Effect(at) = v.settings_page.level {
        if at >= effects.len() {
            v.settings_page.level = Level::Effects;
        }
    }
    v.settings_page.sliders.clear();
    v.settings_page.dragging = None;
    v.settings_page.stale = false;
    let level = v.settings_page.level.clone();
    v.settings_page.switches_for = Some(level.clone());
    let mut trail = vec![(ui.t("settings.title", &[]), Level::Root)];
    match &level {
        Level::Root => {}
        Level::Plugins => {
            trail.push((ui.t("plugins.title", &[]), Level::Plugins));
            build_plugins(v, ui, m, deep);
        }
        Level::Effects => {
            trail.push((ui.t("effects.title", &[]), Level::Effects));
            build_effects(v, ui, m, deep);
        }
        Level::DesktopLyrics => trail.push((ui.t("desktop-lyrics.title", &[]), Level::DesktopLyrics)),
        Level::Plugin(id) => {
            trail.push((ui.t("plugins.title", &[]), Level::Plugins));
            let name = m.plugin.registry.name_of(id);
            trail.push((name, level.clone()));
            build_plugin(v, ui, m, deep, id);
        }
        Level::Effect(at) => {
            trail.push((ui.t("effects.title", &[]), Level::Effects));
            trail.push((effects[*at].plugin.name.clone(), level.clone()));
            build_effect(v, ui, m, deep, *at);
        }
    }
    let own = v.doc.id(level.container());
    for c in [root, deep, v.doc.id("settings-desktop-lyrics")] {
        ui.tree.set_layout(c, |l| l.display = if c == own { Display::Flex } else { Display::None });
    }
    let page = &mut v.settings_page;
    page.trail = trail;
    page.crumbs = None;
    page.place(ui, v.doc);
}

/// A card (icon, title over description); what it controls is added to it.
fn card(v: &mut Views, ui: &mut Ui, parent: Id, icon: Icon, title: &str, desc: &str, name: &str) -> Option<Id> {
    let args = [Val::from(icon), Val::Str(title.into()), Val::Str(desc.into())];
    v.doc.instantiate(ui, parent, None, "SettingsCard", &args, name).ok()
}

/// An item of a group's list (`SettingsItem`): where what it holds goes. Without a description
/// its title stands alone, centred.
fn item(v: &mut Views, ui: &mut Ui, group: Id, title: &str, desc: &str, name: &str) -> Option<Id> {
    let args = [Val::Str(title.into()), Val::Str(desc.into())];
    v.doc.instantiate(ui, group, None, "SettingsItem", &args, name).ok()?;
    Some(v.doc.id(&format!("{name}.card")))
}

/// A card's button.
fn button(v: &mut Views, ui: &mut Ui, card: Option<Id>, label: &str, command: Command, name: &str) {
    if let Some(card) = card {
        let label = ui.t(label, &[]);
        let _ =
            v.doc.instantiate(ui, card, None, "CardButton", &[Val::Str(label), Val::Str(command.to_string())], name);
    }
}

/// A toggle switch at the card's right: on or off, its words at its left. Built again in the
/// place of one that showed the other state, it flips with WinUI's motion.
/// What an effect's card says of it: why it was taken out, else its vendor and version.
fn effect_face(ui: &Ui, e: &crate::player_core::clap::EffectInfo) -> (Icon, String) {
    match &e.failure {
        Some(why) => (Icon::ErrorCircle, ui.t("effects.failed.desc", &[("why", why)])),
        None if e.plugin.vendor.is_empty() => (Icon::Speaker2, e.plugin.version.clone()),
        None => (Icon::Speaker2, format!("{}  ·  {}", e.plugin.vendor, e.plugin.version)),
    }
}

/// An effect's bypass switch: on while it processes.
fn bypass_toggle(
    v: &mut Views,
    ui: &mut Ui,
    c: Option<Id>,
    e: &crate::player_core::clap::EffectInfo,
    command: Command,
    name: &str,
) {
    let words = ui.t(if e.bypass { "effects.off" } else { "effects.on" }, &[]);
    toggle(v, ui, c, !e.bypass, &words, command, name);
}

fn toggle(v: &mut Views, ui: &mut Ui, card: Option<Id>, on: bool, words: &str, command: Command, name: &str) {
    let Some(card) = card else { return };
    let args = [Val::Str(command.to_string()), Val::Bool(on), Val::Str(words.into())];
    if v.doc.instantiate(ui, card, None, "Toggle", &args, name).is_err() {
        return;
    }
    let id = |part: &str| v.doc.id(&format!("{name}.{part}"));
    let parts = switch::Parts {
        root: id("row"),
        track: id("track"),
        knob: id("knob"),
        knob_off: id("off"),
        knob_on: id("on"),
        outline: id("outline"),
        fill: id("fill"),
    };
    let was = v.settings_page.was.iter().find(|(n, _)| n == name).map(|(_, w)| *w);
    switch::show(ui, parts, switch_skin(v.doc), on, was, ui.now());
    v.settings_page.switches.push((name.to_owned(), on, parts));
}

/// The switches' motion (`switch_*` in `player.nui`).
fn switch_skin(doc: &Doc) -> switch::Skin {
    let t = crate::views::layout::Scale::of(doc).text;
    let ease = |k: &str| crate::views::sheet_ease(doc, &format!("switch_{k}_"));
    // The knob's box is its pressed size; a size is its height's scale and its width's share.
    let (press, over) = (doc.num("switch_knob_press"), doc.num("switch_knob_over"));
    let size = |w: f32, h: f32| (h / over, w / (press * h / over));
    switch::Skin {
        travel: doc.num("switch_travel") * t,
        slide: doc.num("switch_slide") as f64,
        slide_ease: ease("slide"),
        fade: doc.num("switch_fade") as f64,
        resize: doc.num("switch_resize") as f64,
        resize_ease: ease("resize"),
        rest: size(doc.num("switch_knob"), doc.num("switch_knob")),
        over: size(over, over),
        pressed: size(press, over),
    }
}

/// The pointer's hold on a switch changed (`Event::Hover`, `Event::Pressed` on `id`): its knob
/// takes the size for it.
pub fn switch_felt(v: &Views, ui: &mut Ui, id: Id) {
    if let Some((_, _, parts)) = v.settings_page.switches.iter().find(|(_, _, p)| p.root == id || p.track == id) {
        switch::feel(ui, *parts, switch_skin(v.doc), ui.now());
    }
}

/// `设置 › 音效`: a card per effect in the chain, in order (what it is, or why it was taken out;
/// its window, moving it, bypass, removing it); then the effects found that can be added.
fn build_effects(v: &mut Views, ui: &mut Ui, m: &Models, deep: Id) {
    use crate::playback::effects::EffectVerb;
    let chain = m.playback.effects.chain.list();
    let effect = |verb: EffectVerb, at: usize| Command::Effect { verb, at };
    if chain.is_empty() {
        let (title, desc) = (ui.t("effects.empty", &[]), ui.t("effects.empty.desc", &[]));
        card(v, ui, deep, Icon::Speaker2, &title, &desc, "se-none");
    }
    for (k, e) in chain.iter().enumerate() {
        let (icon, desc) = effect_face(ui, e);
        let c = card(v, ui, deep, icon, &e.plugin.name, &desc, &format!("se{k}"));
        button(v, ui, c, "effects.remove", effect(EffectVerb::Remove, k), &format!("se{k}-remove"));
        if k + 1 < chain.len() {
            button(v, ui, c, "effects.down", effect(EffectVerb::Down, k), &format!("se{k}-down"));
        }
        if k > 0 {
            button(v, ui, c, "effects.up", effect(EffectVerb::Up, k), &format!("se{k}-up"));
        }
        // A taken-out effect offers nothing of its own: no window, parameters or bypass.
        if e.failure.is_some() {
            continue;
        }
        if e.has_window {
            button(v, ui, c, "effects.window", effect(EffectVerb::Window, k), &format!("se{k}-window"));
        }
        if e.has_params {
            let params = Command::Settings(Level::Effect(k));
            button(v, ui, c, "effects.params.btn", params, &format!("se{k}-params"));
        }
        bypass_toggle(v, ui, c, e, effect(EffectVerb::Bypass, k), &format!("se{k}-on"));
    }
    let heading = ui.t("effects.add", &[]);
    let _ = v.doc.instantiate(ui, deep, None, "SettingsHeading", &[Val::Str(heading)], "se-add");
    match &m.playback.effects.found {
        None => {
            let title = ui.t("effects.scanning", &[]);
            card(v, ui, deep, Icon::Speaker2, &title, "", "se-scanning");
        }
        Some(found) => {
            let (title, desc) = (ui.t("effects.scan", &[]), ui.t("effects.scan.desc", &[]));
            let c = card(v, ui, deep, Icon::Speaker2, &title, &desc, "se-scan");
            button(v, ui, c, "effects.scan.btn", Command::EffectsScan, "se-rescan");
            for (k, f) in found.iter().enumerate() {
                let c = card(v, ui, deep, Icon::Speaker2, &f.name, &f.vendor, &format!("sf{k}"));
                button(v, ui, c, "effects.add.btn", effect(EffectVerb::Add, k), &format!("sf{k}-add"));
            }
        }
    }
}

/// `设置 › 音效 › <name>`: the effect (what it is or why it was taken out; its window, bypass),
/// and under it, in one list (Windows' sound output list), a row per parameter as last read
/// (`EffectsModel::params`), each module's under its name:
/// its value as the plugin prints it, and a slider; a switch for a stepped one of two values, a
/// dropdown for one of a few; nothing to change for a read-only one.
fn build_effect(v: &mut Views, ui: &mut Ui, m: &Models, deep: Id, at: usize) {
    use crate::playback::effects::EffectVerb;
    let Some(e) = m.playback.effects.chain.list().into_iter().nth(at) else { return };
    let effect = |verb: EffectVerb| Command::Effect { verb, at };
    let (icon, desc) = effect_face(ui, &e);
    let args = [Val::from(icon), Val::Str(e.plugin.name.clone()), Val::Str(desc)];
    let Ok(group) = v.doc.instantiate(ui, deep, None, "SettingsGroup", &args, "ep-head") else { return };
    let c = Some(v.doc.id("ep-head.card"));
    if e.failure.is_some() {
        return;
    }
    if e.has_window {
        button(v, ui, c, "effects.window", effect(EffectVerb::Window), "ep-window");
    }
    bypass_toggle(v, ui, c, &e, effect(EffectVerb::Bypass), "ep-on");
    let params = match &m.playback.effects.params {
        Some((key, params)) if *key == e.key => params,
        _ => return,
    };
    if params.is_empty() {
        let title = ui.t("effects.params.none", &[]);
        item(v, ui, group, &title, "", "ep-none");
    }
    let mut module = "";
    let mut under_heading = false;
    for (k, p) in params.iter().enumerate() {
        if p.module != module {
            module = &p.module;
            let title = module.rsplit('/').next().unwrap_or(module).to_owned();
            let _ =
                v.doc.instantiate(ui, group, None, "SettingsSubheading", &[Val::Str(title)], &format!("ep{k}-module"));
            under_heading = true;
        }
        // A switch says its value itself.
        let switch = p.stepped && p.choices.len() == 2 && !p.read_only;
        let desc = if switch { "" } else { p.text.as_str() };
        let c = item(v, ui, group, &p.name, desc, &format!("ep{k}"));
        // A module's first row stays with its name: no seam between them.
        if std::mem::take(&mut under_heading) {
            ui.tree.set_layout(v.doc.id(&format!("ep{k}.rule")), |l| l.height = Len::Px(0.0));
        }
        let Some(c) = c.filter(|_| !p.read_only) else { continue };
        if switch {
            let on = p.value >= p.max - 0.5;
            let value = if on { p.min } else { p.max };
            let command = Command::Param { at, id: p.id, value };
            toggle(v, ui, Some(c), on, &p.choices[usize::from(on)], command, &format!("ep{k}-on"));
        } else if p.stepped && p.choices.len() > 2 {
            let action = Command::ParamDrop { at, id: p.id }.to_string();
            let name = format!("ep{k}-drop");
            if v.doc.instantiate(ui, c, None, "Dropdown", &[Val::Str(action)], &name).is_ok() {
                ui.tree.set_text(v.doc.id(&format!("{name}.value")), &p.text);
            }
        } else {
            let name = format!("ep{k}-slider");
            if v.doc.instantiate(ui, c, None, "ParamSlider", &[], &name).is_ok() {
                let bar = v.doc.id(&format!("{name}.bar"));
                ui.tree.set_semantics(bar, |a| a.label = Some(p.name.clone()));
                let s = Slider { name, bar, at, id: p.id, min: p.min, max: p.max, stepped: p.stepped };
                show_slider(v, ui, &s, p.value);
                v.settings_page.sliders.push(s);
            }
        }
    }
}

/// `ParamSlider`'s width and its pill's height (`player.nui`), in dips at text scale 1.
const SLIDER_W: f32 = 200.0;
const SLIDER_H: f32 = 6.0;

/// A slider's fill at `value`: up to it, a round end at least (the face's volume bar).
fn show_slider(v: &mut Views, ui: &mut Ui, s: &Slider, value: f64) {
    let span = s.max - s.min;
    // As a screen reader reads and sets it (`Event::Value`): a stepped one by one, else by a
    // twentieth of its span.
    let step = if s.stepped { 1.0 } else { span / 20.0 };
    ui.tree.set_semantics(s.bar, |a| a.range = Some(Range { value, min: s.min, max: s.max, step }));
    let frac = if span > 0.0 { ((value - s.min) / span).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let t = crate::views::layout::Scale::of(v.doc).text;
    ui.tree.set_layout(v.doc.id(&format!("{}.fill", s.name)), |l| {
        l.width = Len::Px(SLIDER_W * t * frac);
        l.min_width = Len::Px(if frac > 0.0 { SLIDER_H * t } else { 0.0 });
    });
}

/// The slider whose bar is `id`, if one is built.
fn slider(v: &Views, id: Id) -> Option<Slider> {
    v.settings_page.sliders.iter().find(|s| s.bar == id).cloned()
}

/// A press or drag on slider `id` at `x`: the value there (whole for a stepped one), shown at
/// once; what to set (`Command::Param`).
pub fn slide(v: &mut Views, ui: &mut Ui, id: Id, x: i32) -> Option<Command> {
    let s = slider(v, id)?;
    let frac = f64::from(crate::views::input::frac_in(ui, id, x));
    let mut value = s.min + frac * (s.max - s.min);
    if s.stepped {
        value = value.round();
    }
    v.settings_page.dragging = Some(id);
    show_slider(v, ui, &s, value);
    Some(Command::Param { at: s.at, id: s.id, value })
}

/// A slider set by the keyboard or a screen reader (`Event::Value`): the value (whole for a
/// stepped one) shown, and what to set.
pub fn set_slider(v: &mut Views, ui: &mut Ui, id: Id, value: f64) -> Option<Command> {
    let s = slider(v, id)?;
    let mut value = value.clamp(s.min.min(s.max), s.max.max(s.min));
    if s.stepped {
        value = value.round();
    }
    show_slider(v, ui, &s, value);
    Some(Command::Param { at: s.at, id: s.id, value })
}

/// The slider let go: the level is built again if what changed meanwhile needs it.
pub fn slid(v: &mut Views, ui: &mut Ui, m: &Models) {
    if v.settings_page.dragging.take().is_some() && std::mem::take(&mut v.settings_page.stale) {
        build(v, ui, m);
    }
}

/// The parameters of the shown effect were read (or one was set here): its level shows them. The
/// same parameters as built: their texts and sliders follow in place (a slider being dragged
/// keeps its thumb under the pointer); otherwise (others, a switch that turned) the level is
/// built again, once a drag is over.
pub fn params_read(v: &mut Views, ui: &mut Ui, m: &Models) {
    if !v.settings_page.open || !matches!(v.settings_page.level, Level::Effect(_)) {
        return;
    }
    let shown = m.playback.effects.params.as_ref().map(|(_, params)| params);
    let text = |ui: &Ui, name: String| v.doc.get(&name).and_then(|e| ui.tree.text_of(e)).map(str::to_owned);
    let built = (0..).take_while(|k| v.doc.get(&format!("ep{k}.card")).is_some()).count();
    let same = shown.is_some_and(|params| {
        // Built with the parameters known: their rows, or the row saying there are none.
        (built > 0 || v.doc.get("ep-none.card").is_some())
            && built == params.len()
            && params.iter().enumerate().all(|(k, p)| {
                let switch = text(ui, format!("ep{k}-on.state"));
                text(ui, format!("ep{k}.title")).as_deref() == Some(p.name.as_str())
                    && switch.is_none_or(|words| words == p.text)
            })
    });
    if !same {
        if v.settings_page.dragging.is_some() {
            v.settings_page.stale = true;
        } else {
            build(v, ui, m);
        }
        return;
    }
    let Some(params) = shown else { return };
    for (k, p) in params.iter().enumerate() {
        if v.doc.get(&format!("ep{k}-on.state")).is_none() {
            ui.tree.set_text(v.doc.id(&format!("ep{k}.desc")), &p.text);
        }
        if let Some(e) = v.doc.get(&format!("ep{k}-drop.value")) {
            ui.tree.set_text(e, &p.text);
        }
        let bar = v.doc.get(&format!("ep{k}-slider.bar"));
        if let Some(s) = bar.filter(|bar| v.settings_page.dragging != Some(*bar)).and_then(|bar| slider(v, bar)) {
            show_slider(v, ui, &s, p.value);
        }
    }
}

/// `设置 › 插件`: a card per plugin, opening its page; then installing one.
fn build_plugins(v: &mut Views, ui: &mut Ui, m: &Models, deep: Id) {
    let list = m.plugin.registry.list.clone();
    if list.is_empty() {
        let none = ui.t("plugins.none", &[]);
        card(v, ui, deep, Icon::Grid, &none, "", "sp-none");
    }
    for (k, pl) in list.iter().enumerate() {
        let pv = PluginView::of(ui, m.plugin, pl, None);
        let icon = if pv.broken.is_some() { Icon::ErrorCircle } else { Icon::Grid };
        let desc = match &pv.conn {
            Some(Conn::Who(who)) => format!("{}  ·  {}", pv.note, ui.t("plugin.conn.who", &[("who", who)])),
            _ => pv.note.clone(),
        };
        let args = [
            Val::from(icon),
            Val::Str(pv.name.clone()),
            Val::Str(desc),
            Val::Str(Command::Settings(Level::Plugin(pv.id.clone())).to_string()),
        ];
        let _ = v.doc.instantiate(ui, deep, None, "SettingsNav", &args, &format!("sp{k}"));
    }
    let (title, desc) = (ui.t("plugins.install", &[]), ui.t("plugins.install.desc", &[]));
    let c = card(v, ui, deep, Icon::ArrowImport, &title, &desc, "sp-install");
    button(v, ui, c, "plugins.folder.btn", Command::PluginFolder, "sp-folder");
    button(v, ui, c, "plugins.install.btn", Command::PluginInstall, "sp-add");
}

/// `设置 › 插件 › <name>`: what the app menu's panel shows (`PluginView`), as cards: name,
/// version and the switch (what it asks for while being enabled); the connection; its
/// playlists to import; a control that could not be read; deleting it.
fn build_plugin(v: &mut Views, ui: &mut Ui, m: &Models, deep: Id, id: &str) {
    let Some(pl) = m.plugin.registry.get(id).cloned() else { return };
    let pv = PluginView::of(ui, m.plugin, &pl, m.pending);
    let mut desc = Vec::new();
    if !pv.version.is_empty() {
        desc.push(ui.t("plugin.version", &[("version", &pv.version)]));
    }
    desc.extend(pv.broken.iter().cloned());
    desc.extend(pv.asks.iter().cloned());
    let icon = if pv.broken.is_some() { Icon::ErrorCircle } else { Icon::Grid };
    let head = card(v, ui, deep, icon, &pv.name, &desc.join("\n"), "pg-head");
    if pv.broken.is_none() {
        let words = if pv.pending { ui.t("plugin.toggle.confirm", &[]) } else { pv.note.clone() };
        toggle(v, ui, head, pv.on, &words, Command::PluginSwitch(id.to_owned()), "pg-on");
    }

    // The connection.
    if let Some(conn) = &pv.conn {
        let desc = match conn {
            Conn::Who(a) => ui.t("plugin.conn.who", &[("who", a)]),
            Conn::Busy { line, .. } => line.clone(),
            Conn::Form(_) => ui.t("plugin.conn.none", &[]),
        };
        let title = ui.t("plugin.conn", &[]);
        let c = card(v, ui, deep, Icon::Person, &title, &desc, "pg-conn");
        let words = v.doc.id("pg-conn.words");
        match conn {
            Conn::Who(_) => button(v, ui, c, "plugin.conn.out", Command::Disconnect(id.to_owned()), "pg-out"),
            Conn::Busy { qr, .. } => {
                if let Some(q) = qr {
                    crate::views::menus::plugin_flyout::qr_code(v.doc, ui, words, q, "start", "pg-qr");
                }
                button(v, ui, c, "plugin.conn.cancel", Command::ConnectCancel(id.to_owned()), "pg-cancel");
            }
            Conn::Form(fields) => {
                for (k, (label, kind)) in fields.iter().enumerate() {
                    let kept = m.plugin.form.kept(id, k);
                    crate::views::menus::plugin_flyout::field(
                        v.doc,
                        ui,
                        words,
                        &format!("pg-f{k}"),
                        k,
                        label,
                        *kind,
                        kept,
                    );
                }
                button(v, ui, c, "plugin.conn.go", Command::Connect(id.to_owned()), "pg-go");
            }
        }
    }
    if pv.lists {
        let (title, desc) = (ui.t("plugin.lists", &[]), ui.t("plugin.lists.desc", &[]));
        let c = card(v, ui, deep, Icon::ArrowImport, &title, &desc, "pg-lists");
        button(v, ui, c, "plugin.lists.btn", Command::PickLists(id.to_owned()), "pg-lists-btn");
    }
    if let Some(fault) = &pv.fault {
        let title = ui.t("plugin.fault", &[]);
        card(v, ui, deep, Icon::ErrorCircle, &title, fault, "pg-fault");
    }
    let (title, desc) = (ui.t("plugin.delete", &[]), ui.t("plugin.delete.desc", &[]));
    let c = card(v, ui, deep, Icon::Delete, &title, &desc, "pg-del");
    button(v, ui, c, "plugin.delete.btn", Command::PluginDelete(id.to_owned()), "pg-del-btn");
}

/// The language ids a font's name is shown in, first found first (the UI is zh-CN for now).
const NAME_LANGS: [u16; 3] = [0x0804, 0x0404, 0x0409];

/// A choice kept in settings that the page offers as a dropdown: its `settings.txt` key, its
/// `Dropdown` or `ComboBox` (`#<key>-drop`, `Command::Drop`) and its list's rows
/// (`Command::Set`). Another one is a row here (and its box in the sheet); the mechanics are
/// `menus::dropdown`'s.
pub struct Setting {
    pub key: &'static str,
    /// Its choices, the default first.
    pub choices: fn(&Ui) -> Vec<Choice>,
    /// Puts a value (`None`: never set) into effect. False: it cannot be (an uninstalled font),
    /// the default is.
    pub apply: fn(&mut Ui, &mut Views, Option<&str>) -> bool,
}

pub const SETTINGS: &[Setting] = &[
    Setting { key: "locale", choices: locale_choices, apply: apply_locale },
    Setting { key: "font", choices: font_choices, apply: apply_font },
    Setting { key: "layout", choices: layout_choices, apply: apply_layout },
    Setting { key: "lyric-style", choices: lyric_style_choices, apply: apply_lyric_style },
    Setting { key: "replay-gain", choices: replay_gain_choices, apply: apply_replay_gain },
    Setting { key: "crossfade", choices: crossfade_choices, apply: apply_crossfade },
    Setting { key: "keyboard-navigation", choices: keyboard_navigation_choices, apply: apply_keyboard_navigation },
    Setting { key: "screen-reader", choices: screen_reader_choices, apply: apply_screen_reader },
    Setting { key: dl::SHOWN, choices: desktop_lyrics_choices, apply: |_, _, v| v == Some("on") },
    Setting { key: dl::LOCKED, choices: desktop_lyrics_lock_choices, apply: |_, _, v| v == Some("on") },
    Setting { key: dl::SIZE, choices: desktop_lyrics_size_choices, apply: |ui, _, v| offered(ui, dl::SIZE, v) },
    Setting { key: dl::COLOUR, choices: desktop_lyrics_colour_choices, apply: |ui, _, v| offered(ui, dl::COLOUR, v) },
    Setting { key: dl::PLATE, choices: desktop_lyrics_plate_choices, apply: |ui, _, v| offered(ui, dl::PLATE, v) },
];

use crate::views::desktop_lyrics as dl;

/// Off (the default), on: what a choice of two says.
fn off_on(ui: &Ui, prefix: &str) -> Vec<Choice> {
    ["off", "on"]
        .into_iter()
        .map(|v| {
            let label = ui.t(&format!("{prefix}.{v}"), &[]);
            Choice { names: vec![label.clone()], label, value: v.to_owned() }
        })
        .collect()
}

/// Off (the default), on: the desktop lyrics are up (`views::desktop_lyrics`, every present).
fn desktop_lyrics_choices(ui: &Ui) -> Vec<Choice> {
    off_on(ui, "desktop-lyrics")
}

/// Off (the default), on: presses go through them.
fn desktop_lyrics_lock_choices(ui: &Ui) -> Vec<Choice> {
    off_on(ui, "desktop-lyrics.lock")
}

/// A choice named by its value: the default first, named so.
fn named(ui: &Ui, values: impl IntoIterator<Item = String>, name: impl Fn(&str) -> String) -> Vec<Choice> {
    values
        .into_iter()
        .enumerate()
        .map(|(k, value)| {
            let label = if k == 0 { ui.t("desktop-lyrics.default", &[("v", &name(&value))]) } else { name(&value) };
            Choice { names: vec![label.clone(), value.clone()], label, value }
        })
        .collect()
}

fn desktop_lyrics_size_choices(ui: &Ui) -> Vec<Choice> {
    named(ui, dl::SIZES.map(|s| s.to_string()), str::to_owned)
}

fn desktop_lyrics_colour_choices(ui: &Ui) -> Vec<Choice> {
    named(ui, dl::COLOURS.map(|(n, _)| n.to_owned()), |n| ui.t(&format!("colour.{n}"), &[]))
}

fn desktop_lyrics_plate_choices(ui: &Ui) -> Vec<Choice> {
    named(ui, dl::PLATES.map(|p| p.to_string()), |p| format!("{p}%"))
}

/// Nothing now: the desktop lyrics read the setting `key` at their next present. True for a
/// value its choices offer other than the default.
fn offered(ui: &Ui, key: &str, value: Option<&str>) -> bool {
    setting(key).is_some_and(|s| (s.choices)(ui).iter().skip(1).any(|c| Some(c.value.as_str()) == value))
}

/// The setting kept under `key`.
pub fn setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// The languages the app speaks (a table each in `ui/`), each named in itself.
pub const LOCALES: [(&str, &str); 2] = [("zh-CN", "\u{7b80}\u{4f53}\u{4e2d}\u{6587}"), ("en", "English")];

/// The system's language: zh-CN for a Chinese display language, else en. Tests speak zh-CN
/// whatever the machine's language (their texts and shots are zh-CN's).
fn system_locale() -> &'static str {
    if cfg!(test) || novaui::platform::locale().starts_with("zh") {
        "zh-CN"
    } else {
        "en"
    }
}

/// Following the system first, then each language.
fn locale_choices(ui: &Ui) -> Vec<Choice> {
    let system = ui.t("locale.system", &[]);
    let mut rows = vec![Choice { names: vec![system.clone()], label: system, value: String::new() }];
    rows.extend(LOCALES.map(|(tag, name)| Choice {
        label: name.to_owned(),
        value: tag.to_owned(),
        names: vec![name.to_owned(), tag.to_owned()],
    }));
    rows
}

/// Every text speaks the language (`value`, else the system's) from the next presentation on.
/// False: no language of the app was kept (the system's is spoken).
pub fn set_locale(ui: &Ui, value: Option<&str>) -> bool {
    let tag = LOCALES.iter().map(|(t, _)| *t).find(|t| Some(*t) == value);
    ui.strings().set_locale(tag.unwrap_or_else(system_locale));
    tag.is_some()
}

fn apply_locale(ui: &mut Ui, _: &mut Views, value: Option<&str>) -> bool {
    set_locale(ui, value)
}

/// The default, bundled Noto Sans, then every installed family, by every name it has.
fn font_choices(ui: &Ui) -> Vec<Choice> {
    let label = ui.t("font.default", &[]);
    let mut rows = vec![Choice { names: vec![label.clone()], label, value: String::new() }];
    for f in system_fonts() {
        let mut names: Vec<String> = f.names.iter().map(|(_, n)| n.clone()).collect();
        names.push(f.family.clone());
        rows.push(Choice { label: f.name(&NAME_LANGS).to_owned(), value: f.family, names });
    }
    rows
}

/// The font goes ahead of every text; an unknown family (uninstalled since) is the default.
fn apply_font(ui: &mut Ui, _: &mut Views, value: Option<&str>) -> bool {
    let family = value.filter(|f| !f.is_empty());
    if family.is_some_and(|f| ui.tree.set_font(Some(f))) {
        return true;
    }
    ui.tree.set_font(None);
    false
}

/// Automatic: two columns once the window is wide enough; single: one at every width.
fn layout_choices(ui: &Ui) -> Vec<Choice> {
    ["auto", "single"]
        .into_iter()
        .map(|v| {
            let label = ui.t(&format!("layout.{v}"), &[]);
            Choice { names: vec![label.clone()], label, value: v.to_owned() }
        })
        .collect()
}

fn apply_layout(ui: &mut Ui, v: &mut Views, value: Option<&str>) -> bool {
    v.doc.set(ui, "single-column", value == Some("single"));
    true
}

/// Off (the default), then by the song's value, then by its album's (`replay_gain_of`).
fn replay_gain_choices(ui: &Ui) -> Vec<Choice> {
    ["off", "track", "album"]
        .into_iter()
        .map(|v| {
            let label = ui.t(&format!("replay-gain.{v}"), &[]);
            Choice { names: vec![label.clone()], label, value: v.to_owned() }
        })
        .collect()
}

/// Nothing on the page: the engine plays at it (`Command::Set`, and the kept one at start).
fn apply_replay_gain(_: &mut Ui, _: &mut Views, value: Option<&str>) -> bool {
    replay_gain_of(value) != crate::player_core::ReplayGain::Off
}

/// The ReplayGain mode a kept `replay-gain` value names; anything else is off.
pub fn replay_gain_of(value: Option<&str>) -> crate::player_core::ReplayGain {
    use crate::player_core::ReplayGain;
    match value {
        Some("track") => ReplayGain::Track,
        Some("album") => ReplayGain::Album,
        _ => ReplayGain::Off,
    }
}

/// Off (the default), on: Tab moves the keyboard focus through the controls.
fn keyboard_navigation_choices(ui: &Ui) -> Vec<Choice> {
    ["off", "on"]
        .into_iter()
        .map(|v| {
            let label = ui.t(&format!("keyboard-navigation.{v}"), &[]);
            Choice { names: vec![label.clone()], label, value: v.to_owned() }
        })
        .collect()
}

/// At once: on, the keyboard moves and uses the focus; off (anything but `on`), Tab, Enter, Space
/// and the arrows are the app's keys (`command::KEYS`).
fn apply_keyboard_navigation(ui: &mut Ui, _: &mut Views, value: Option<&str>) -> bool {
    let on = value == Some("on");
    ui.set_keyboard_navigation(on);
    on
}

/// Off (the default), on: screen readers reach the window from the next start.
fn screen_reader_choices(ui: &Ui) -> Vec<Choice> {
    ["off", "on"]
        .into_iter()
        .map(|v| {
            let label = ui.t(&format!("screen-reader.{v}"), &[]);
            Choice { names: vec![label.clone()], label, value: v.to_owned() }
        })
        .collect()
}

/// Nothing now: the window is made with it at the next start (`screen_reader_of`).
fn apply_screen_reader(_: &mut Ui, _: &mut Views, value: Option<&str>) -> bool {
    screen_reader_of(value)
}

/// Whether a kept `screen-reader` value turns screen readers on; anything but `on` is off.
pub fn screen_reader_of(value: Option<&str>) -> bool {
    value == Some("on")
}

/// The longest crossfade offered, in seconds.
const CROSSFADE_MAX: u64 = 12;

/// Off (the default), then 1 to `CROSSFADE_MAX` seconds.
fn crossfade_choices(ui: &Ui) -> Vec<Choice> {
    let off = ui.t("crossfade.off", &[]);
    let mut rows = vec![Choice { names: vec![off.clone()], label: off, value: "off".to_owned() }];
    rows.extend((1..=CROSSFADE_MAX).map(|n| {
        let label = ui.t("crossfade.secs", &[("n", &n.to_string())]);
        Choice { names: vec![label.clone()], label, value: n.to_string() }
    }));
    rows
}

/// Nothing on the page: the songs handed over from now on overlap by it (`Player::hand_next`).
fn apply_crossfade(_: &mut Ui, _: &mut Views, value: Option<&str>) -> bool {
    !crossfade_of(value).is_zero()
}

/// The overlap a kept `crossfade` value names: whole seconds from 1 to `CROSSFADE_MAX`;
/// anything else is off.
pub fn crossfade_of(value: Option<&str>) -> std::time::Duration {
    let secs = value.and_then(|v| v.parse::<u64>().ok()).filter(|s| (1..=CROSSFADE_MAX).contains(s));
    std::time::Duration::from_secs(secs.unwrap_or(0))
}

/// The built-in style, then every installed one (`app::lyric_styles`), by name.
fn lyric_style_choices(ui: &Ui) -> Vec<Choice> {
    let label = ui.t("lyric-style.builtin", &[]);
    let mut rows = vec![Choice { names: vec![label.clone()], label, value: String::new() }];
    let locale = ui.strings().locale();
    let installed = crate::app::lyric_styles::root().map(|r| crate::app::lyric_styles::installed(&r, &locale));
    rows.extend(installed.into_iter().flatten().map(|s| Choice {
        names: vec![s.name.clone(), s.id.clone()],
        label: s.name,
        value: s.id,
    }));
    rows
}

/// The lines are drawn with the installed style `value`; false (none, or it is gone or no longer
/// reads): with the built-in one.
fn apply_lyric_style(ui: &mut Ui, v: &mut Views, value: Option<&str>) -> bool {
    let locale = ui.strings().locale();
    let root = crate::app::lyric_styles::root();
    let src = value
        .filter(|id| !id.is_empty())
        .and_then(|id| crate::app::lyric_styles::find(root.as_deref()?, id, &locale))
        .and_then(|s| s.source().ok());
    let chosen = src.is_some();
    let ok = v.side_panel.set_lyric_style(src);
    v.doc.set(ui, "lyric-style-installed", chosen && ok);
    chosen && ok
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sheet's `Toggle` is the switch `switch::Parts` names: its knob's centre 10 dips in
    /// from the end it shows, its knob sizes WinUI's (12 at rest, 14 over, 17 × 14 pressed).
    #[test]
    fn the_sheets_switch_is_made_of_the_parts_its_motion_moves() {
        let mut ui = Ui::new(400, 200, 1.0);
        let mut doc = Doc::new(novaui::dsl::parse(include_str!("../../ui/player.nui")).unwrap());
        let root = ui.tree.root();
        doc.build(&mut ui, root).unwrap();
        let skin = switch_skin(&doc);
        let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4;
        assert!(near(skin.rest, (12.0 / 14.0, 14.0 / 17.0)), "{:?}", skin.rest);
        assert!(near(skin.over, (1.0, 14.0 / 17.0)) && near(skin.pressed, (1.0, 1.0)));
        assert_eq!(skin.travel, 20.0 * crate::views::layout::Scale::of(&doc).text);
        for (name, on) in [("t-off", false), ("t-on", true)] {
            let args = [Val::Str("noop".into()), Val::Bool(on), Val::Str("w".into())];
            doc.instantiate(&mut ui, root, None, "Toggle", &args, name).unwrap();
        }
        novaui::backend::frame(&mut ui.tree, &mut novaui::backend::Software::new(400, 200), 0.0);
        for (name, end) in [("t-off", 10.0), ("t-on", 30.0)] {
            let id = |part: &str| doc.id(&format!("{name}.{part}"));
            let (track, knob) = (ui.tree.rect(id("track")), ui.tree.rect(id("knob")));
            let t = crate::views::layout::Scale::of(&doc).text;
            let centre = (knob.x + knob.w / 2 - track.x) as f32;
            assert!((centre - end * t).abs() <= 1.0, "{name}: knob centre {centre}, track {track:?}");
            for part in ["row", "outline", "fill", "off", "on"] {
                assert!(doc.get(&format!("{name}.{part}")).is_some(), "{name}.{part}");
            }
        }
    }
}
