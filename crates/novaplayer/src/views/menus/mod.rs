//! Flyouts: which one is up and its rows. How a flyout opens, stands and scrolls is novaui's
//! `widgets::Flyout`.
pub mod app_menu;
pub mod cover;
pub mod drag;
pub mod dropdown;
pub mod option;
pub mod output;
pub mod playlist;
pub mod plugin_flyout;
pub mod row;
pub mod sort;

pub(crate) use crate::app::command::Command;
use crate::app::{Models, Views};
use novaui::dsl::Val;
pub(crate) use novaui::widgets::flyout::{Along, Place, Side};
use novaui::widgets::Flyout;
use novaui::*;

/// Which flyout is up in `#flyout` (one at a time; the plugin flyout stands beside the app one).
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Playlist,
    App,
    Drag,
    Row,
    Sort,
    Cover,
    /// The song's option values, for the song opened `song`th (`Playback::song_version`).
    Option {
        song: u64,
    },
    Output,
    /// A settings dropdown's list.
    Drop(DropList),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropList {
    /// Whose list it is: the setting it chooses, or the box of an effect's parameter.
    pub key: String,
    /// What each row does when picked (a setting's choice, an effect parameter's value), and the
    /// row the keys are on.
    pub picks: Vec<Command>,
    pub lit: Option<usize>,
}

impl Kind {
    /// Its width, a sheet `let`; `None`: as wide as its widest row, between the sheet's
    /// `flyout_w_min` (the output flyout's own) and `flyout_w_max`.
    fn width(&self) -> Option<&'static str> {
        match self {
            Kind::Playlist | Kind::App => Some("flyout_w"),
            Kind::Drag => Some("flyout_w_drag"),
            Kind::Row | Kind::Sort => Some("flyout_w_narrow"),
            Kind::Cover => Some("flyout_w_cover"),
            Kind::Option { .. } | Kind::Output | Kind::Drop(_) => None,
        }
    }
}

pub(crate) fn is_open(v: &Views, f: impl Fn(&Kind) -> bool) -> bool {
    v.menus.open.as_ref().is_some_and(f)
}

/// Every flyout goes.
pub fn close_menus(v: &mut Views, ui: &mut Ui) {
    let host = v.id("flyout");
    if let Some(t) = v.menus.main.close(ui, host) {
        v.menus.clock.at(t);
    }
    v.menus.open = None;
    close_plugin_flyout(v, ui);
}

/// Only the side flyout goes — a plugin's, or the row menu's playlist picker (Esc, or its row
/// pressed again); the one it stands beside stays.
pub fn close_plugin_flyout(v: &mut Views, ui: &mut Ui) {
    v.menus.plugin = None;
    v.menus.pick = None;
    let host = v.id("plugin-flyout");
    if let Some(t) = v.menus.side.close(ui, host) {
        v.menus.clock.at(t);
    }
}

/// A flyout of `kind` starts in `#flyout`: another one up goes (the same kind is rebuilt in
/// place), its rows go, its width is the sheet's. Returns the host to fill.
pub(crate) fn begin(v: &mut Views, ui: &mut Ui, kind: Kind) -> Id {
    let same = v.menus.open.as_ref().is_some_and(|k| std::mem::discriminant(k) == std::mem::discriminant(&kind));
    if !same {
        close_menus(v, ui);
    }
    let host = v.id("flyout");
    v.menus.main.reset(ui, host);
    v.doc.prune(ui);
    let (w, min, max) = match kind.width() {
        Some(n) => (Len::Px(v.doc.num(n)), Len::Auto, Len::Auto),
        None => {
            let min = if kind == Kind::Output { "flyout_w_min_output" } else { "flyout_w_min" };
            (Len::Auto, Len::Px(v.doc.num(min)), Len::Px(v.doc.num("flyout_w_max")))
        }
    };
    ui.tree.set_layout(host, |l| {
        l.width = w;
        l.min_width = min;
        l.max_width = max;
    });
    v.menus.open = Some(kind);
    host
}

/// Shows `#flyout` with the rows `begin` was given, placed at `place` once laid out. `anchor`:
/// the button that opened it (pressed again, it only closes it).
pub(crate) fn show(v: &mut Views, ui: &mut Ui, place: Place, anchor: Option<Id>) {
    let host = v.id("flyout");
    v.menus.main.top = ui.tree.rect(v.id("titlebar")).b() as f32 / ui.scale();
    v.menus.main.open(ui, host, place, anchor);
}

/// The side flyout (`#plugin-flyout`), filled, beside `row` of the main one.
pub(crate) fn show_side(v: &mut Views, ui: &mut Ui, row: Id) {
    let host = v.id("plugin-flyout");
    v.menus.side.top = ui.tree.rect(v.id("titlebar")).b() as f32 / ui.scale();
    let place = Place::on(row, Side::Right, Along::Start, v.doc.num("flyout_side_gap"));
    v.menus.side.open(ui, host, place, None);
}

