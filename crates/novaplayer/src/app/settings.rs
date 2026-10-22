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
