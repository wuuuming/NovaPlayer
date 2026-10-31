//! The queue row menu, and the playlist picker beside it.
use crate::app::{Models, Views};
use crate::views::menus::*;
use novaui::*;

/// A queue row's menu: to the top, 添加到 › and 移动到 › (a playlist picked beside it), remove;
/// the song's info last. The list's current song (the one that plays, or where the list goes on
/// from) neither moves nor goes.
pub(crate) fn open_row_menu(v: &mut Views, ui: &mut Ui, m: &Models, place: usize, x: i32, y: i32) {
    let Some(pos) = v.side_panel.pos_of(place) else { return close_menus(v, ui) };
    let menu = begin(v, ui, Kind::Row);
    let stays = pos == m.lists.shown_list.pos;
    let _ = item(
        v,
        ui,
        menu,
        Icon::ArrowUpload,
        &ui.t("menu.to-top", &[]),
        "",
        Some(Command::ToTop(pos)),
        pos != 0,
        "rm-top",
    );
    for (verb, mv, enabled, name) in [("menu.add-to", false, true, "rm-add"), ("menu.move-to", true, !stays, "rm-mv")] {
        let icon = if mv { Icon::ArrowExport } else { Icon::Add };
        if let Some(row) = item(v, ui, menu, icon, &ui.t(verb, &[]), "", Some(Command::RowPick(pos, mv)), enabled, name)
        {
            let _ = v.doc.instantiate(ui, row, None, "MenuChevron", &[], &format!("{name}.more"));
        }
    }
    let _ =
        item(v, ui, menu, Icon::Delete, &ui.t("menu.remove", &[]), "", Some(Command::Remove(pos)), !stays, "rm-del");
    let _ = v.doc.instantiate(ui, menu, None, "MenuRule", &[], "rm-r");
    let _ = item(
        v,
        ui,
        menu,
        Icon::MusicInfoFilled,
        &ui.t("info.title", &[]),
        "",
        Some(Command::RowInfo(pos)),
        true,
        "rm-info",
    );
    let s = ui.scale();
    show(v, ui, Place::at(x as f32 / s, y as f32 / s), None);
}

/// The chosen rows' menu (multi-select): how many are chosen, then 添加到 › and 移动到 › (a
/// playlist picked beside it) and 删除 (a second press does it). The list's current song is
/// neither moved nor taken out: with only it chosen, those rows are off.
pub(crate) fn open_selection_menu(v: &mut Views, ui: &mut Ui, m: &Models, x: i32, y: i32) {
    let at = v.side_panel.selected_positions(&m.lists.shown_list);
    let movable = at.iter().any(|&p| p != m.lists.shown_list.pos);
    let menu = begin(v, ui, Kind::Selection);
    let n = at.len().to_string();
    let _ = item(v, ui, menu, Icon::MultiselectLtr, &ui.t("select.n", &[("n", &n)]), "", None, false, "sm-n");
    let _ = v.doc.instantiate(ui, menu, None, "MenuRule", &[], "sm-r");
    for (verb, mv, enabled, name) in
        [("menu.add-to", false, !at.is_empty(), "sm-add"), ("menu.move-to", true, movable, "sm-mv")]
    {
        let icon = if mv { Icon::ArrowExport } else { Icon::Add };
        if let Some(row) = item(v, ui, menu, icon, &ui.t(verb, &[]), "", Some(Command::SelPick(mv)), enabled, name) {
            let _ = v.doc.instantiate(ui, row, None, "MenuChevron", &[], &format!("{name}.more"));
        }
    }
    let _ = item(v, ui, menu, Icon::Delete, &ui.t("menu.remove", &[]), "", Some(Command::RemoveSel), movable, "sm-del");
    let s = ui.scale();
    show(v, ui, Place::at(x as f32 / s, y as f32 / s), None);
}

/// 删除's first press: it says what the second does, lit.
pub(crate) fn ask_remove(v: &mut Views, ui: &mut Ui, m: &Models) {
    let n =
        v.side_panel.selected_positions(&m.lists.shown_list).iter().filter(|&&p| p != m.lists.shown_list.pos).count();
    if let (Some(label), Some(row)) = (v.doc.get("sm-del.label"), v.doc.get("sm-del.item")) {
        ui.tree.set_text(label, &ui.t("menu.remove-n.confirm", &[("n", &n.to_string())]));
        ui.tree.set_lit(row, true);
    }
}

/// The playlists the row's song (or the chosen songs, for `None`) can go to, beside its
/// 添加到 / 移动到 row: a new one first, then every kept one but the shown list (the songs are
/// in it already). Pressed again, it goes.
pub(crate) fn open_pick(v: &mut Views, ui: &mut Ui, m: &Models, pos: Option<usize>, mv: bool) {
    let (add, mov) = if pos.is_some() { ("rm-add.item", "rm-mv.item") } else { ("sm-add.item", "sm-mv.item") };
    let row = v.doc.get(if mv { mov } else { add });
    let (Some(row), true) = (row, is_open(v, |k| matches!(k, Kind::Row | Kind::Selection))) else { return };
    let again = v.menus.pick == Some((pos, mv));
    close_plugin_flyout(v, ui);
    for name in [add, mov] {
        if let Some(r) = v.doc.get(name) {
            ui.tree.set_lit(r, r == row && !again);
        }
    }
    if again {
        return;
    }
    let host = v.id("plugin-flyout");
    let panel = v.menus.side.reset(ui, host);
    v.doc.prune(ui);
    v.menus.pick = Some((pos, mv));
    let put = |list| match (pos, mv) {
        (Some(pos), true) => Command::MoveRow(pos, list),
        (Some(pos), false) => Command::AddRow(pos, list),
        (None, true) => Command::MoveSel(list),
        (None, false) => Command::AddSel(list),
    };
    let _ = item(v, ui, panel, Icon::Add, &ui.t("list.new", &[]), "", Some(put(None)), true, "rp-new");
    let shown = m.lists.shown_list.id;
    let mut rule = true;
    for (k, e) in m.lists.store.entries.iter().enumerate().filter(|(_, e)| Some(e.id) != shown) {
        if std::mem::take(&mut rule) {
            let _ = v.doc.instantiate(ui, panel, None, "MenuRule", &[], "rp-r");
        }
        let n = super::count_note(ui, e);
        let _ = item(v, ui, panel, Icon::MusicNote1, &e.name, &n, Some(put(Some(e.id))), true, &format!("rp-e{k}"));
    }
    show_side(v, ui, row);
}
