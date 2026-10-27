//! A dropdown's list (WinUI's editable ComboBox): the choices whose names hold the typed words,
//! the row the keys are on lit; its box naming the kept choice. A dropdown is not a
//! settings-page thing: the page only says what its choices are and what one does
//! (`settings_page::SETTINGS`), or an effect's parameter with a few values (`open_param`).
use crate::app::{Models, Player, Views};
use crate::views::menus::*;
use crate::views::settings_page::{setting, SETTINGS};
use novaui::dsl::Doc;
use novaui::*;

/// One choice of a dropdown: what it is called, what `settings.txt` keeps, and every name typing
/// finds it by (`app::query::Query`).
pub struct Choice {
    pub label: String,
    pub value: String,
    pub names: Vec<String>,
}

/// The choice among `rows` that stands for `value`: itself, or (never set, no longer offered: an
/// uninstalled font) the default.
fn standing<'a>(rows: &'a [Choice], value: Option<&str>) -> Option<&'a Choice> {
    rows.iter().find(|c| Some(c.value.as_str()) == value).or(rows.first())
}

/// Puts the setting `key`'s value (`None`: never set) into effect and names it in its box.
pub fn apply(ui: &mut Ui, v: &mut Views, key: &str, value: Option<&str>) {
    let Some(s) = setting(key) else { return };
    let value = if (s.apply)(ui, v, value) { value } else { None };
    name_kept(ui, v.doc, key, value);
}

/// Names `value` (as kept) in the box of `key`: a text, or the text of a box that takes typing.
fn name_kept(ui: &mut Ui, doc: &mut Doc, key: &str, value: Option<&str>) {
    let Some(s) = setting(key) else { return };
    let rows = (s.choices)(ui);
    let Some(label) = standing(&rows, value).map(|c| c.label.clone()) else { return };
    let id = doc.id(&format!("{key}-drop.value"));
    if ui.text_edit(id).is_some() {
        ui.set_edit_text(id, &label);
    } else {
        ui.tree.set_text(id, &label);
    }
}

/// The list of the dropdown `key`: its choices where every word of `query` is found in a name
/// (all of them for an empty one), the best found first (`Query`), the kept one checked. Typing
/// in its box opens it narrowed.
pub fn open(v: &mut Views, ui: &mut Ui, m: &Models, key: &str, query: &str) {
    let Some(s) = setting(key) else { return };
    let all = (s.choices)(ui);
    let current = standing(&all, m.settings.get(key)).map_or(String::new(), |c| c.value.clone());
    let q = crate::app::query::Query::new(query);
    let mut found: Vec<_> =
        all.iter().filter_map(|c| q.find(c.names.iter().map(String::as_str)).map(|f| (f, c))).collect();
    found.sort_by_key(|f| f.0);
    let listed: Vec<_> = found.into_iter().map(|f| f.1).collect();
    let rows: Vec<(Command, String)> = listed
        .iter()
        .map(|c| (Command::Set { key: key.to_owned(), value: c.value.clone() }, c.label.clone()))
        .collect();
    let current = listed.iter().position(|c| c.value == current);
    let box_el = v.id(&format!("{key}-drop.drop"));
    fill(v, ui, key, box_el, &rows, current, !q.is_empty());
}

/// The list of the dropdown of parameter `id` of the effect at `at` (`settings_page`'s effect
/// level): its values as the plugin prints them, the one it has checked.
pub fn open_param(v: &mut Views, ui: &mut Ui, m: &Models, at: usize, id: u32) {
    let Some((_, params)) = &m.playback.effects.params else { return };
    let Some((k, p)) = params.iter().enumerate().find(|(_, p)| p.id == id) else { return };
    let first = p.min.round();
    let rows: Vec<(Command, String)> = p
        .choices
        .iter()
        .enumerate()
        .map(|(j, label)| (Command::Param { at, id, value: first + j as f64 }, label.clone()))
        .collect();
    let current = (p.value.round() - first).max(0.0) as usize;
    let Some(box_el) = v.doc.get(&format!("ep{k}-drop.drop")) else { return };
    let key = format!("ep{k}-drop");
    fill(v, ui, &key, box_el, &rows, Some(current).filter(|c| *c < rows.len()), false);
}

/// A box that takes typing left (Esc, a press elsewhere, a pick): it names the kept choice again.
pub fn left_box(v: &mut Views, ui: &mut Ui, m: &Models, key: &str) {
    name_kept(ui, v.doc, key, m.settings.get(key));
}

