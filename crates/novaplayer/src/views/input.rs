//! Event handling: clicks by action name, keys (spec, "Transport under the art"), drags (scrubber, gain, queue rows),
//! menus (the spec's playlist menu, "Playlists"; row menu, "Queue page"), files.

use crate::app::command::{run, Command, KEYS};
use crate::app::Player;
use crate::views::import_dialog::*;
use crate::views::menus::*;
use crate::views::menus::{cover::*, drag::*, output::*, playlist::*, row::*};
use crate::views::side_panel::{Narrow, Page};
use novaui::*;
use std::time::Duration;

use novaui::input::key::{BROWSER_BACK, DOWN, END, ENTER, ESCAPE, HOME, PAGE_DOWN, PAGE_UP, UP};

/// The open dialog's sound files (`filter.music`).
/// The control bar's volume bar.
pub(crate) fn is_gain(p: &Player, id: Id) -> bool {
    id == p.id("gain")
}

pub(crate) fn frac_in(ui: &Ui, id: Id, x: i32) -> f32 {
    let r = ui.tree.rect(id);
    ((x - r.x) as f32 / r.w.max(1) as f32).clamp(0.0, 1.0)
}

pub fn handle(p: &mut Player, ui: &mut Ui, ev: Event) {
    handle_event(p, ui, ev);
    // A handler that changed the song closes its option flyout before the next event.
    p.views(|v, m| crate::views::menus::close_stale(v, ui, m));
}

