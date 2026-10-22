//! The toast.
use crate::app::*;
use novaui::*;

/// Seconds a toast stays: news, and a failure the user must have time to read.
pub(crate) const TOAST_INFO: f64 = 4.0;

pub(crate) const TOAST_ERROR: f64 = 8.0;

impl Player {
    /// The toast goes away now (state only; `present_toast` takes it off the screen).
    pub(crate) fn hide_toast(&mut self) {
        self.toast.hide();
    }

    /// News: shown `TOAST_INFO`.
    pub fn toast(&mut self, ui: &Ui, s: String) {
        self.toast_for(ui, s, TOAST_INFO);
    }

    /// A failure: shown `TOAST_ERROR`, long enough to read it.
    /// An install (a plugin, a lyric style) failed, and why.
    pub fn toast_install_failed(&mut self, ui: &Ui, why: &str) {
        self.toast_error(ui, ui.t("toast.install-failed", &[("e", why)]));
    }

    pub fn toast_error(&mut self, ui: &Ui, s: String) {
        self.toast_for(ui, s, TOAST_ERROR);
    }

    /// Shows `s` for `secs`; the same message again does not extend it (a repeat is not news).
    /// State only: the toast element follows at the end of the frame (`present_toast`).
    pub fn toast_for(&mut self, ui: &Ui, s: String, secs: f64) {
        let up = self.toast.toast_until > ui.now();
        if up && self.toast.toast_text == s {
            return;
        }
        self.toast.toast_text = s;
        self.toast.toast_until = ui.now() + secs;
        self.toast.stale = true;
    }
}

/// The toast element follows the toast's state: a new message shows (full opacity, woken
/// when it is due to go), a hidden or expired one goes. Idle, it is one branch.
pub(crate) fn present_toast(v: &mut Views, ui: &mut Ui) {
    if v.toast.toast_until > 0.0 && ui.now() >= v.toast.toast_until {
        v.toast.hide();
    }
    if !std::mem::take(&mut v.toast.stale) {
        return;
    }
    let t = v.id("toast");
    if v.toast.toast_until > 0.0 {
        v.doc.set(ui, "toast", v.toast.toast_text.clone());
        ui.tree.set_layout(t, |l| l.display = Display::Flex);
        ui.tree.set_opacity(t, 1.0);
        v.toast.clock.at(v.toast.toast_until);
    } else {
        ui.tree.set_layout(t, |l| l.display = Display::None);
    }
}

/// `Player` state owned by this area.
pub struct Toast {
    /// Its frames and deadlines (`novaui::Clock`).
    pub clock: novaui::Clock,
    pub toast_until: f64,
    /// What the toast says now (a repeat does not extend it).
    pub toast_text: String,
    /// The state changed and the element has not followed yet.
    stale: bool,
}

impl Toast {
    /// It goes away: no deadline, no text, its element to be taken off.
    pub fn hide(&mut self) {
        self.toast_until = 0.0;
        self.toast_text.clear();
        self.stale = true;
    }

    /// Its clock is the toast's end (`Msg::Toast`).
    pub fn new(ui: &mut novaui::Ui) -> Self {
        Toast {
            clock: ui.clock(crate::app::Msg::Toast),
            toast_until: Default::default(),
            toast_text: Default::default(),
            stale: Default::default(),
        }
    }
}
