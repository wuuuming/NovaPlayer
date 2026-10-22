//! The artwork menu.
use crate::app::{Models, Views};
use crate::views::menus::*;
use novaui::*;

/// 4.11: the artwork's menu, for a local song without its own picture: change the cover, and
/// restore it once one was chosen. Any other song's artwork has no menu (its info is the queue
/// row's, and the control bar's info button's).
pub(crate) fn open_cover_menu(v: &mut Views, ui: &mut Ui, m: &Models, x: i32, y: i32) {
    if !(m.playback.opened && v.now_playing.cover_editable) {
        return close_menus(v, ui);
    }
    let menu = begin(v, ui, Kind::Cover);
    let _ =
        item(v, ui, menu, Icon::Rename, &ui.t("menu.cover-change", &[]), "", Some(Command::CoverChange), true, "cm-ch");
    if m.cover_chosen() {
        let _ = item(
            v,
            ui,
            menu,
            Icon::ArrowClockwise,
            &ui.t("menu.cover-restore", &[]),
            "",
            Some(Command::CoverRestore),
            true,
            "cm-rs",
        );
    }
    let s = ui.scale();
    show(v, ui, Place::at(x as f32 / s, y as f32 / s), None);
}
