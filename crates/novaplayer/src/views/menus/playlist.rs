//! The playlist menu.
use crate::app::{Models, Player, Views};
use crate::views::menus::*;
use novaui::dsl::Val;
use novaui::*;

pub fn open_playlist_menu(v: &mut Views, ui: &mut Ui, m: &Models, rename: Option<u32>) {
    let menu = begin(v, ui, Kind::Playlist);
    let entries = m.lists.store.entries.clone();
    if entries.is_empty() {
        let _ = item(v, ui, menu, Icon::MusicNote1, &ui.t("lists.none", &[]), "", None, false, "m-none");
    }
    v.menus.rename_id = None;
    for (k, e) in entries.iter().enumerate() {
        let open = m.lists.shown_list.id == Some(e.id);
        if rename == Some(e.id) {
            // The name box.
            let args =
                [Val::from(Icon::Rename), Val::Key("list.name-hint".into()), Val::Str(Command::ListName.to_string())];
            if v.doc.instantiate(ui, menu, None, "MenuEdit", &args, "m-rename").is_ok() {
                let edit = v.doc.id("m-rename.edit");
                ui.set_edit_text(edit, &e.name);
                ui.set_focus(Some(edit));
                v.menus.rename_id = Some(e.id);
            }
            continue;
        }
        let action = (!open).then_some(Command::Switch(e.id));
        let pre = format!("m-e{k}");
        if let Some(it) = check_item(v, ui, menu, open, &e.name, &super::count_note(ui, e), action, true, &pre) {
            // Delete: hover shows ✕; the first press asks, the second deletes.
            let asking = m.pending == Some(&crate::app::command::Confirm::DeletePlaylist(e.id));
            let args = [Val::Str(Command::Delete(e.id).to_string()), Val::Bool(asking)];
            let _ = v.doc.instantiate(ui, it, None, "MenuDelete", &args, &format!("{pre}.del"));
        }
    }
    let _ = v.doc.instantiate(ui, menu, None, "MenuRule", &[], "m-r1");
    let new = ui.t("list.new", &[]);
    let _ = item(v, ui, menu, Icon::Add, &new, "", Some(Command::New), true, "m-new");
    let can_rename = m.lists.shown_list.id.is_some();
    let _ = item(v, ui, menu, Icon::Rename, &ui.t("menu.rename", &[]), "", Some(Command::Rename), can_rename, "m-ren");
    let _ = v.doc.instantiate(ui, menu, None, "MenuRule", &[], "m-r2");
    let _ =
        item(v, ui, menu, Icon::ArrowImport, &ui.t("menu.add-songs", &[]), "", Some(Command::AddFiles), true, "m-is");
    let _ =
        item(v, ui, menu, Icon::ArrowImport, &ui.t("menu.add-folder", &[]), "", Some(Command::AddFolder), true, "m-if");
    // A link copied to the clipboard, when an enabled plugin turns such links into playlists:
    // the row is enabled once the clipboard has answered (`show_link`).
    let _ = item(
        v,
        ui,
        menu,
        Icon::ArrowImport,
        &ui.t("menu.import-link", &[]),
        "",
        Some(Command::ImportLink),
        false,
        "m-il",
    );
    ui.read_clipboard(crate::app::Answer::MenuLink);
    let _ =
        item(v, ui, menu, Icon::ArrowImport, &ui.t("menu.import-list", &[]), "", Some(Command::Import), true, "m-im");
    let _ = item(
        v,
        ui,
        menu,
        Icon::ArrowExport,
        &ui.t("menu.export", &[]),
        "",
        Some(Command::Export),
        !m.lists.shown_list.is_empty(),
        "m-ex",
    );
    let rescan = m.lists.shown_list.list.origin.rescannable();
    let _ =
        item(v, ui, menu, Icon::ArrowClockwise, &ui.t("menu.rescan", &[]), "", Some(Command::Rescan), rescan, "m-rs");
    let head = v.id("pl-heading");
    show(v, ui, Place::on(head, Side::Below, Along::Start, 4.0), Some(head));
}

/// The clipboard has answered the playlist menu: its "import link" row is enabled when the text
/// is one link an enabled plugin turns into a playlist.
pub(crate) fn show_link(v: &mut Views, ui: &mut Ui, m: &Models, text: Option<&str>) {
    let Some(row) = v.doc.get("m-il") else { return };
    let link = text.map(str::trim).filter(|t| !t.contains('\n'));
    let can = link.is_some_and(|l| crate::plugins::client::plugin_for(&m.plugin.registry, l).is_some());
    ui.tree.set_disabled(row, !can);
}

pub(crate) fn finish_rename(p: &mut Player, ui: &mut Ui, text: &str) {
    if let Some(id) = p.menus.rename_id.take() {
        let name = if text.trim().is_empty() { ui.t("list.default", &[]) } else { text.trim().to_string() };
        p.lists.store.rename(id, &name);
        if p.lists.shown_list.id == Some(id) {
            p.lists.shown_list.list.name = name;
            p.schedule_save();
        }
        p.views(|v, m| open_playlist_menu(v, ui, m, None));
    }
}
