//! The import dialog.
use crate::app::{Models, Views};
use crate::views::menus::*;
use novaui::dsl::Val;
use novaui::*;

/// The import dialog (modelled on WinUI's ContentDialog): a signed-in plugin account's own
/// playlists, each to pick for import (docs/plugins.md §lists).
pub fn open_import_dialog(v: &mut Views, ui: &mut Ui, m: &Models) {
    close_menus(v, ui);
    let Some(ImportPick { plugin, .. }) = &v.import.pick else { return };
    let svc = m
        .plugin
        .registry
        .get(plugin)
        .and_then(|x| x.manifest.as_ref())
        .map(crate::plugins::registry::Manifest::name)
        .unwrap_or_else(|| ui.t("import.online", &[]));
    v.doc.set(ui, "import-title", ui.t("import.title", &[("svc", &svc)]));
    let veil = v.id("import-veil");
    ui.tree.set_layout(veil, |l| l.display = Display::Flex);
    ui.tree.set_action(veil, Some("import-cancel"));
    let card = v.id("import-dialog");
    ui.tree.set_layout(card, |l| l.display = Display::Flex);
    fill_import_list(v, ui);
}

/// The lists' rows and their group headings, in the plugin's order: (heading?, row).
pub(crate) fn import_total(doc: &novaui::dsl::Doc, items: &[crate::app::PickItem]) -> f32 {
    let (row, head) = (doc.num("import_row"), doc.num("import_head"));
    let mut last = None;
    let mut h = 0.0;
    for it in items {
        if !it.group.is_empty() && last != Some(it.group.as_str()) {
            h += head;
        }
        last = Some(it.group.as_str());
        h += row;
    }
    h
}

/// Rebuilds the dialog's rows from `import.pick` (on open, a toggle, 全选): each list a
/// CheckBox row, under its group's heading.
pub(crate) fn fill_import_list(v: &mut Views, ui: &mut Ui) {
    let Some(ImportPick { items, .. }) = &v.import.pick else { return };
    let items: Vec<(String, String, Option<u64>, bool, bool)> =
        items.iter().map(|i| (i.group.clone(), i.name.clone(), i.count, i.on, i.done)).collect();
    let rows = v.id("import-list-rows");
    v.doc.clear(ui, rows);
    let mut last: Option<String> = None;
    for (k, (group, name, count, on, done)) in items.iter().enumerate() {
        if !group.is_empty() && last.as_deref() != Some(group.as_str()) {
            let _ = v.doc.instantiate(ui, rows, None, "ImportGroup", &[Val::Str(group.clone())], &format!("ih{k}"));
        }
        last = Some(group.clone());
        let prefix = format!("il{k}");
        let note = if *done {
            ui.t("import.done", &[])
        } else {
            count.map_or(String::new(), |c| ui.t("songs.n", &[("n", &c.to_string())]))
        };
        let state = Val::Num(if *on || *done { 1.0 } else { 0.0 });
        let Ok(row) =
            v.doc.instantiate(ui, rows, None, "ImportRow", &[Val::Str(name.clone()), Val::Str(note), state], &prefix)
        else {
            continue;
        };
        if *done {
            ui.tree.set_action(row, None);
            ui.tree.set_disabled(row, true);
            ui.tree.set_visual(row, |v| v.opacity = 0.45);
        } else {
            ui.tree.set_action(row, Some(&Command::ImportToggle(k).to_string()));
        }
    }
    let open: Vec<_> = items.iter().filter(|i| !i.4).collect();
    let picked = open.iter().filter(|i| i.3).count();
    let all = if picked == 0 {
        0
    } else if picked == open.len() {
        1
    } else {
        2
    };
    v.doc.set(ui, "import-all", all as f32);
    v.doc.set(
        ui,
        "import-count",
        if picked == 0 { String::new() } else { ui.t("import.picked", &[("n", &picked.to_string())]) },
    );
    place_import_dialog(v, ui);
}