fn handle_event(p: &mut Player, ui: &mut Ui, ev: Event) {
    if p.side_panel.rail_event(ui, &p.doc, &p.lists.shown_list, &ev) {
        return;
    }
    if p.views(|v, _| crate::views::menus::flyout_event(v, ui, &ev)) {
        return;
    }
    match ev {
        Event::Click { id, action: Some(a) } => {
            if let Some(c) = crate::app::command::of_press(p, ui, id, &a) {
                run(p, ui, c);
            }
        }
        // A slider pressed and let go without a drag.
        Event::Click { id, .. } if p.settings_page.dragging == Some(id) => {
            p.views(|v, m| crate::views::settings_page::slid(v, ui, m));
        }
        Event::Click { .. } => {}
        Event::DoubleClick { id, action } => {
            if action.is_none() && ui.tree.is_descendant(id, p.id("titlebar")) {
                ui.window.toggle_maximize();
            }
        }
        Event::RightClick { id, x, y } => {
            if id == p.id("order") {
                // Shuffling: a right press draws a new order (the playing song stays first).
                if p.playing_queue_mut().reshuffle() {
                    if p.lists.playing_list.is_none() {
                        p.side_panel.locate = true;
                    }
                    p.schedule_save();
                    p.toast(ui, ui.t("toast.reshuffled", &[]));
                }
            } else if id == p.id("vol") {
                // Listed again each time; shown at once when listed before, else when they land.
                p.list_outputs(ui);
                if p.playback.output.listed {
                    p.views(|v, m| open_output_menu(v, ui, m));
                } else {
                    p.playback.output.asked = true;
                }
            } else if id == p.id("cover") && p.playback.opened {
                p.views(|v, m| open_cover_menu(v, ui, m, x, y));
            } else if let Some(place) = row_place(p, ui, id) {
                p.views(|v, m| open_row_menu(v, ui, m, place, x, y));
            }
        }
        Event::PressOutside { .. } => p.views(|v, _| close_menus(v, ui)),
        Event::Wheel { id, dy } => {
            if is_gain(p, id) || id == p.id("vol") {
                // One notch (48 dips) = 5 %: the volume is reachable at any window size.
                p.set_volume(p.playback.volume + dy / 48.0 * 0.05);
                p.show_vol_tip(ui);
            } else if id == p.id("q-list") {
                let q = &p.lists.shown_list;
                p.side_panel.wheel_queue(dy, q, ui, &p.doc);
            } else if id == p.id("lyrics-pane") {
                p.side_panel.wheel_lyrics(dy, ui.now());
            } else if id == p.id("info-pane") {
                p.side_panel.wheel_info(ui, &p.doc, dy);
            } else if id == p.id("settings-page") {
                p.settings_page.wheel(ui, &mut p.doc, dy);
            } else if id == p.id("import-list") {
                p.views(|v, _| wheel_import(v, ui, dy));
            }
        }
        Event::Press { id, x, .. } => {
            if id == p.id("track") && !ui.tree.el(id).disabled() {
                p.transport.scrub = Some(frac_in(ui, id, x));
            } else if is_gain(p, id) {
                p.transport.gain_drag = true;
                let v = frac_in(ui, id, x);
                p.set_volume(v);
            } else if let Some(c) = p.views(|v, _| crate::views::settings_page::slide(v, ui, id, x)) {
                run(p, ui, c);
            }
        }
        Event::DragStart { id, y, .. } => {
            if let Some(place) =
                row_place(p, ui, id).filter(|_| ui.tree.el(id).action().is_some_and(|a| a.is("row-grip")))
            {
                let y_list = queue_y(p, ui, y) + p.side_panel.scroll;
                let grab = y_list - place as f32 * p.side_panel.row_h();
                p.side_panel.drag_start(place, grab);
            }
        }
        Event::Drag { id, x, y } => {
            if id == p.id("track") && p.transport.scrub.is_some() {
                p.transport.scrub = Some(frac_in(ui, id, x));
            } else if is_gain(p, id) {
                let v = frac_in(ui, id, x);
                p.set_volume(v);
            } else if p.settings_page.dragging == Some(id) {
                if let Some(c) = p.views(|v, _| crate::views::settings_page::slide(v, ui, id, x)) {
                    run(p, ui, c);
                }
            } else if p.side_panel.drag.is_some() {
                let (_, lh) = ui.tree.size(p.id("q-list"));
                if p.side_panel.drag_to(queue_y(p, ui, y), lh, 1.0 / 60.0) {
                    p.side_panel.clock.frame();
                }
                drag_over(p, ui, x, y);
            }
        }
        Event::DragEnd { id, x, y } => {
            if id == p.id("track") {
                if let (Some(_), Some(d)) = (p.transport.scrub.take(), p.playback.snap.duration) {
                    let f = frac_in(ui, id, x);
                    p.seek(d.mul_f32(f));
                }
            } else if is_gain(p, id) {
                p.transport.gain_drag = false;
            } else if p.settings_page.dragging == Some(id) {
                p.views(|v, m| crate::views::settings_page::slid(v, ui, m));
            } else if p.side_panel.drag.is_some() {
                let (target, faded) = (p.menus.drag.over, p.menus.drag.came && !p.menus.drag.shown);
                let from = p.side_panel.drag.map(|d| d.0);
                if let (Some(list), Some(from)) = (target, from) {
                    // 8.5: let go on a playlist: the song joins it; the row goes back.
                    p.side_panel.drag = None;
                    p.side_panel.end_drag();
                    add_to_list(p, ui, from, list);
                } else if faded || !ui.tree.rect(p.id("q-list")).contains(x, y) {
                    // Carried off the target once it had come up: called off, nothing moves.
                    p.side_panel.drag = None;
                    p.side_panel.end_drag();
                } else if let Some((from, to)) = p.side_panel.drag_end() {
                    p.lists.shown_list.move_to(from, to);
                    p.schedule_save();
                }
                drag_end(p, ui);
            }
        }
        Event::DragCancel { .. } => {
            p.side_panel.end_drag();
            drag_end(p, ui);
            p.transport.scrub = None;
            p.transport.gain_drag = false;
            p.views(|v, m| crate::views::settings_page::slid(v, ui, m));
        }
        Event::Key { vk, mods } => key(p, ui, vk, mods),
        Event::MiddleDown { id, x, y } => {
            if id == p.id("q-list") {
                p.side_panel.autoscroll_start(ui, &p.doc, y);
                let _ = x;
            } else if p.side_panel.autoscrolling() {
                p.side_panel.autoscroll_stop(ui);
            }
        }
        Event::MiddleMove { y, .. } => p.side_panel.autoscroll_steer(ui, y),
        Event::MiddleUp { .. } => p.side_panel.autoscroll_release(ui),
        // A field's events carry what it is for (`on-edit`).
        Event::Edit { id, action: Some(a), text } => match Command::of(&a, || None) {
            Some(Command::Search) => {
                p.side_panel.set_narrow(if text.trim().is_empty() {
                    None
                } else {
                    Some(Narrow { text, ..Default::default() })
                });
            }
            Some(Command::Form(k)) => {
                if let Some(plugin) = form_plugin(p, ui, id) {
                    p.plugin.form.set(&plugin, k, text);
                }
            }
            Some(Command::DropTyped(key)) => {
                p.views(|v, md| crate::views::menus::dropdown::open(v, ui, md, &key, &text))
            }
            _ => {}
        },
        Event::Submit { id, action: Some(a), text } => match Command::of(&a, || None) {
            Some(Command::ListName) => finish_rename(p, ui, &text),
            // Enter in a connection form connects.
            Some(Command::Form(_)) => {
                if let Some(plugin) = form_plugin(p, ui, id) {
                    run(p, ui, Command::Connect(plugin));
                }
            }
            // Enter in a dropdown's box picks the lit row; with none, it only leaves the box.
            Some(Command::DropTyped(_)) => {
                crate::views::menus::dropdown::pick(p, ui);
                ui.set_focus(None);
            }
            _ => {}
        },
        Event::Blur { id, action: Some(a) } => match Command::of(&a, || None) {
            Some(Command::ListName) => {
                let t = ui.edit_text(id).to_string();
                finish_rename(p, ui, &t);
            }
            Some(Command::DropTyped(key)) => p.views(|v, md| crate::views::menus::dropdown::left_box(v, ui, md, &key)),
            _ => {}
        },
        // Esc in a dropdown's box is the box's (WinUI's editable ComboBox): it closes the list,
        // not the page under it. In another field it leaves the field and goes on as Esc.
        Event::Cancel { action, .. } => {
            if matches!(Command::of(&action, || None), Some(Command::DropTyped(_))) {
                p.views(|v, _| close_menus(v, ui));
            } else {
                key(p, ui, ESCAPE, Mods::default());
            }
        }
        Event::Edit { .. } | Event::Submit { .. } | Event::Blur { .. } => {}
        Event::Files(paths) => p.add_paths(ui, paths, false),
        // WinUI dismisses light-dismiss flyouts when the window changes size: their anchor moved.
        Event::Resized => p.views(|v, _| close_menus(v, ui)),
        Event::Hover { id, .. } | Event::Pressed { id, .. } => {
            p.views(|v, _| crate::views::settings_page::switch_felt(v, ui, id))
        }
        // The keyboard or a screen reader set a slider: the song's position (s), the volume (%),
        // an effect's parameter.
        Event::Value { id, value } => {
            if id == p.id("track") {
                if p.playback.snap.duration.is_some() && !ui.tree.el(id).disabled() {
                    p.seek(Duration::from_secs_f64(value.max(0.0)));
                }
            } else if is_gain(p, id) {
                p.set_volume(value as f32 / 100.0);
                p.show_vol_tip(ui);
            } else if let Some(c) = p.views(|v, _| crate::views::settings_page::set_slider(v, ui, id, value)) {
                run(p, ui, c);
            }
        }
        // The keyboard focus came to an element: in the queue or on the settings page, it
        // scrolls into view.
        Event::Reveal { id } => {
            if let Some(place) = row_place(p, ui, id) {
                let q = &p.lists.shown_list;
                p.side_panel.reveal_row(place, q, ui, &p.doc);
                p.side_panel.clock.frame();
            } else {
                p.settings_page.reveal(ui, &mut p.doc, id);
            }
        }
        // The player's window is no overlay: it is never moved as one.
        Event::Message(_) | Event::WindowFocus(_) | Event::Moved => {}
    }
}

