//! The player's volume, kept between runs (b7b33f5's `kept_gain`).
//!
//! Two numbers: the level playing (0 is muted) and the level unmuting goes back to, the last one
//! above zero. Kept in `%LOCALAPPDATA%\NovaPlayer\volume.txt` as `volume 0.350` and
//! `unmuted 0.700`, read at start before any sound opens, and kept (`app::kept`) a second after
//! it changes and at exit.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gain {
    pub level: f32,
    pub unmuted: f32,
}

impl Gain {
    pub const FULL: Self = Gain { level: 1.0, unmuted: 1.0 };

    /// At `level`; unmuting goes back to it from here on, when it is not a mute.
    pub fn at(self, level: f32) -> Self {
        let level = level.clamp(0.0, 1.0);
        Gain { level, unmuted: if level > 0.0 { level } else { self.unmuted } }
    }

    pub fn text(self) -> String {
        format!("volume {:.3}\nunmuted {:.3}\n", self.level, self.unmuted)
    }

    /// What `text` keeps; anything missing or out of range is the full gain's.
    pub fn parse(text: &str) -> Self {
        let mut g = Self::FULL;
        for line in text.lines() {
            let value = |name: &str| {
                line.strip_prefix(name).and_then(|r| r.trim().parse::<f32>().ok()).filter(|v| v.is_finite())
            };
            if let Some(v) = value("volume ") {
                g.level = v.clamp(0.0, 1.0);
            } else if let Some(v) = value("unmuted ") {
                g.unmuted = v.clamp(0.0, 1.0);
            }
        }
        // Unmuting always goes somewhere that can be heard.
        if g.unmuted <= 0.0 {
            g.unmuted = if g.level > 0.0 { g.level } else { 1.0 };
        }
        g
    }
}

pub struct KeptGain {
    pub gain: Gain,
    file: Option<PathBuf>,
    written: Gain,
}

impl KeptGain {
    pub fn load(file: Option<PathBuf>) -> Self {
        let gain =
            file.as_deref().and_then(|f| std::fs::read_to_string(f).ok()).map_or(Gain::FULL, |t| Gain::parse(&t));
        KeptGain { gain, file, written: gain }
    }

    pub fn player() -> Self {
        Self::load(crate::data_dir().map(|d| d.join("volume.txt")))
    }

    /// Set to `level`.
    pub fn set(&mut self, level: f32) {
        self.gain = self.gain.at(level);
    }

    /// The file and its text, when the gain is not as last written (it counts as written).
    pub fn unwritten(&mut self) -> Option<(PathBuf, String)> {
        if self.gain == self.written {
            return None;
        }
        self.written = self.gain;
        Some((self.file.clone()?, self.gain.text()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_remembers_the_level_to_return_to() {
        let g = Gain::FULL.at(0.35).at(0.0);
        assert_eq!(g, Gain { level: 0.0, unmuted: 0.35 });
        assert_eq!(Gain::parse(&g.text()), g);
    }

    #[test]
    fn bad_text_is_full_and_a_zero_unmute_is_audible() {
        assert_eq!(Gain::parse("volume x\nunmuted 7"), Gain { level: 1.0, unmuted: 1.0 });
        assert_eq!(Gain::parse("volume 0\nunmuted 0"), Gain { level: 0.0, unmuted: 1.0 });
    }

    #[test]
    fn written_once_per_change() {
        let dir = std::env::temp_dir().join(format!("np-gain-{}", std::process::id()));
        let f = dir.join("volume.txt");
        let mut k = KeptGain::load(Some(f.clone()));
        k.set(0.5);
        k.set(0.4);
        let (file, text) = k.unwritten().expect("changed");
        assert_eq!(
            text,
            "volume 0.400
unmuted 0.400
"
        );
        assert_eq!(k.unwritten(), None);
        k.set(0.0);
        let (_, text) = k.unwritten().expect("muted");
        crate::files::write_atomic(&file, text.as_bytes()).expect("written");
        assert_eq!(KeptGain::load(Some(f)).gain, Gain { level: 0.0, unmuted: 0.4 });
        let _ = std::fs::remove_dir_all(dir);
    }
}