/// Sizes and centres the import dialog: the list shows up to 8 rows' height (a cut row says more
/// follow), the card is its content plus the title and the footer.
pub(crate) fn place_import_dialog(v: &mut Views, ui: &mut Ui) {
    let total = v.import.pick.as_ref().map_or(0.0, |p| import_total(v.doc, &p.items));
    let wh = ui.tree.window_size().1 as f32 / ui.scale();
    // The card never runs off a short window: the list gives the room up first.
    let (fixed, row) = (v.doc.num("dlg_chrome"), v.doc.num("import_row"));
    let view = total.min(row * 8.0).min((wh - 64.0 - fixed).max(row * 2.0));
    let list = v.id("import-list");
    ui.tree.set_layout(list, |l| l.height = Len::Px(view));
    scroll_import(v, ui, total, view);
    let card_h = fixed + view;
    let card = v.id("import-dialog");
    ui.tree.set_layout(card, |l| l.inset[0] = Len::Px(((wh - card_h) / 2.0).max(32.0)));
    let any = v.import.pick.as_ref().is_some_and(|p| p.items.iter().any(|i| i.on));
    ui.tree.set_disabled(v.id("import-ok"), !any);
}

/// Closes the import dialog without importing.
pub fn close_import_dialog(v: &mut Views, ui: &mut Ui) {
    for n in ["import-veil", "import-dialog"] {
        ui.tree.set_layout(v.id(n), |l| l.display = Display::None);
    }
    v.doc.clear(ui, v.id("import-list-rows"));
    v.import.set_pick(None);
}

/// The dialog list's wheel scroll: a notch moves 48 dips, and the rows glide there as every
/// scroll does.
pub(crate) fn wheel_import(v: &mut Views, ui: &mut Ui, dy: f32) {
    let Some((total, view)) = import_extent(v, ui) else { return };
    v.import.scroll.wheel(dy, ui.now(), 0.0, (total - view).max(0.0), crate::views::SCROLL);
    scroll_import(v, ui, total, view);
}

/// The import list's rows where their glide is now, while it moves (each presentation).
pub(crate) fn present_import(v: &mut Views, ui: &mut Ui) {
    if v.import.scroll.done(ui.now()) {
        return;
    }
    if let Some((total, view)) = import_extent(v, ui) {
        scroll_import(v, ui, total, view);
    }
}

/// The import list's content and view heights (dips), while the dialog offers something.
fn import_extent(v: &Views, ui: &Ui) -> Option<(f32, f32)> {
    let ImportPick { items, .. } = v.import.pick.as_ref()?;
    Some((import_total(v.doc, items), ui.tree.size(v.id("import-list")).1 / ui.scale()))
}

/// The import list's scroll kept within its content (`total` in a list `view` high), and its
/// rows standing where the glide is now; a frame follows while it moves.
fn scroll_import(v: &mut Views, ui: &mut Ui, total: f32, view: f32) {
    let now = ui.now();
    v.import.scroll.hold(0.0, (total - view).max(0.0), now, crate::views::SCROLL);
    if !v.import.scroll.done(now) {
        v.menus.clock.frame();
    }
    let off = v.import.scroll.at(now);
    let rows = v.id("import-list-rows");
    ui.tree.set_layout(rows, |l| l.inset[0] = Len::Px(-off));
}

/// `Player` state owned by this area.
#[derive(Default)]
pub struct ImportDialog {
    /// What the dialog offers.
    pub pick: Option<ImportPick>,
    /// The dialog list's scroll offset (dips), gliding as every scroll does.
    pub scroll: novaui::anim::Glide,
}

impl ImportDialog {
    /// What the dialog offers changed: its list scrolls from the top.
    pub fn set_pick(&mut self, pick: Option<ImportPick>) {
        self.pick = pick;
        self.scroll.jump(0.0);
    }
}

/// The import dialog's offering: a plugin and its account's lists, each picked or not.
pub struct ImportPick {
    pub plugin: String,
    pub items: Vec<crate::app::PickItem>,
}
