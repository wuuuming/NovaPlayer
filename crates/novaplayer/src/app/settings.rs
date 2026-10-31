//! Small settings files.

/// The settings page's choices (novaplayer-spec.md, "Settings page"): `NovaPlayer/settings.txt`,
/// `key value` lines. A rewrite keeps every line it does not set, in its place, so a key this
/// build does not know survives a newer build's file.
#[derive(Default)]
pub struct SettingsFile {
    path: Option<std::path::PathBuf>,
    lines: Vec<(String, String)>,
}

impl SettingsFile {
    /// The file in the data dir (none: nothing is kept).
    pub fn load() -> Self {
        Self::at(crate::data_dir().map(|d| d.join("settings.txt")))
    }

    pub fn at(path: Option<std::path::PathBuf>) -> Self {
        let text = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        let lines = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| match l.split_once(char::is_whitespace) {
                Some((k, v)) => (k.to_owned(), v.trim().to_owned()),
                None => (l.to_owned(), String::new()),
            })
            .collect();
        SettingsFile { path, lines }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Sets `key`; false: it was so already. The file is the caller's to keep
    /// (`Player::set_setting`).
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        if self.get(key) == Some(value) {
            return false;
        }
        match self.lines.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value.to_owned(),
            None => self.lines.push((key.to_owned(), value.to_owned())),
        }
        true
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    /// The file's text.
    pub fn text(&self) -> String {
        self.lines.iter().map(|(k, v)| format!("{k} {v}\n")).collect()
    }
}

impl super::Player {
    /// Sets a setting (`settings.txt`) and keeps it.
    /// A setting is chosen: put into effect and named in its box (`dropdown::apply`), and kept
    /// (`None`: the setting's default, kept as empty).
    pub fn choose_setting(&mut self, ui: &mut novaui::Ui, key: &str, value: Option<&str>) {
        self.views(|v, _| crate::views::menus::dropdown::apply(ui, v, key, value));
        self.set_setting(key, value.unwrap_or(""));
    }

    pub fn set_setting(&mut self, key: &str, value: &str) {
        if self.settings.set(key, value) {
            self.keeping.changed(super::kept::Kept::Settings);
        }
    }
}

/// Lyrics layers switched off, kept in `NovaPlayer/lyrics-layers.txt`, a layer id a line
/// (a layer not listed is on). Older files kept `plugin/layer`: read as the layer.
pub mod layer_setting {
    use std::collections::BTreeSet;

    pub fn file() -> Option<std::path::PathBuf> {
        crate::data_dir().map(|d| d.join("lyrics-layers.txt"))
    }

    pub fn get() -> BTreeSet<String> {
        file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| {
                t.lines()
                    .filter_map(|l| l.trim().rsplit('/').next())
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The file's text (`app::kept`).
    pub fn text(off: &BTreeSet<String>) -> String {
        off.iter().map(|l| format!("{l}\n")).collect()
    }
}

/// Where the window and its side panel were when the player last ran (`NovaPlayer/window.txt`,
/// `key value` lines as `settings.txt`): the window's bounds (novaui's, kept as handed out), the
/// side panel open or folded, its page, and the queue's top row. A second-level page (a search,
/// a narrowing, the song's info) and the settings page are not kept: the player opens on the
/// first level.
pub mod window_state {
    use crate::views::side_panel::Page;
    use novaui::platform::Bounds;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct WindowState {
        pub bounds: Option<Bounds>,
        pub panel_open: bool,
        pub page: Page,
        /// The queue's top row (rows from the list's start; a fraction is a row part scrolled).
        pub queue_top: f32,
    }

    pub fn file() -> Option<std::path::PathBuf> {
        crate::data_dir().map(|d| d.join("window.txt"))
    }

    /// The kept state; `None`: none kept (the first run).
    pub fn load() -> Option<WindowState> {
        let f = super::SettingsFile::at(Some(file()?));
        let panel_open = match f.get("panel")? {
            "open" => true,
            "folded" => false,
            _ => return None,
        };
        Some(WindowState {
            bounds: f.get("bounds").and_then(Bounds::parse),
            panel_open,
            page: if f.get("page") == Some("lyrics") { Page::Lyrics } else { Page::Queue },
            queue_top: f
                .get("queue-top")
                .and_then(|v| v.parse().ok())
                .filter(|v: &f32| v.is_finite())
                .unwrap_or(0.0)
                .max(0.0),
        })
    }

    /// The file's text (`app::kept`).
    pub fn text(s: &WindowState) -> String {
        let mut out = String::new();
        if let Some(b) = s.bounds {
            out += &format!("bounds {}\n", b.text());
        }
        out += &format!("panel {}\n", if s.panel_open { "open" } else { "folded" });
        out += &format!("page {}\n", if s.page == Page::Lyrics { "lyrics" } else { "queue" });
        out += &format!("queue-top {:.2}\n", s.queue_top);
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// What is written is read back as it was; a file of another shape is no state.
        #[test]
        fn window_state_round_trips() {
            let dir = std::env::temp_dir().join(format!("np-window-state-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("window.txt");
            let s = WindowState {
                bounds: Some(Bounds { x: -2460, y: 150, width: 1350, height: 960, maximized: true }),
                panel_open: true,
                page: Page::Lyrics,
                queue_top: 12.5,
            };
            std::fs::write(&path, text(&s)).unwrap();
            let f = crate::app::settings::SettingsFile::at(Some(path.clone()));
            assert_eq!(f.get("bounds"), Some("-2460 150 1350 960 1"));
            assert_eq!(Bounds::parse(f.get("bounds").unwrap()), s.bounds);
            assert_eq!(f.get("queue-top"), Some("12.50"));
            assert_eq!(Bounds::parse("1 2 0 4 0"), None, "no size, no bounds");
            assert_eq!(Bounds::parse("1 2 3"), None);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
