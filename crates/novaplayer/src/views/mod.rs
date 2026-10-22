pub mod import_dialog;
pub mod input;
pub mod layout;
pub mod menus;
pub mod now_playing;
pub mod plugin_controls;
pub mod plugin_view;
mod present;
pub mod settings_page;
pub mod side_panel;
pub mod toast;
pub mod transport;

/// How everything that scrolls moves (the queue, the lyrics, the info page, the settings page,
/// the import list, the flyouts): NovaUI's `Glide` on WinUI's curve.
pub(crate) const SCROLL: novaui::anim::GlideSkin = novaui::anim::GlideSkin::WINUI;

/// The scroll rail's skin (`rail_*` in `player.nui`): the queue's, the lyrics' and a scrolling
/// flyout's are the one rail.
/// A rail's wake (`sync_rail`): frames while it moves, its hover delay's time while it waits.
pub(crate) fn rail_wake(clock: &novaui::Clock, ui: &novaui::Ui, wake: Option<f64>) {
    match wake {
        Some(t) if t <= ui.now() => clock.frame(),
        Some(t) => clock.at(t),
        None => {}
    }
}

/// A curve the sheet names by prefix: `<prefix>x1`, `<prefix>y1`, `<prefix>x2`, `<prefix>y2`.
pub(crate) fn sheet_ease(doc: &novaui::dsl::Doc, prefix: &str) -> novaui::Ease {
    let n = |c: &str| doc.num(&format!("{prefix}{c}"));
    novaui::Ease::Bezier(n("x1"), n("y1"), n("x2"), n("y2"))
}

/// Bindings written reach their expressions: a frame on the doc's clock when one moved.
pub(crate) fn update_doc(doc: &mut novaui::dsl::Doc, clock: &novaui::Clock, ui: &mut novaui::Ui) {
    if doc.update(ui) {
        clock.frame();
    }
}

pub(crate) fn rail_skin(doc: &novaui::dsl::Doc) -> novaui::widgets::scroll::Skin {
    novaui::widgets::scroll::Skin {
        thin: doc.num("rail_thin"),
        width: doc.num("rail_width"),
        thumb_width: doc.num("rail_thumb_width"),
        minimum: doc.num("rail_minimum"),
        enter_delay: doc.num("rail_enter") as f64,
        leave_delay: doc.num("rail_leave") as f64,
        duration: doc.num("rail_motion") as f64,
        ease: sheet_ease(doc, "rail_ease_"),
    }
}
