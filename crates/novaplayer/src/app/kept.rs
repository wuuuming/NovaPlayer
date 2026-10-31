//! What must outlive the process (principles R5: one owner). Each file of kept state is a
//! [`Kept`]: marked changed where it changes, written a fixed time after its first change since
//! it was last written (a slider drag, a burst of list edits is one write), by the store's
//! thread, and at once by `Player::rescue` (on close and on a panic).
//!
//! The text is made by `Player::keep` from the state as it is when the write is due; the
//! effects chain writes its own files, its plugins' states asked on the store's thread.

use novaui::Clock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kept {
    /// `settings.txt`: the settings page's choices.
    Settings,
    /// `options.txt`: each plugin's chosen option.
    Options,
    /// `output.txt`: where the sound goes.
    Output,
    /// `lyrics-layers.txt`: the lyric layers switched off.
    Layers,
    /// `volume.txt`.
    Volume,
    /// `effects.txt` and `effects/`: the effects chain and its plugins' states.
    Effects,
    /// The playlists, their index, the queue's place in them.
    Lists,
    /// `window.txt`: where the window and its side panel were (`settings::window_state`).
    Window,
}

impl Kept {
    pub const ALL: [Kept; 8] = [
        Kept::Settings,
        Kept::Options,
        Kept::Output,
        Kept::Layers,
        Kept::Volume,
        Kept::Effects,
        Kept::Lists,
        Kept::Window,
    ];

    /// How long after its first change it is written (seconds): a choice at once, what a drag
    /// or playing moves later.
    fn after(self) -> f64 {
        match self {
            Kept::Settings | Kept::Options | Kept::Output | Kept::Layers => 0.0,
            Kept::Volume | Kept::Window => 1.0,
            Kept::Effects | Kept::Lists => 2.0,
        }
    }
}

/// Which kept files have changed, and when each is due.
pub struct Keeping {
    due: [Option<f64>; Kept::ALL.len()],
    /// Changed since the last turn, its time not yet set.
    changed: [bool; Kept::ALL.len()],
    clock: Clock,
}

impl Keeping {
    pub fn new(clock: Clock) -> Self {
        Keeping { due: [None; Kept::ALL.len()], changed: [false; Kept::ALL.len()], clock }
    }

    /// `what` changed: it is written its time after the first change not yet written.
    pub fn changed(&mut self, what: Kept) {
        self.changed[what as usize] = true;
    }

    /// Changed and not yet written.
    pub fn pending(&self, what: Kept) -> bool {
        self.changed[what as usize] || self.due[what as usize].is_some()
    }

    /// At the end of a turn: what is due at `now` (no longer pending); the clock is set for
    /// the next.
    pub fn due(&mut self, now: f64) -> Vec<Kept> {
        let mut out = Vec::new();
        let mut next = f64::INFINITY;
        for k in Kept::ALL {
            let i = k as usize;
            if std::mem::take(&mut self.changed[i]) && self.due[i].is_none() {
                self.due[i] = Some(now + k.after());
            }
            match self.due[i] {
                Some(t) if t <= now => {
                    self.due[i] = None;
                    out.push(k);
                }
                Some(t) => next = next.min(t),
                None => {}
            }
        }
        if next.is_finite() {
            self.clock.at(next);
        }
        out
    }

    /// Everything pending, at once (`Player::rescue`); none is pending after.
    pub fn all(&mut self) -> Vec<Kept> {
        Kept::ALL
            .into_iter()
            .filter(|&k| {
                let i = k as usize;
                std::mem::take(&mut self.changed[i]) | self.due[i].take().is_some()
            })
            .collect()
    }
}

impl super::Player {
    /// Writes `what` as it is now, on the store's thread.
    pub fn keep(&mut self, what: Kept) {
        let store = &self.lists.store;
        let file = |file: Option<std::path::PathBuf>, text: String| {
            if let Some(f) = file {
                store.keep(f, text.into_bytes(), None);
            }
        };
        match what {
            Kept::Settings => file(self.settings.path().map(Into::into), self.settings.text()),
            Kept::Options => file(crate::plugins::client::option::file(), crate::plugins::client::option::text()),
            Kept::Layers => file(
                super::settings::layer_setting::file(),
                super::settings::layer_setting::text(&self.now_playing.layers_off),
            ),
            Kept::Volume => {
                if let Some((f, text)) = self.playback.kept_gain.unwritten() {
                    file(Some(f), text);
                }
            }
            Kept::Output => {
                let output = &self.playback.output;
                if let Some(f) = output.file() {
                    let inbox = self.inbox.clone();
                    let failed = move |e: std::io::Error| {
                        let note = novaui::i18n::t("output.not-saved", &[("e", &e.to_string())]);
                        inbox.send(super::Job::NotKept(note));
                    };
                    store.keep(f.to_owned(), output.choice.text().into_bytes(), Some(Box::new(failed)));
                }
            }
            Kept::Effects => {
                if let Some(write) = self.playback.effects.writer() {
                    store.run(write);
                }
            }
            Kept::Lists => self.save_now(),
            Kept::Window => {
                file(super::settings::window_state::file(), super::settings::window_state::text(&self.window_state))
            }
        }
    }
}
