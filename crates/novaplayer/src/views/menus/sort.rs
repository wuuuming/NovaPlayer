//! The sort menu.
use crate::app::{Models, Views};
use crate::views::menus::*;
use novaui::*;

/// The List button's menu (WinUI DropDown): the queue's sort keys. The one in force carries its
/// direction (up: ascending, down: descending); choosing it again reverses it, choosing another
/// starts ascending.
pub(crate) fn open_sort_menu(v: &mut Views, ui: &mut Ui, m: &Models, button: Id) {
    let menu = begin(v, ui, Kind::Sort);
    let now = m.lists.shown_list.sorted_by();
    for (k, s) in crate::library::queue::Sort::ALL.into_iter().enumerate() {
        let (icon, on, desc) = match now {
            Some((n, d)) if n == s => (if d { Icon::ArrowDownFilled } else { Icon::ArrowUpFilled }, true, !d),
            _ => (Icon::ArrowUpFilled, false, false),
        };
        let action = Some(Command::Sort(s, desc));
        let prefix = format!("sm{k}");
        let label = ui.t(&format!("sort.{}", s.key()), &[]);
        if item(v, ui, menu, icon, &label, "", action, !m.lists.shown_list.is_empty(), &prefix).is_some() {
            ui.tree.set_off(v.doc.id(&format!("{prefix}.icon")), !on);
        }
    }
    // Hung under the button, its right edge on the button's.
    show(v, ui, Place::on(button, Side::Below, Along::End, 4.0), Some(button));
}