pub(crate) fn row_place(p: &Player, ui: &Ui, id: Id) -> Option<usize> {
    let list = p.id("q-list");
    let mut cur = Some(id);
    while let Some(c) = cur {
        if ui.tree.el(c).action().is_some_and(|a| a.name.starts_with("row")) {
            return Some(ui.tree.el(c).tag() as usize);
        }
        if c == list {
            return None;
        }
        cur = ui.tree.parent(c);
    }
    None
}

/// Back one step in the side panel. A search left this way leaves its box empty and unfocused
/// at the next present (`present_search`, the one place that clears it).
pub(crate) fn back_panel(p: &mut Player) {
    p.side_panel.back();
}

pub(crate) fn key(p: &mut Player, ui: &mut Ui, vk: u32, mods: Mods) {
    if vk == ESCAPE && p.side_panel.autoscrolling() {
        p.side_panel.autoscroll_stop(ui);
        return;
    }
    if p.menus.open.is_some() {
        // A dropdown's list: the keys step along it and Enter picks.
        match vk {
            UP | DOWN => p.views(|v, _| crate::views::menus::dropdown::step(v, ui, if vk == UP { -1 } else { 1 })),
            ENTER => {
                crate::views::menus::dropdown::pick(p, ui);
            }
            _ => {}
        }
        if vk == ESCAPE {
            // The plugin panel goes first; the flyout under it on the next press.
            if p.menus.plugin.is_some() || p.menus.pick.is_some() {
                p.views(|v, _| close_plugin_flyout(v, ui));
            } else {
                p.views(|v, _| close_menus(v, ui));
            }
        }
        return;
    }
    if mods.ctrl || mods.alt {
        return;
    }
    // In a text field, Up / Down are not the volume's. A dropdown's box opens its list with them.
    if let Some(f) = ui.focused_field().filter(|_| matches!(vk, UP | DOWN | PAGE_UP | PAGE_DOWN)) {
        let action = ui.text_edit(f).and_then(|e| e.action.clone());
        let typed = action.and_then(|a| Command::of(&a, || None)).filter(|_| matches!(vk, UP | DOWN));
        if let Some(Command::DropTyped(key)) = typed {
            p.views(|v, m| crate::views::menus::dropdown::open(v, ui, m, &key, ""));
        }
        return;
    }
    // Page keys scroll the open queue (a page less a row, WinUI ScrollViewer's), Home / End to
    // its ends; outside a text field.
    let queue_up = p.side_panel.shown.shows(Page::Queue) && ui.focused_field().is_none();
    if queue_up && matches!(vk, PAGE_UP | PAGE_DOWN | HOME | END) {
        let q = &p.lists.shown_list;
        p.side_panel.page_queue(vk == PAGE_DOWN || vk == END, matches!(vk, HOME | END), q, ui, &p.doc);
        return;
    }
    if let Some((_, c)) = KEYS.iter().find(|(k, _)| *k == vk) {
        return run(p, ui, c.clone());
    }
    match vk {
        ESCAPE => {
            if p.settings_page.open && ui.focused_field().is_none() {
                p.settings_page.show(ui, &mut p.doc, false);
            } else if ui.window.fullscreen() {
                ui.window.set_fullscreen(false);
            } else if p.side_panel.second_level() && ui.focused_field().is_none() {
                back_panel(p);
            }
        }
        // The mouse's back button goes up one level, as in the queue: on the settings page, its
        // parent level (the page's first level's parent is playback).
        BROWSER_BACK if p.settings_page.open => p.views(|v, md| crate::views::settings_page::up(v, ui, md)),
        BROWSER_BACK => back_panel(p),
        _ => {}
    }
}

/// The plugin whose sign-in form the field `id` is in: its flyout's, or its settings page's.
fn form_plugin(p: &Player, ui: &Ui, id: Id) -> Option<String> {
    let flyout = p.id("plugin-flyout");
    if ui.tree.ancestors(id).any(|a| a == flyout) {
        return p.menus.plugin.clone();
    }
    match &p.settings_page.level {
        crate::views::settings_page::Level::Plugin(x) => Some(x.clone()),
        _ => None,
    }
}

/// Where the pointer is from the queue list's top, in dips (window pixel `y`).
fn queue_y(p: &Player, ui: &Ui, y: i32) -> f32 {
    let r = ui.tree.rect(p.id("q-list"));
    (y - r.y) as f32 / ui.scale()
}
