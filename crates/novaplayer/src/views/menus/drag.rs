//! Dragging a queue row onto a playlist.
use crate::app::Player;
use crate::views::menus::*;
use novaui::*;

/// A carried row's drop target (8.5, b7b33f5 playlist_menu.rs): the row carried onto the
/// heading's name brings up the playlists, the open one standing aside, the one under the
/// pointer lit and saying 加入. It comes and goes as the way in beside the artwork does: up at
/// once over 167 ms, held while the row is over the name or the playlists, gone 2 s after the
/// last move on the name or as soon as the row leaves both, out over 333 ms.
#[derive(Default)]
pub struct RowDrag {
    pub shown: bool,
    /// The target came up during this drag.
    pub came: bool,
    /// The playlist under the pointer.
    pub over: Option<u32>,
    hide_at: f64,
    last: (i32, i32),
}

pub(crate) const DRAG_IDLE: f64 = 2.0;

pub(crate) fn drag_over(p: &mut Player, ui: &mut Ui, x: i32, y: i32) {
    let name = ui.tree.rect(p.id("pl-name"));
    let on_name =
        name.w > 0 && x >= name.x - 8 && x < name.x + name.w + 8 && y >= name.y - 4 && y < name.y + name.h + 4;
    let menu = p.id("flyout");
    let mr = ui.tree.rect(menu);
    let on_menu = p.menus.drag.shown && mr.w > 0 && x >= mr.x && x < mr.x + mr.w && y >= mr.y && y < mr.y + mr.h;
    let moved = (x, y) != p.menus.drag.last;
    p.menus.drag.last = (x, y);
    if on_name && !p.menus.drag.shown {
        open_drag_menu(p, ui);
    }
    if on_name && moved {
        p.menus.drag.hide_at = ui.now() + DRAG_IDLE;
        p.menus.clock.at(p.menus.drag.hide_at);
    }
    if on_menu {
        p.menus.drag.hide_at = f64::INFINITY;
    } else if !on_name && p.menus.drag.shown {
        p.menus.drag.hide_at = ui.now();
    }
    // The playlist under the pointer, lit.
    let mut over = None;
    if on_menu {
        if let Some(t) = ui.tree.hit(x, y, |e| e.action().and_then(drag_target).is_some()) {
            over = ui.tree.el(t).action().and_then(drag_target);
        }
    }
    if over != p.menus.drag.over {
        for (k, e) in p.lists.store.entries.clone().iter().enumerate() {
            if let Some(it) = p.doc.get(&format!("c-e{k}.item")) {
                let lit = over == Some(e.id);
                ui.tree.set_lit(it, lit);
                if let Some(n) = p.doc.get(&format!("c-e{k}.note")) {
                    ui.tree.set_text(n, &if lit { ui.t("drag.add", &[]) } else { super::count_note(ui, e) });
                }
            }
        }
        p.menus.drag.over = over;
    }
    drag_tick(p, ui);
}

pub(crate) fn open_drag_menu(p: &mut Player, ui: &mut Ui) {
    p.views(|v, m| {
        let menu = begin(v, ui, Kind::Drag);
        for (k, e) in m.lists.store.entries.iter().enumerate() {
            let open = m.lists.shown_list.id == Some(e.id);
            // The open one stands aside: the row is in it already.
            let action = Some(Command::AddTo(e.id));
            let _ =
                check_item(v, ui, menu, false, &e.name, &super::count_note(ui, e), action, !open, &format!("c-e{k}"));
        }
        let name = v.id("pl-name");
        show(v, ui, Place::on(name, Side::Below, Along::Start, 4.0).shift(-8.0).fixed(), None);
        // The reveal's own timing: in over 167 ms on the standard curve, no travel.
        v.menus.main.enter_as(Pose::CLEAR_PLACED, 0.167, Ease::Standard);
        // The drag owns the pointer: the plate takes no press, and no press outside closes it.
        let menu_id = menu;
        ui.remove_popup(menu_id);
    });
    p.menus.drag.shown = true;
    p.menus.drag.came = true;
    p.menus.drag.over = None;
}

pub(crate) fn hide_drag_menu(p: &mut Player, ui: &mut Ui) {
    if !p.menus.drag.shown {
        return;
    }
    p.menus.drag.shown = false;
    p.menus.drag.over = None;
    let menu = p.id("flyout");
    p.menus.open = None;
    // Out over 333 ms, accelerating away.
    ui.tree.set_visual(menu, |v| v.hit = false);
    let end = ui.tree.exit(menu, Pose::CLEAR, 0.333, Ease::Accelerate, ui.now(), Then::Hide);
    p.menus.clock.at(end);
}

/// Every turn: the target goes once its time is up.
pub fn drag_tick(p: &mut Player, ui: &mut Ui) {
    if p.menus.drag.shown && ui.now() >= p.menus.drag.hide_at {
        hide_drag_menu(p, ui);
    } else if p.menus.drag.shown && p.menus.drag.hide_at.is_finite() {
        p.menus.clock.at(p.menus.drag.hide_at);
    }
}

pub(crate) fn drag_end(p: &mut Player, ui: &mut Ui) {
    hide_drag_menu(p, ui);
    p.menus.drag = RowDrag::default();
}

/// The song at play position `from` joins the kept playlist `list`.
pub(crate) fn add_to_list(p: &mut Player, ui: &mut Ui, from: usize, list: u32) {
    p.put_row(ui, from, Some(list), false);
}

/// The playlist a drag target row adds to (`Command::AddTo`).
fn drag_target(a: &Action) -> Option<u32> {
    match Command::of(a, || None) {
        Some(Command::AddTo(id)) => Some(id),
        _ => None,
    }
}
