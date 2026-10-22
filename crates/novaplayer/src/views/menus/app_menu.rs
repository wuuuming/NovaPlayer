//! The app menu.
use crate::app::{Models, Views};
use crate::views::menus::*;
use novaui::*;

/// The app menu under the title bar's menu button: each installed plugin with its state;
/// pressing one opens its flyout beside this one (`plugin_flyout.rs`). Then install and the
/// folder, cache, and the settings page last. Opened again while up, it is rebuilt in place (a
/// connection finished).
pub fn open_app_menu(v: &mut Views, ui: &mut Ui, m: &Models) {
    use crate::views::plugin_view::PluginView;
    let menu = begin(v, ui, Kind::App);
    let list: Vec<crate::plugins::registry::Plugin> = m.plugin.registry.list.clone();
    if list.is_empty() {
        let none = ui.t("plugins.none", &[]);
        let _ = item(v, ui, menu, Icon::Navigation, &none, "", None, false, "pl-none");
    }
    let mut side = None;
    for (k, pl) in list.iter().enumerate() {
        let pv = PluginView::of(ui, m.plugin, pl, None);
        let prefix = format!("pl{k}");
        let open = v.menus.plugin.as_deref() == Some(pv.id.as_str());
        // The row opens the plugin's flyout (a chevron says so); why one is broken is there.
        let note = if pv.broken.is_some() { "" } else { pv.note.as_str() };
        let Some(row) =
            check_item(v, ui, menu, pv.on, &pl.name(), note, Some(Command::PluginFlyout(pv.id.clone())), true, &prefix)
        else {
            continue;
        };
        if pv.broken.is_some() {
            let ic = v.doc.id(&format!("{prefix}.icon"));
            ui.tree.set_icon(ic, Icon::ErrorCircle);
            ui.tree.set_off(ic, false);
        }
        let _ = v.doc.instantiate(ui, row, None, "MenuChevron", &[], &format!("{prefix}.more"));
        ui.tree.set_lit(row, open);
        if open {
            side = Some((pl.clone(), row));
        }
    }
    let _ = v.doc.instantiate(ui, menu, None, "MenuRule", &[], "pl-r0");
    let _ = item(
        v,
        ui,
        menu,
        Icon::ArrowImport,
        &ui.t("menu.plugin-install", &[]),
        "",
        Some(Command::PluginInstall),
        true,
        "pl-inst",
    );
    let _ = item(
        v,
        ui,
        menu,
        Icon::ArrowExport,
        &ui.t("menu.plugin-folder", &[]),
        "",
        Some(Command::PluginFolder),
        m.plugin.registry.root.is_some(),
        "pl-dir",
    );
    let _ = item(
        v,
        ui,
        menu,
        Icon::ArrowClockwise,
        &ui.t("menu.plugin-reload", &[]),
        "",
        Some(Command::PluginReload),
        true,
        "pl-rl",
    );
    let _ = item(
        v,
        ui,
        menu,
        Icon::Delete,
        &ui.t("menu.cache-clear", &[]),
        "",
        Some(Command::OnlineCacheClear),
        true,
        "pl-cc",
    );
    let _ = v.doc.instantiate(ui, menu, None, "MenuRule", &[], "pl-r1");
    let settings = ui.t("settings.open", &[]);
    let _ =
        item(v, ui, menu, Icon::Settings, &settings, "", Some(Command::Settings(Default::default())), true, "pl-set");
    let button = v.id("app-menu");
    show(v, ui, Place::on(button, Side::Below, Along::Start, 4.0).shift(4.0), Some(button));
    // Its plugin's flyout, beside the row it belongs to.
    match side {
        Some((pl, row)) => crate::views::menus::plugin_flyout::open_panel(v, ui, m, &pl, row),
        None => close_plugin_flyout(v, ui),
    }
}