/// After layout: flyouts opened this turn take their place; every flyout's rail follows it.
pub(crate) fn after_layout(v: &mut Views, ui: &mut Ui) {
    let (main, side) = (v.id("flyout"), v.id("plugin-flyout"));
    let placed = v.menus.main.place(ui, main);
    if let Some((row, center)) = v.menus.reveal.take().filter(|&(r, _)| ui.tree.alive(r)) {
        if placed {
            ui.tree.resolve_layout();
        }
        v.menus.main.reveal(ui, main, row, center);
    }
    // The plugin flyout stands against a row of the app one: that row where it was just placed.
    if v.menus.side.pending() {
        ui.tree.resolve_layout();
        v.menus.side.place(ui, side);
    }
    let skin = crate::views::rail_skin(v.doc);
    for (n, host) in [("flyout", main), ("plugin-flyout", side)] {
        let parts = rail_parts(v, n);
        let fly = if host == main { &mut v.menus.main } else { &mut v.menus.side };
        let wake = fly.sync_rail(ui, host, parts, skin);
        crate::views::rail_wake(&v.menus.clock, ui, wake);
    }
}

fn rail_parts(v: &Views, n: &str) -> novaui::widgets::scroll::Parts {
    novaui::widgets::scroll::Parts {
        root: v.id(&format!("{n}-rail")),
        track: v.id(&format!("{n}-rail-track")),
        thumb: v.id(&format!("{n}-rail-thumb")),
    }
}

/// A press, drag or wheel on a flyout's rail, or a wheel over a flyout.
pub(crate) fn flyout_event(v: &mut Views, ui: &mut Ui, ev: &Event) -> bool {
    let (main, side) = (v.id("flyout"), v.id("plugin-flyout"));
    if let Event::Wheel { id, dy } = *ev {
        if id == main || id == side {
            let fly = if id == main { &mut v.menus.main } else { &mut v.menus.side };
            fly.wheel(ui, id, dy);
            return true;
        }
    }
    let (pm, ps) = (rail_parts(v, "flyout"), rail_parts(v, "plugin-flyout"));
    let hit = v.menus.main.rail_event(ui, main, pm, ev) || v.menus.side.rail_event(ui, side, ps, ev);
    if hit {
        v.menus.clock.frame();
    }
    hit
}

#[allow(clippy::too_many_arguments)] // Flat menu-row description; a struct would only rename the arguments.
pub(crate) fn item(
    v: &mut Views,
    ui: &mut Ui,
    menu: Id,
    icon: Icon,
    label: &str,
    note: &str,
    command: Option<Command>,
    enabled: bool,
    prefix: &str,
) -> Option<Id> {
    let id = v.doc.instantiate(ui, menu, None, "MenuItem", &[Val::from(icon), Val::Str(label.into())], prefix).ok()?;
    ui.tree.set_action(id, command.map(|c| c.to_string()).as_deref());
    let note_id = v.doc.id(&format!("{prefix}.note"));
    ui.tree.set_text(note_id, note);
    ui.tree.set_disabled(id, !enabled);
    Some(id)
}

/// A choice row: the checkmark only on the chosen one (the others keep its column).
#[allow(clippy::too_many_arguments)] // Flat menu-row description; a struct would only rename the arguments.
pub(crate) fn check_item(
    v: &mut Views,
    ui: &mut Ui,
    menu: Id,
    on: bool,
    label: &str,
    note: &str,
    command: Option<Command>,
    enabled: bool,
    prefix: &str,
) -> Option<Id> {
    let id = item(v, ui, menu, Icon::Checkmark, label, note, command, enabled, prefix)?;
    ui.tree.set_off(v.doc.id(&format!("{prefix}.icon")), !on);
    Some(id)
}

/// The option flyout lists the values of the song it was opened for: once another song opened,
/// it goes (the flyouts follow the model; nothing asks them to close).
pub fn close_stale(v: &mut Views, ui: &mut Ui, m: &Models) {
    if is_open(v, |k| matches!(k, Kind::Option { song } if *song != m.playback.song_version)) {
        close_menus(v, ui);
    }
}

/// `Player` state owned by this area.
pub struct Menus {
    /// Its frames and deadlines (`novaui::Clock`).
    pub clock: novaui::Clock,
    /// `#flyout` and `#plugin-flyout`.
    pub main: Flyout,
    pub side: Flyout,
    /// What `#flyout` holds while it is up.
    pub open: Option<Kind>,
    /// A row of it to scroll into view once it is placed (centred: true).
    pub reveal: Option<(Id, bool)>,
    /// The plugin whose flyout is up (beside the app one).
    pub plugin: Option<String>,
    /// The row menu's playlist picker up beside it: the row's position, and whether the song
    /// moves (移动到) rather than is copied (添加到).
    pub pick: Option<(usize, bool)>,
    /// A carried row's drop target on the heading (8.5).
    pub drag: crate::views::menus::drag::RowDrag,
    /// The playlist being renamed in the playlist flyout (its name box is up).
    pub rename_id: Option<u32>,
}

impl Menus {
    /// Its clock delivers `Msg::Menus`.
    pub fn new(ui: &mut novaui::Ui) -> Self {
        Menus {
            clock: ui.clock(crate::app::Msg::Menus),
            main: Default::default(),
            side: Default::default(),
            open: Default::default(),
            reveal: Default::default(),
            plugin: Default::default(),
            pick: Default::default(),
            drag: Default::default(),
            rename_id: Default::default(),
        }
    }
}

/// A kept playlist's note in a menu: how many songs it holds.
pub(crate) fn count_note(ui: &novaui::Ui, e: &crate::library::store::Entry) -> String {
    ui.t("songs.n", &[("n", &e.count.to_string())])
}
