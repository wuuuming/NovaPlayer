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

/// The playlists the row's song can go to, beside its 添加到 / 移动到 row: a new one first, then
/// every kept one but the shown list (the song is in it already). Pressed again, it goes.
pub(crate) fn open_pick(v: &mut Views, ui: &mut Ui, m: &Models, pos: usize, mv: bool) {
    let row = v.doc.get(if mv { "rm-mv.item" } else { "rm-add.item" });
    let (Some(row), true) = (row, is_open(v, |k| *k == Kind::Row)) else { return };
    let again = v.menus.pick == Some((pos, mv));
    close_plugin_flyout(v, ui);
    for name in ["rm-add.item", "rm-mv.item"] {
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
    let put = |list| if mv { Command::MoveRow(pos, list) } else { Command::AddRow(pos, list) };
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
