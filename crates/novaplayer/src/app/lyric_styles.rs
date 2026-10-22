//! Lyric style packages (`docs/extensions.md`, "Lyric styles"): a folder holding `style.json` and the style's
//! `.nui`, installed under `<data dir>/lyric-styles/<id>/` from a folder or a package (zip,
//! tar.gz) as a plugin is. The choice is the setting `lyric-style` (`settings_page::SETTINGS`);
//! the built-in style is the default and always there.
//!
//! ```json
//! { "id": "one-line", "name": { "en": "One line", "zh-CN": "单行" }, "style": "style.nui" }
//! ```
//!
//! `name` is a text or one per language (the first by tag when none is the player's); `style` a file in
//! the folder (`style.nui` when not named). Nothing here runs: a style is a `.nui` fragment the
//! style profile admits (`side_panel::STYLE_PROFILE`), checked when it is installed and again
//! when it is used.
use novaplayer_plugin_protocol::json;
use std::path::{Path, PathBuf};

/// The most a style's `.nui` may be (the profile's own limit, read no further).
const MAX_STYLE: u64 = 16 * 1024;

/// An installed style.
#[derive(Clone, Debug)]
pub struct Style {
    pub id: String,
    /// Its name in the player's language.
    pub name: String,
    /// Its `.nui`, inside its folder.
    file: PathBuf,
}

impl Style {
    /// Its `.nui`, read.
    pub fn source(&self) -> Result<String, String> {
        let len = std::fs::metadata(&self.file).map_err(|e| e.to_string())?.len();
        if len > MAX_STYLE {
            return Err(novaui::i18n::t("err.style-big", &[]));
        }
        std::fs::read_to_string(&self.file).map_err(|e| e.to_string())
    }
}

/// Where styles are installed (tests use a temporary folder of their own).
pub fn root() -> Option<PathBuf> {
    crate::data_dir().map(|d| d.join("lyric-styles"))
}

/// The styles installed under `root`, by name.
pub fn installed(root: &Path, locale: &str) -> Vec<Style> {
    let mut out: Vec<Style> =
        std::fs::read_dir(root).into_iter().flatten().flatten().filter_map(|e| read(&e.path(), locale).ok()).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    out
}

/// The style `id` installed under `root`.
pub fn find(root: &Path, id: &str, locale: &str) -> Option<Style> {
    valid_id(id).then(|| read(&root.join(id), locale).ok()).flatten()
}

/// The style a folder holds (its `style.json`).
fn read(dir: &Path, locale: &str) -> Result<Style, String> {
    let text =
        std::fs::read_to_string(dir.join("style.json")).map_err(|_| novaui::i18n::t("err.no-style-json", &[]))?;
    let v = json::parse(&text).map_err(|e| novaui::i18n::t("err.style-json", &[("e", &e.to_string())]))?;
    let id = v.get("id").and_then(json::Value::as_str).unwrap_or_default().to_owned();
    if !valid_id(&id) {
        return Err(novaui::i18n::t("err.bad-id", &[("id", &id)]));
    }
    let name = match v.get("name") {
        Some(json::Value::Str(s)) => s.clone(),
        Some(json::Value::Obj(names)) => {
            names.get(locale).or_else(|| names.values().next()).and_then(json::Value::as_str).unwrap_or(&id).to_owned()
        }
        _ => id.clone(),
    };
    let file = v.get("style").and_then(json::Value::as_str).unwrap_or("style.nui");
    if !crate::plugins::registry::inside_folder(file) {
        return Err(novaui::i18n::t("err.style-path", &[]));
    }
    Ok(Style { id, name, file: dir.join(file) })
}

/// Installs under `root` the style a folder or package holds (replacing one with its id), once
/// its `.nui` passes the style profile. Its id and name.
pub fn install(root: &Path, from: &Path, locale: &str) -> Result<(String, String), String> {
    if from.is_dir() {
        return install_dir(root, from, locale);
    }
    let tmp = std::env::temp_dir().join(format!(
        "novaplayer-style-{}-{}",
        std::process::id(),
        crate::plugins::registry::stamp()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    let r = (|| {
        crate::plugins::registry::archive::extract(from, &tmp)?;
        let dir = crate::plugins::registry::archive::holding(&tmp, "style.json")
            .ok_or_else(|| novaui::i18n::t("err.no-style-json", &[]))?;
        install_dir(root, &dir, locale)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn install_dir(root: &Path, from: &Path, locale: &str) -> Result<(String, String), String> {
    let s = read(from, locale)?;
    crate::views::side_panel::parse_style(&s.source()?).map_err(|e| novaui::i18n::t("err.style-bad", &[("e", &e)]))?;
    let to = root.join(&s.id);
    if to == from {
        return Ok((s.id, s.name));
    }
    let _ = std::fs::remove_dir_all(&to);
    crate::plugins::registry::copy_dir(from, &to)
        .map_err(|e| novaui::i18n::t("err.copy-failed", &[("e", &e.to_string())]))?;
    Ok((s.id, s.name))
}

/// Removes the style `id` installed under `root`: only ever a folder directly under it.
pub fn remove(root: &Path, id: &str) -> Result<(), String> {
    if !valid_id(id) || !root.join(id).is_dir() {
        return Err(novaui::i18n::t("err.no-such-style", &[]));
    }
    std::fs::remove_dir_all(root.join(id)).map_err(|e| novaui::i18n::t("err.delete-failed", &[("e", &e.to_string())]))
}

/// A folder name of ours: letters, digits, `-_.`, not leading with a dot (as a plugin's id).
use crate::plugins::registry::valid_id;

#[cfg(test)]
mod tests {
    /// The example package installs from its folder, is listed by its name in the player's
    /// language, passes the profile, and is removed again; a style outside the profile is refused.
    #[test]
    fn the_example_installs_lists_and_removes() {
        let tmp = std::env::temp_dir().join(format!("np-styles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("lyric-styles");
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lyric-styles/one-line");
        let installed = super::install(&root, &example, "zh-CN").unwrap();
        assert_eq!(installed, ("one-line".to_owned(), "单行".to_owned()));
        let list = super::installed(&root, "en");
        assert_eq!(
            list.iter().map(|s| (s.id.as_str(), s.name.as_str())).collect::<Vec<_>>(),
            [("one-line", "One line")]
        );
        assert!(super::find(&root, "one-line", "en").unwrap().source().unwrap().contains("LyricLine"));
        assert!(super::find(&root, "../one-line", "en").is_none());
        // A style the profile refuses (an element it does not admit) is not installed.
        let bad = tmp.join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("style.json"), r#"{ "id": "bad" }"#).unwrap();
        std::fs::write(bad.join("style.nui"), "component LyricLine(line) { Image { } }").unwrap();
        assert!(super::install(&root, &bad, "en").is_err());
        assert!(super::find(&root, "bad", "en").is_none());
        super::remove(&root, "one-line").unwrap();
        assert!(super::installed(&root, "en").is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