/// While a dropdown's box takes typing, it stands without its plate (as the search box does).
pub fn present_boxes(v: &mut Views, ui: &mut Ui) {
    for s in SETTINGS {
        let key = s.key;
        let Some(e) = v.doc.get(&format!("{key}-drop.value")).filter(|e| ui.text_edit(*e).is_some()) else { continue };
        let plate = ui.focus() != Some(e);
        if !plate {
            // Typing in it looks names up here, on the UI thread.
            crate::app::query::warm();
        }
        for n in ["lum", "tint"] {
            let id = v.doc.id(&format!("{key}-drop.{n}"));
            if ui.tree.el(id).visual().visible != plate {
                ui.tree.set_visible(id, plate);
            }
        }
    }
}

/// The list of the dropdown `key` (its box `box_el`): `rows` as `(what a pick does, label)`, row
/// `current` checked; the row the keys are on lit (the first match while typing, else the
/// current one) and in view. Under the box (over it when the room below is short), exactly as
/// wide as it (a long name ends in an ellipsis). Open already (typing narrows it), it is the same
/// flyout, filled again.
#[allow(clippy::too_many_arguments)]
fn fill(
    v: &mut Views,
    ui: &mut Ui,
    key: &str,
    box_el: Id,
    rows: &[(Command, String)],
    current: Option<usize>,
    typing: bool,
) {
    let list = DropList { key: key.to_owned(), picks: rows.iter().map(|(c, _)| c.clone()).collect(), lit: None };
    let menu = begin(v, ui, Kind::Drop(list));
    let w = ui.tree.rect(box_el).w as f32 / ui.scale();
    ui.tree.set_layout(menu, |l| {
        l.width = Len::Px(w);
        l.min_width = Len::Auto;
        l.max_width = Len::Auto;
    });
    for (k, (pick, label)) in rows.iter().enumerate() {
        check_item(v, ui, menu, Some(k) == current, label, "", Some(pick.clone()), true, &format!("dm{k}"));
    }
    if rows.is_empty() {
        let none = ui.t("drop.none", &[]);
        check_item(v, ui, menu, false, &none, "", None, false, "dm-none");
    }
    show(v, ui, Place::on(box_el, Side::Below, Along::Start, 4.0), Some(box_el));
    let lit = if typing { (!rows.is_empty()).then_some(0) } else { current };
    if let Some(k) = lit {
        light(v, ui, Some(k));
        // The current choice in the middle; the first match at the top.
        v.menus.reveal = Some((v.doc.id(&format!("dm{k}.item")), !typing));
    }
}

fn list<'a>(v: &'a mut Views) -> Option<&'a mut DropList> {
    match v.menus.open.as_mut() {
        Some(Kind::Drop(l)) => Some(l),
        _ => None,
    }
}

/// Lights row `k` of the open list (the row the keys are on), the one lit before not.
fn light(v: &mut Views, ui: &mut Ui, k: Option<usize>) {
    let Some(was) = list(v).map(|l| std::mem::replace(&mut l.lit, k)) else { return };
    for (i, on) in [(was, false), (k, true)] {
        if let Some(e) = i.and_then(|i| v.doc.get(&format!("dm{i}.item"))) {
            ui.tree.set_lit(e, on);
        }
    }
}

/// Up / Down along the open list (`by` rows), scrolled to keep the row in view.
pub fn step(v: &mut Views, ui: &mut Ui, by: i32) {
    let Some(l) = list(v) else { return };
    let n = l.picks.len() as i32;
    if n == 0 {
        return;
    }
    let k = match l.lit {
        Some(k) => (k as i32 + by).clamp(0, n - 1),
        None if by > 0 => 0,
        None => n - 1,
    } as usize;
    light(v, ui, Some(k));
    let (host, row) = (v.id("flyout"), v.doc.id(&format!("dm{k}.item")));
    v.menus.main.reveal(ui, host, row, false);
}

/// Enter on a dropdown: the row the keys are on is picked, as a press on it would. False: no
/// list is up, or no row is lit.
pub fn pick(p: &mut Player, ui: &mut Ui) -> bool {
    let Some(Kind::Drop(l)) = p.menus.open.as_ref() else { return false };
    let Some(pick) = l.lit.and_then(|k| l.picks.get(k)).cloned() else { return false };
    crate::app::command::run(p, ui, pick);
    true
}
