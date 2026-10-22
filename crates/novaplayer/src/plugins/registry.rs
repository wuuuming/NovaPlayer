//! Installed plugins: what is on disk under `<data>/NovaPlayer/plugins/<id>/plugin.json`,
//! and which of them the user has enabled.
//!
//! A plugin is a program the user chose to run (an executable plus its manifest), with the
//! user's own rights: there is no sandbox. So none runs until it is enabled in the plugin
//! flyout, after its declared capabilities were shown; enabling records a hash of its files
//! (`prints`), and a plugin whose files changed since is disabled again until confirmed.
//!
//! This module only knows the manifests and the enabled set. Running plugins (the JSON-lines
//! protocol, `docs/plugins.md`) is the plugin host's job.

use super::prints::Prints;
use novaplayer_plugin_protocol::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use novaplayer_plugin_protocol::API;

/// What a plugin declares it does (the flyout shows these before it is enabled).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provides {
    /// File extensions it decodes (lower case, no dot).
    pub decode: Vec<String>,
    /// Metadata for the songs it resolves (never for local files).
    pub metadata: bool,
    /// Small covers for many of its songs at once (`thumbs`): a list's thumbnails prefetched.
    pub thumbs: bool,
    /// URI schemes of the songs it resolves (`resolve`): the entries of its M3U8 lists.
    pub schemes: Vec<String>,
    /// Link schemes / hosts it turns into an M3U8 playlist (`playlist`).
    pub links: Vec<String>,
    /// The source can be connected to an account or a server (`source.connect`): the player
    /// draws the panel and keeps the credential in its vault.
    pub connect: Option<Connect>,
    /// The source lists the connected user's own playlists (`source.lists`).
    pub lists: bool,
    /// One choice its songs come in (id, name: "quality", "音质"): `options`, `resolve`'s
    /// `option`.
    pub option: Option<(String, String)>,
    /// The template of the line under the artwork for its songs: `{res}` the stream's own
    /// resolution, `{option}` the chosen value's name.
    pub style_res: Option<String>,
}

/// What a source asks the user for in a `form` flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Url,
    /// Typed masked, never kept by the player.
    Password,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
}

/// `provides.source.connect`: how a source is connected, in its order of preference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Connect {
    /// `qr`, `url`, `code` or `form`.
    pub flows: Vec<String>,
    /// The fields of the `form` flow (non-empty exactly when `form` is a flow).
    pub fields: Vec<Field>,
}

/// The flows a manifest may offer (`provides.source.connect.flows`): all the panel can show.
pub const FLOWS: &[&str] = &["qr", "url", "code", "form"];
/// At most this many form fields (the panel is a small flyout).
const MAX_FIELDS: usize = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: u32,
    /// The program, relative to the plugin's folder.
    pub exec: String,
    pub provides: Provides,
    /// Hosts it says it reaches (shown; not enforced).
    pub network: Vec<String>,
    /// It answers overlapping calls by `id`, in any order (`docs/plugins.md` §The pipe). Without
    /// it the host sends one call at a time.
    pub concurrent: bool,
    /// Controls it adds to the player's face (`contributes.controls`, controls.rs), or why they
    /// could not be read (the plugin's other capabilities stay).
    pub controls: Result<Vec<crate::plugins::controls::Control>, String>,
    /// Its own texts (`strings`): locale → (key, text). The registry puts them in the process's
    /// table under `id:` when it lists the plugin (`Registry::scan`).
    pub strings: Vec<(String, Vec<(String, String)>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Installed, not enabled.
    Off,
    /// Enabled and unchanged since.
    On,
    /// Was enabled, but its files changed: off until confirmed again.
    Changed,
    /// Was enabled; its files are being checked against what was confirmed (a worker hashes
    /// them, `prints`). Off until then.
    Checking,
    /// Cannot run: why (bad manifest, other protocol version, missing program).
    Broken(String),
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub dir: PathBuf,
    /// `None` when the manifest could not be read (then `state` says why).
    pub manifest: Option<Manifest>,
    pub state: State,
}

impl Plugin {
    /// It is on: its program runs (it is called).
    pub fn is_on(&self) -> bool {
        self.state == State::On
    }

    pub fn id(&self) -> String {
        self.manifest.as_ref().map_or_else(
            || self.dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            |m| m.id.clone(),
        )
    }
    pub fn name(&self) -> String {
        self.manifest.as_ref().map_or_else(|| self.id(), Manifest::name)
    }
    /// One line for the flyout: what it does.
    pub fn summary(&self) -> String {
        let Some(m) = &self.manifest else { return String::new() };
        let p = &m.provides;
        let mut s = Vec::new();
        if !p.decode.is_empty() {
            s.push(novaui::i18n::t("plugin.does.decode", &[("formats", &p.decode.join("/"))]));
        }
        if p.metadata {
            s.push(novaui::i18n::t("plugin.does.metadata", &[]));
        }
        if !p.links.is_empty() {
            s.push(novaui::i18n::t("plugin.does.links", &[]));
        }
        if let Some((_, n)) = &p.option {
            s.push(novaui::i18n::t("plugin.does.option", &[("what", n)]));
        }
        if p.connect.is_some() {
            s.push(novaui::i18n::t("plugin.does.connect", &[]));
        }
        match &m.controls {
            Ok(c) if !c.is_empty() => s.push(novaui::i18n::t("plugin.does.controls", &[])),
            Err(why) => s.push(novaui::i18n::t("plugin.does.controls-bad", &[("why", why)])),
            _ => {}
        }
        s.join(" · ")
    }
}

pub struct Registry {
    pub root: Option<PathBuf>,
    pub list: Vec<Plugin>,
    /// id → fingerprint of its files when it was enabled.
    enabled: BTreeMap<String, String>,
    prints: Prints,
}

impl Registry {
    #[cfg(any(test, feature = "test-support"))]
    pub fn open(root: Option<PathBuf>) -> Self {
        Self::open_watched(root, || {})
    }

    /// Opens the registry; `checked` is called (on a worker) each time a plugin's files have
    /// been hashed, from the first scan on (the app scans again, `Msg::PluginsChecked`).
    pub fn open_watched(root: Option<PathBuf>, checked: impl Fn() + Send + Sync + 'static) -> Self {
        let prints = Prints::open(root.as_deref());
        prints.on_done(checked);
        let mut r = Registry { root, list: Vec::new(), enabled: BTreeMap::new(), prints };
        r.enabled = r.read_enabled();
        r.scan();
        r
    }

    fn enabled_file(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join("enabled.txt"))
    }

    fn read_enabled(&self) -> BTreeMap<String, String> {
        read_enabled(self.root.as_deref())
    }

    fn write_enabled(&self) {
        let Some(f) = self.enabled_file() else { return };
        let text: String = self.enabled.iter().map(|(id, h)| format!("{id}\t{h}\n")).collect();
        let _ = crate::files::write_atomic(&f, text.as_bytes());
    }

    /// Re-reads the plugin folders (cheap: one small file each). What follows a change is done
    /// here, once, whoever scanned: the host runs the enabled plugins, the face cache knows
    /// their builds. True when what is listed changed (the app's views and controls follow,
    /// `Player::plugins_changed`).
    pub fn scan(&mut self) -> bool {
        let list = self.scanner().read();
        self.adopt(list)
    }

    /// What reads the plugin folders, on any thread (a task's: a folder on a slow disk does not
    /// hold the UI thread); its list goes to `adopt`.
    pub fn scanner(&self) -> Scanner {
        Scanner { root: self.root.clone(), enabled: self.enabled.clone(), prints: self.prints.clone() }
    }

    /// Takes a scan's list (`scanner`): as `scan`.
    pub fn adopt(&mut self, list: Vec<Plugin>) -> bool {
        let seen = |l: &[Plugin]| -> Vec<(PathBuf, State, Option<String>)> {
            l.iter().map(|p| (p.dir.clone(), p.state.clone(), p.manifest.as_ref().map(|m| m.version.clone()))).collect()
        };
        let changed = seen(&list) != seen(&self.list);
        // A plugin's own texts (`id:key`), for its views and its controls.
        for m in list.iter().filter_map(|p| p.manifest.as_ref()) {
            m.add_strings();
        }
        self.list = list;
        if changed {
            crate::plugins::pcm::sync(self);
            crate::plugins::client::set_stamps(self);
        }
        changed
    }
}

/// A registry's reading of its folders (`Registry::scanner`), sent to a task.
pub struct Scanner {
    root: Option<PathBuf>,
    enabled: BTreeMap<String, String>,
    prints: Prints,
}

impl Scanner {
    /// The plugin folders as they are now.
    pub fn read(&self) -> Vec<Plugin> {
        let mut list = Vec::new();
        let Some(root) = &self.root else { return list };
        // Not the folders an install stages in (`.<id>.new`, `.<id>.old`).
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let plugin = match read_manifest(&dir) {
                Err(why) => Plugin { dir, manifest: None, state: State::Broken(why) },
                Ok(m) => {
                    let state = if m.api != API {
                        State::Broken(novaui::i18n::t(
                            "plugin.bad-api",
                            &[("want", &m.api.to_string()), ("have", &API.to_string())],
                        ))
                    } else if !dir.join(&m.exec).is_file() {
                        State::Broken(novaui::i18n::t("plugin.no-exec", &[("exec", &m.exec)]))
                    } else {
                        // Not hashed here: a plugin whose files are not as last hashed is
                        // hashed on a worker (an Off one too, so enabling it finds it known).
                        let known = self.prints.known(&dir);
                        if known.is_none() {
                            self.prints.check(&dir);
                        }
                        match (self.enabled.get(&m.id), known) {
                            (None, _) => State::Off,
                            (Some(_), None) => State::Checking,
                            (Some(h), Some(now)) if *h == now => State::On,
                            (Some(_), Some(_)) => State::Changed,
                        }
                    };
                    Plugin { dir, manifest: Some(m), state }
                }
            };
            list.push(plugin);
        }
        list
    }
}

impl Registry {
    /// What enabling `id` needs: its folder and the fingerprints that know it (`Prints::now`
    /// hashes it when its files are not as last hashed: a task's work). None: it cannot run.
    pub fn enabling(&self, id: &str) -> Option<(PathBuf, Prints)> {
        let p = self.get(id)?;
        (p.manifest.is_some() && !matches!(p.state, State::Broken(_))).then(|| (p.dir.clone(), self.prints.clone()))
    }

    /// Enables `id` (the user confirmed its capabilities) as its files are now: `print`.
    pub fn enable(&mut self, id: &str, print: String) {
        self.enabled.insert(id.to_owned(), print);
        self.write_enabled();
    }

    /// Disables `id`: what was confirmed for it is forgotten.
    pub fn disable(&mut self, id: &str) {
        self.enabled.remove(id);
        self.write_enabled();
    }

    /// Enables or disables a plugin, hashing and scanning here (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_enabled(&mut self, id: &str, on: bool) -> bool {
        if on {
            let Some((dir, prints)) = self.enabling(id) else { return false };
            self.enable(id, prints.now(&dir));
        } else {
            self.disable(id);
        }
        self.scan();
        true
    }

    /// The folder deleting `id` removes (`remove_files`): only ever one directly under the
    /// plugins folder.
    pub fn removal(&self, id: &str) -> Result<PathBuf, String> {
        let dir = self.get(id).map(|p| p.dir.clone()).ok_or_else(|| novaui::i18n::t("err.no-such-plugin", &[]))?;
        if dir.parent() != self.root.as_deref() {
            return Err(novaui::i18n::t("err.not-in-plugins", &[]));
        }
        Ok(dir)
    }

    /// Installs a plugin here (tests): as the app's task (`install_package`), then `disable`
    /// (a reinstall is a change: confirm again) and `scan`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn install_package(&mut self, from: &Path) -> Result<String, String> {
        let root = self.root.clone().ok_or_else(|| novaui::i18n::t("err.no-data-dir", &[]))?;
        let (id, name) = install_package(&root, from)?;
        self.disable(&id);
        self.scan();
        Ok(name)
    }

    /// What a reload reads (`reloaded`), as a task: what is enabled, and the folders.
    pub fn reloader(&self) -> impl FnOnce() -> (BTreeMap<String, String>, Vec<Plugin>) + Send + 'static {
        let mut scanner = self.scanner();
        move || {
            scanner.enabled = read_enabled(scanner.root.as_deref());
            let list = scanner.read();
            (scanner.enabled, list)
        }
    }

    /// A reload done here (the screenshot suites, Windows only): as the app's task, then
    /// `reloaded`.
    #[cfg(all(feature = "test-support", windows))]
    pub fn reload(&mut self) {
        let (enabled, list) = self.reloader()();
        self.reloaded(enabled, list);
    }

    /// 重新加载插件: the folder as read again (`reloader`), every running plugin stopped (the
    /// next call starts the program now on disk).
    pub fn reloaded(&mut self, enabled: BTreeMap<String, String>, list: Vec<Plugin>) {
        self.enabled = enabled;
        self.adopt(list);
        crate::plugins::pcm::reload(self);
        crate::plugins::client::set_stamps(self);
    }

    /// Tests: a plugin as if installed and enabled, with what it provides set by `f`.
    #[cfg(all(feature = "test-support", windows))] // the screenshot suites (Windows only)
    pub fn force_on_for_test(&mut self, id: &str, f: impl FnOnce(&mut Provides)) {
        let mut provides = Provides::default();
        f(&mut provides);
        let manifest = Manifest {
            id: id.into(),
            name: id.into(),
            version: "1".into(),
            api: API,
            exec: String::new(),
            network: Vec::new(),
            concurrent: false,
            provides,
            controls: Ok(Vec::new()),
            strings: Vec::new(),
        };
        self.list.retain(|p| p.id() != id);
        self.list.push(Plugin { dir: PathBuf::new(), manifest: Some(manifest), state: State::On });
    }

    pub fn get(&self, id: &str) -> Option<&Plugin> {
        self.list.iter().find(|p| p.id() == id)
    }

    /// Plugin `id`'s manifest, when there is such a plugin and its manifest reads.
    pub fn manifest(&self, id: &str) -> Option<&Manifest> {
        self.get(id)?.manifest.as_ref()
    }

    /// Plugin `id`'s template for the line under the artwork (`style.res`), when it has one.
    pub fn style_res(&self, id: &str) -> Option<String> {
        self.manifest(id)?.provides.style_res.clone()
    }

    /// How many plugins are on.
    pub fn on_count(&self) -> usize {
        self.list.iter().filter(|x| x.is_on()).count()
    }

    /// What plugin `id` is called on screen (empty when there is none).
    pub fn name_of(&self, id: &str) -> String {
        self.get(id).map(|x| x.name()).unwrap_or_default()
    }

    /// Tests: the fingerprint workers finished, the folder scanned again.
    #[cfg(any(test, feature = "test-support"))]
    pub fn settle(&mut self) {
        self.prints.settle();
        self.scan();
    }
}

/// What is enabled in the plugins folder `root` (`enabled.txt`: id, fingerprint).
fn read_enabled(root: Option<&Path>) -> BTreeMap<String, String> {
    let Some(f) = root.map(|r| r.join("enabled.txt")) else { return BTreeMap::new() };
    std::fs::read_to_string(f)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(id, h)| (id.to_owned(), h.to_owned()))
        .collect()
}

/// Stops plugin `id` and deletes its folder `dir` (`Registry::removal`). A task's work; the
/// caller `disable`s it and scans after. (Its stored credential is the caller's: the vault is
/// not the registry's.)
pub fn remove_files(id: &str, dir: &Path) -> Result<(), String> {
    crate::plugins::pcm::host().unregister(id);
    std::fs::remove_dir_all(dir).map_err(|e| novaui::i18n::t("err.delete-failed", &[("e", &e.to_string())]))
}

/// Installs a plugin into the plugins folder `root` from its package as distributed: a `.zip`,
/// `.tgz` / `.tar.gz` (or a folder holding `plugin.json`). The package's `plugin.json` may sit at
/// its top or in its one top folder. Installed off: enabling is a separate, confirmed step. A
/// task's work: its id and name; the caller `disable`s it (a reinstall is a change) and scans.
pub fn install_package(root: &Path, from: &Path) -> Result<(String, String), String> {
    if from.is_dir() {
        return install(root, from);
    }
    let tmp = std::env::temp_dir().join(format!("novaplayer-plugin-{}-{}", std::process::id(), stamp()));
    let _ = std::fs::remove_dir_all(&tmp);
    let r = (|| {
        archive::extract(from, &tmp)?;
        let dir = archive::holding(&tmp, "plugin.json").ok_or_else(|| novaui::i18n::t("err.no-manifest", &[]))?;
        install(root, &dir)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

/// Copies a plugin folder (one holding `plugin.json`) into the plugins folder. Installed
/// off: enabling is a separate, confirmed step.
///
/// Over an installed one it is staged beside it (`.<id>.new`), the old one's process
/// stopped (Windows keeps a running program's file locked), and the two swapped by renaming:
/// a copy that fails leaves the installed plugin as it was.
fn install(root: &Path, from: &Path) -> Result<(String, String), String> {
    let m = read_manifest(from)?;
    if !valid_id(&m.id) {
        return Err(novaui::i18n::t("err.bad-id", &[("id", &m.id)]));
    }
    let to = root.join(&m.id);
    if to == from {
        return Err(novaui::i18n::t("err.in-plugins", &[]));
    }
    let failed = |e: std::io::Error| novaui::i18n::t("err.copy-failed", &[("e", &e.to_string())]);
    let (new, old) = (root.join(format!(".{}.new", m.id)), root.join(format!(".{}.old", m.id)));
    let _ = std::fs::remove_dir_all(&new);
    let _ = std::fs::remove_dir_all(&old);
    if let Err(e) = copy_dir(from, &new) {
        let _ = std::fs::remove_dir_all(&new);
        return Err(failed(e));
    }
    // The program runs: on Linux it must be executable whatever the package kept.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(new.join(&m.exec), std::fs::Permissions::from_mode(0o755));
    }
    crate::plugins::pcm::host().unregister(&m.id);
    let swapped = (|| {
        if to.exists() {
            std::fs::rename(&to, &old)?;
        }
        std::fs::rename(&new, &to).inspect_err(|_| {
            let _ = std::fs::rename(&old, &to);
        })
    })();
    let _ = std::fs::remove_dir_all(&new);
    let _ = std::fs::remove_dir_all(&old);
    swapped.map_err(failed)?;
    Ok((m.id.clone(), m.name()))
}

/// A number unique enough for a temporary folder's name.
pub(crate) fn stamp() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

/// Plugin packages: `.zip` (stored or deflated entries) and gzip'd tar. Every entry is written
/// under the target folder only (no absolute paths, no `..`, no links), at most 4096 entries
/// and 512 MiB in all.
pub(crate) mod archive {
    /// The packages `extract` reads, as a dialog filter.
    pub(crate) const PICK: &str = "*.zip;*.tgz;*.tar.gz";

    use std::path::{Component, Path, PathBuf};

    const MAX_TOTAL: usize = 512 << 20;
    const MAX_ENTRIES: usize = 4096;

    pub fn extract(file: &Path, to: &Path) -> Result<(), String> {
        let data = std::fs::read(file).map_err(|e| novaui::i18n::t("err.pkg-read", &[("e", &e.to_string())]))?;
        std::fs::create_dir_all(to).map_err(|e| novaui::i18n::t("err.pkg-unpack", &[("e", &e.to_string())]))?;
        if data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06") {
            zip(&data, to)
        } else if data.starts_with(&[0x1f, 0x8b]) {
            tar(&gunzip(&data)?, to)
        } else if data.len() > 262 && &data[257..262] == b"ustar" {
            tar(&data, to)
        } else {
            Err(novaui::i18n::t("err.pkg-kind", &[]))
        }
    }

    /// The folder holding `name` (`plugin.json`, a lyric style's `style.json`): the package's
    /// top, else the shallowest one below.
    pub fn holding(root: &Path, name: &str) -> Option<PathBuf> {
        let mut level = vec![root.to_path_buf()];
        for _ in 0..4 {
            let mut next = Vec::new();
            for d in &level {
                if d.join(name).is_file() {
                    return Some(d.clone());
                }
                next.extend(
                    std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()),
                );
            }
            next.sort();
            level = next;
        }
        None
    }

    /// A name Windows reads as something other than a file: a device (`CON`, `nul.txt`, `COM1`
    /// …, whatever the extension), or one it would cut short (a trailing dot or space).
    fn windows_reserved(part: &str) -> bool {
        let stem = part.split('.').next().unwrap_or(part).trim_end().to_ascii_uppercase();
        let numbered = ["COM", "LPT"].iter().find_map(|p| stem.strip_prefix(p)).is_some_and(|n| {
            (n.len() == 1 && n.bytes().all(|b| b.is_ascii_digit())) || matches!(n, "\u{b9}" | "\u{b2}" | "\u{b3}")
        });
        let device = numbered || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$");
        device || part.ends_with(['.', ' '])
    }

    /// An entry's name made a path under `to`, or None when it would leave it. Judged the same on
    /// every system: a package made on one is opened on another (`C:/x` is a drive on Windows, a
    /// folder named `C:` elsewhere; both are refused, as is any `:` a Windows path would read,
    /// and any part Windows reads as a device).
    fn safe(to: &Path, name: &str) -> Option<PathBuf> {
        let name = name.replace('\\', "/");
        if name.starts_with('/') || name.contains(':') {
            return None;
        }
        let rel = Path::new(name.trim_start_matches("./"));
        let mut out = to.to_path_buf();
        let mut any = false;
        for c in rel.components() {
            match c {
                Component::Normal(p) if p.to_str().is_some_and(windows_reserved) => return None,
                Component::Normal(p) => {
                    out.push(p);
                    any = true;
                }
                Component::CurDir => {}
                _ => return None,
            }
        }
        any.then_some(out)
    }

    fn write(to: &Path, name: &str, bytes: &[u8], total: &mut usize) -> Result<(), String> {
        *total += bytes.len();
        if *total > MAX_TOTAL {
            return Err(novaui::i18n::t("err.pkg-large", &[]));
        }
        let p = safe(to, name).ok_or_else(|| novaui::i18n::t("err.pkg-path", &[("path", name)]))?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| novaui::i18n::t("err.pkg-unpack", &[("e", &e.to_string())]))?;
        }
        std::fs::write(&p, bytes).map_err(|e| novaui::i18n::t("err.pkg-unpack", &[("e", &e.to_string())]))
    }

    fn u16le(b: &[u8], at: usize) -> Option<usize> {
        Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize)
    }

    fn u32le(b: &[u8], at: usize) -> Option<usize> {
        Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
    }

    fn inflate(raw: &[u8], size: usize) -> Result<Vec<u8>, String> {
        miniz_oxide::inflate::decompress_to_vec_with_limit(raw, size.clamp(1, MAX_TOTAL))
            .map_err(|_| novaui::i18n::t("err.pkg-damaged", &[]))
    }

    /// Code page 437's upper half: a zip entry's name is in it unless the entry says UTF-8.
    const CP437_HIGH: &str = "\u{c7}\u{fc}\u{e9}\u{e2}\u{e4}\u{e0}\u{e5}\u{e7}\u{ea}\u{eb}\u{e8}\u{ef}\u{ee}\u{ec}\u{c4}\u{c5}\
        \u{c9}\u{e6}\u{c6}\u{f4}\u{f6}\u{f2}\u{fb}\u{f9}\u{ff}\u{d6}\u{dc}\u{a2}\u{a3}\u{a5}\u{20a7}\u{192}\
        \u{e1}\u{ed}\u{f3}\u{fa}\u{f1}\u{d1}\u{aa}\u{ba}\u{bf}\u{2310}\u{ac}\u{bd}\u{bc}\u{a1}\u{ab}\u{bb}\
        \u{2591}\u{2592}\u{2593}\u{2502}\u{2524}\u{2561}\u{2562}\u{2556}\u{2555}\u{2563}\u{2551}\u{2557}\u{255d}\u{255c}\u{255b}\u{2510}\
        \u{2514}\u{2534}\u{252c}\u{251c}\u{2500}\u{253c}\u{255e}\u{255f}\u{255a}\u{2554}\u{2569}\u{2566}\u{2560}\u{2550}\u{256c}\u{2567}\
        \u{2568}\u{2564}\u{2565}\u{2559}\u{2558}\u{2552}\u{2553}\u{256b}\u{256a}\u{2518}\u{250c}\u{2588}\u{2584}\u{258c}\u{2590}\u{2580}\
        \u{3b1}\u{df}\u{393}\u{3c0}\u{3a3}\u{3c3}\u{b5}\u{3c4}\u{3a6}\u{398}\u{3a9}\u{3b4}\u{221e}\u{3c6}\u{3b5}\u{2229}\
        \u{2261}\u{b1}\u{2265}\u{2264}\u{2320}\u{2321}\u{f7}\u{2248}\u{b0}\u{2219}\u{b7}\u{221a}\u{207f}\u{b2}\u{25a0}\u{a0}";

    /// A zip entry's name: UTF-8 when its flag 11 says so, else code page 437 (the format's
    /// default; what a tool that does not set the flag on ASCII names writes).
    fn entry_name(raw: &[u8], utf8: bool) -> String {
        if utf8 {
            return String::from_utf8_lossy(raw).into_owned();
        }
        raw.iter()
            .map(
                |&b| {
                    if b < 0x80 {
                        char::from(b)
                    } else {
                        CP437_HIGH.chars().nth(usize::from(b - 0x80)).unwrap_or('?')
                    }
                },
            )
            .collect()
    }

    /// The central directory's entries (sizes there are right even when the local header defers
    /// them to a data descriptor).
    fn zip(b: &[u8], to: &Path) -> Result<(), String> {
        let bad = || novaui::i18n::t("err.pkg-damaged", &[]);
        let eocd = (0..b.len().saturating_sub(21))
            .rev()
            .take(65557)
            .find(|&i| b[i..].starts_with(b"PK\x05\x06"))
            .ok_or_else(bad)?;
        let n = u16le(b, eocd + 10).ok_or_else(bad)?;
        let mut at = u32le(b, eocd + 16).ok_or_else(bad)?;
        if n > MAX_ENTRIES {
            return Err(novaui::i18n::t("err.pkg-many", &[]));
        }
        let mut total = 0;
        for _ in 0..n {
            if !b.get(at..).is_some_and(|r| r.starts_with(b"PK\x01\x02")) {
                return Err(bad());
            }
            let flags = u16le(b, at + 8).ok_or_else(bad)?;
            let method = u16le(b, at + 10).ok_or_else(bad)?;
            let csize = u32le(b, at + 20).ok_or_else(bad)?;
            let usize_ = u32le(b, at + 24).ok_or_else(bad)?;
            let (nl, el, cl) = (
                u16le(b, at + 28).ok_or_else(bad)?,
                u16le(b, at + 30).ok_or_else(bad)?,
                u16le(b, at + 32).ok_or_else(bad)?,
            );
            let attr = u32le(b, at + 38).ok_or_else(bad)?;
            let local = u32le(b, at + 42).ok_or_else(bad)?;
            let name_raw = b.get(at + 46..at + 46 + nl).ok_or_else(bad)?;
            let name = entry_name(name_raw, flags & 0x800 != 0);
            at += 46 + nl + el + cl;
            // A folder, or a link (unix mode S_IFLNK in the high half): nothing to write.
            let unix_mode = attr >> 16;
            if name.ends_with('/') || unix_mode & 0o170000 == 0o120000 {
                continue;
            }
            if flags & 1 != 0 {
                return Err(novaui::i18n::t("err.pkg-encrypted", &[]));
            }
            if !b.get(local..).is_some_and(|r| r.starts_with(b"PK\x03\x04")) {
                return Err(bad());
            }
            let (lnl, lel) = (u16le(b, local + 26).ok_or_else(bad)?, u16le(b, local + 28).ok_or_else(bad)?);
            let start = local + 30 + lnl + lel;
            let raw = b.get(start..start + csize).ok_or_else(bad)?;
            // Checked against the total before inflating, by the size the entry claims; inflating
            // stops at that size.
            if total + usize_ > MAX_TOTAL {
                return Err(novaui::i18n::t("err.pkg-large", &[]));
            }
            let bytes = match method {
                0 if csize == usize_ => raw.to_vec(),
                0 => return Err(bad()),
                8 => inflate(raw, usize_)?,
                m => return Err(novaui::i18n::t("err.pkg-method", &[("m", &m.to_string())])),
            };
            write(to, &name, &bytes, &mut total)?;
        }
        Ok(())
    }

    fn gunzip(b: &[u8]) -> Result<Vec<u8>, String> {
        let bad = || novaui::i18n::t("err.pkg-damaged", &[]);
        if b.len() < 18 || b[2] != 8 {
            return Err(bad());
        }
        let flg = b[3];
        let mut at = 10;
        if flg & 4 != 0 {
            at += 2 + u16le(b, at).ok_or_else(bad)?;
        }
        for bit in [8u8, 16] {
            if flg & bit != 0 {
                at += b.get(at..).ok_or_else(bad)?.iter().position(|&c| c == 0).ok_or_else(bad)? + 1;
            }
        }
        if flg & 2 != 0 {
            at += 2;
        }
        let isize_ = u32le(b, b.len() - 4).ok_or_else(bad)?;
        inflate(b.get(at..b.len() - 8).ok_or_else(bad)?, isize_.max(1 << 20))
    }

    fn tar(b: &[u8], to: &Path) -> Result<(), String> {
        let bad = || novaui::i18n::t("err.pkg-damaged", &[]);
        let mut at = 0;
        let (mut total, mut count) = (0, 0);
        let mut long_name: Option<String> = None;
        while at + 512 <= b.len() {
            let h = &b[at..at + 512];
            if h.iter().all(|&c| c == 0) {
                break;
            }
            let field =
                |s: usize, l: usize| String::from_utf8_lossy(&h[s..s + l]).trim_end_matches('\0').trim().to_string();
            let size = usize::from_str_radix(&field(124, 12), 8).map_err(|_| bad())?;
            let kind = h[156];
            let prefix = field(345, 155);
            let mut name = field(0, 100);
            if !prefix.is_empty() && &h[257..262] == b"ustar" {
                name = format!("{prefix}/{name}");
            }
            let data = b.get(at + 512..at + 512 + size).ok_or_else(bad)?;
            at += 512 + size.div_ceil(512) * 512;
            count += 1;
            if count > MAX_ENTRIES * 2 {
                return Err(novaui::i18n::t("err.pkg-many", &[]));
            }
            match kind {
                // GNU long name: the next entry's name.
                b'L' => long_name = Some(String::from_utf8_lossy(data).trim_end_matches('\0').to_string()),
                b'0' | 0 | b'7' => {
                    let n = long_name.take().unwrap_or(name);
                    write(to, &n, data, &mut total)?;
                }
                // Folders, links, pax headers and the rest: nothing to write.
                _ => long_name = None,
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn tmp(tag: &str) -> PathBuf {
            let d = std::env::temp_dir().join(format!("np-archive-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            d
        }

        /// A zip as Windows' Compress-Archive writes one: deflated entries under one top folder.
        fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
            let (mut out, mut cd) = (Vec::new(), Vec::new());
            for (name, data) in entries {
                let comp = miniz_oxide::deflate::compress_to_vec(data, 6);
                let off = out.len() as u32;
                let local = |v: &mut Vec<u8>, sig: &[u8]| {
                    v.extend_from_slice(sig);
                };
                local(&mut out, b"PK\x03\x04");
                out.extend_from_slice(&[20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
                out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
                out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                out.extend_from_slice(&(name.len() as u16).to_le_bytes());
                out.extend_from_slice(&[0, 0]);
                out.extend_from_slice(name.as_bytes());
                out.extend_from_slice(&comp);
                cd.extend_from_slice(b"PK\x01\x02");
                cd.extend_from_slice(&[20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
                cd.extend_from_slice(&(comp.len() as u32).to_le_bytes());
                cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
                cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
                cd.extend_from_slice(&[0; 8]);
                cd.extend_from_slice(&[0; 4]);
                cd.extend_from_slice(&off.to_le_bytes());
                cd.extend_from_slice(name.as_bytes());
            }
            let cd_off = out.len() as u32;
            out.extend_from_slice(&cd);
            out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
            out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
            out.extend_from_slice(&cd_off.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out
        }

        fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
            let mut out = Vec::new();
            for (name, data) in entries {
                let mut h = [0u8; 512];
                h[..name.len()].copy_from_slice(name.as_bytes());
                h[100..107].copy_from_slice(b"0000755");
                h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
                h[156] = b'0';
                h[257..262].copy_from_slice(b"ustar");
                h[148..156].copy_from_slice(b"        ");
                let sum: u32 = h.iter().map(|&c| u32::from(c)).sum();
                h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
                out.extend_from_slice(&h);
                out.extend_from_slice(data);
                out.resize(out.len().div_ceil(512) * 512, 0);
            }
            out.resize(out.len() + 1024, 0);
            out
        }

        fn gz(raw: &[u8]) -> Vec<u8> {
            let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
            out.extend(miniz_oxide::deflate::compress_to_vec(raw, 6));
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
            out
        }

        #[test]
        fn zip_and_tgz_packages_unpack_to_their_plugin_folder() {
            let json = br#"{"id":"demo","api":3,"exec":"demo.exe"}"#;
            for (tag, bytes) in [
                (
                    "zip",
                    zip_bytes(&[
                        ("novaplayer-plugin-demo/plugin.json", json),
                        ("novaplayer-plugin-demo/demo.exe", b"MZ program"),
                    ]),
                ),
                ("tgz", gz(&tar_bytes(&[("pkg/plugin.json", json), ("pkg/demo.exe", b"MZ program")]))),
                ("flat", zip_bytes(&[("plugin.json", json), ("demo.exe", b"MZ program")])),
            ] {
                let (d, f) =
                    (tmp(tag), std::env::temp_dir().join(format!("np-archive-{tag}-{}.pkg", std::process::id())));
                std::fs::write(&f, bytes).unwrap();
                extract(&f, &d).unwrap_or_else(|e| panic!("{tag}: {e}"));
                let dir = holding(&d, "plugin.json").expect(tag);
                assert_eq!(std::fs::read(dir.join("plugin.json")).unwrap(), json, "{tag}");
                assert_eq!(std::fs::read(dir.join("demo.exe")).unwrap(), b"MZ program", "{tag}");
                let _ = std::fs::remove_dir_all(&d);
                let _ = std::fs::remove_file(&f);
            }
        }

        #[test]
        fn entries_never_leave_the_folder() {
            for name in ["../evil.txt", "a/../../evil.txt", "/etc/evil", "C:/evil", "..\\evil.txt"] {
                let (d, f) =
                    (tmp("slip"), std::env::temp_dir().join(format!("np-archive-slip-{}.zip", std::process::id())));
                std::fs::write(&f, zip_bytes(&[(name, b"x")])).unwrap();
                assert!(extract(&f, &d).is_err(), "{name} refused");
                assert!(!d.parent().unwrap().join("evil.txt").exists());
                let _ = std::fs::remove_dir_all(&d);
                let _ = std::fs::remove_file(&f);
            }
        }

        /// Regression: device names passed `safe()`; a name without the UTF-8 flag
        /// was read as UTF-8; an entry was inflated before the total was checked.
        #[test]
        fn device_names_cp437_names_and_the_total_are_judged_before_writing() {
            // `zip_bytes` leaves flag 11 clear: these names are ASCII, read the same either way.
            assert!(safe(Path::new("x"), "com\u{b9}").is_none());
            for name in ["CON", "sub/nul.txt", "LPT1.json", "aux", "trailing.", "space "] {
                let (d, f) =
                    (tmp("device"), std::env::temp_dir().join(format!("np-archive-device-{}.zip", std::process::id())));
                std::fs::write(&f, zip_bytes(&[(name, b"x")])).unwrap();
                assert!(extract(&f, &d).is_err(), "{name} refused");
                let _ = std::fs::remove_dir_all(&d);
                let _ = std::fs::remove_file(&f);
            }
            assert!(safe(Path::new("x"), "console.txt").is_some() && safe(Path::new("x"), "com10").is_some());

            assert_eq!(entry_name(b"caf\x82 \xe1.txt", false), "caf\u{e9} \u{df}.txt");
            assert_eq!(entry_name("caf\u{e9}.txt".as_bytes(), true), "caf\u{e9}.txt");

            // One byte inside, 600 MiB claimed: refused by the claim, nothing inflated.
            let mut zip = zip_bytes(&[("big.bin", b"x")]);
            let cd = zip.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
            zip[cd + 24..cd + 28].copy_from_slice(&(600u32 << 20).to_le_bytes());
            let (d, f) =
                (tmp("claim"), std::env::temp_dir().join(format!("np-archive-claim-{}.zip", std::process::id())));
            std::fs::write(&f, zip).unwrap();
            let e = extract(&f, &d).unwrap_err();
            assert_eq!(e, novaui::i18n::t("err.pkg-large", &[]));
            let _ = std::fs::remove_dir_all(&d);
            let _ = std::fs::remove_file(&f);
        }

        #[test]
        fn not_an_archive_is_refused() {
            let (d, f) =
                (tmp("junk"), std::env::temp_dir().join(format!("np-archive-junk-{}.zip", std::process::id())));
            std::fs::write(&f, b"hello").unwrap();
            assert!(extract(&f, &d).is_err());
            let _ = std::fs::remove_dir_all(&d);
            let _ = std::fs::remove_file(&f);
        }
    }
}

/// A file a manifest names is a relative path inside the plugin's folder: not absolute, no `..`,
/// no drive (`D:x.exe` is not absolute, but joined to the folder it is on drive D).
pub(crate) fn inside_folder(rel: &str) -> bool {
    !(Path::new(rel).is_absolute() || rel.split(['/', '\\']).any(|c| c == ".." || c.contains(':')))
}

/// A folder name of ours (a plugin's id, a lyric style's): letters, digits and `-_.`, at most
/// 64 bytes, not starting with a dot.
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        && !id.starts_with('.')
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type()?.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

impl Registry {
    /// The fingerprint an enabled plugin was confirmed with (the face cache keys on it).
    pub fn stamp_of(&self, id: &str) -> Option<&str> {
        self.enabled.get(id).map(String::as_str)
    }

    /// The enabled plugins' controls, in the plugins' order.
    pub fn controls(&self) -> Vec<(String, Vec<crate::plugins::controls::Control>)> {
        self.list
            .iter()
            .filter(|p| p.is_on())
            .filter_map(|p| Some((p.id(), p.manifest.as_ref()?.controls.clone().ok()?)))
            .filter(|(_, c)| !c.is_empty())
            .collect()
    }
}

/// `provides.source.connect`: `flows` (all of [`FLOWS`], not empty) and, with `form`, its `fields`.
fn read_connect(c: &json::Value) -> Result<Connect, String> {
    let flows: Vec<String> = c
        .get("flows")
        .and_then(json::Value::as_array)
        .map(|a| a.iter().filter_map(json::Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    if flows.is_empty() {
        return Err("source.connect has no flows (qr / url / code / form)".into());
    }
    if let Some(f) = flows.iter().find(|f| !FLOWS.contains(&f.as_str())) {
        return Err(format!("source.connect.flows: no flow {f}"));
    }
    let mut fields = Vec::new();
    for f in c.get("fields").and_then(json::Value::as_array).into_iter().flatten() {
        let s = |k: &str| f.get(k).and_then(json::Value::as_str).map(str::trim).filter(|t| !t.is_empty());
        let id = s("id").ok_or("source.connect.fields: an item has no id")?.to_owned();
        if fields.iter().any(|x: &Field| x.id == id) {
            return Err(format!("source.connect.fields: id {id} twice"));
        }
        let kind = match s("kind").unwrap_or("text") {
            "text" => FieldKind::Text,
            "url" => FieldKind::Url,
            "password" => FieldKind::Password,
            k => return Err(format!("source.connect.fields: no kind {k}")),
        };
        fields.push(Field { label: s("label").unwrap_or(&id).to_owned(), id, kind });
    }
    if fields.len() > MAX_FIELDS {
        return Err(format!("source.connect.fields: at most {MAX_FIELDS} items"));
    }
    if flows.iter().any(|f| f == "form") == fields.is_empty() {
        return Err("source.connect: fields come with the form flow only".into());
    }
    Ok(Connect { flows, fields })
}

/// The plugin's `strings` (`plugins.md` §plugin.json): locale → (key, text).
fn read_strings(strings: Option<&json::Value>) -> Vec<(String, Vec<(String, String)>)> {
    let Some(json::Value::Obj(locales)) = strings else { return Vec::new() };
    locales
        .iter()
        .filter_map(|(locale, table)| match table {
            json::Value::Obj(t) => Some((
                locale.clone(),
                t.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect(),
            )),
            _ => None,
        })
        .collect()
}

impl Manifest {
    /// Its `strings` into the process's table under `id:`, in place of what it had (an update,
    /// a reinstall).
    fn add_strings(&self) {
        novaui::i18n::global().replace_under(&format!("{}:", self.id), self.strings.iter().cloned());
    }

    /// One of its texts as shown: `@key` its own string in the player's language, else as written.
    pub fn text(&self, raw: &str) -> String {
        match raw.strip_prefix('@') {
            Some(key) => novaui::i18n::t(&format!("{}:{key}", self.id), &[]),
            None => raw.to_owned(),
        }
    }

    /// Its name as shown.
    pub fn name(&self) -> String {
        self.text(&self.name)
    }

    /// The name of the choice its songs come in (`option.name`), as shown.
    pub fn option_name(&self) -> Option<String> {
        self.provides.option.as_ref().map(|(_, n)| self.text(n))
    }
}

pub fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(dir.join("plugin.json")).map_err(|_| "no plugin.json".to_string())?;
    let v = json::parse(&text).map_err(|e| format!("plugin.json cannot be parsed: {e}"))?;
    let s = |k: &str| v.get(k).and_then(json::Value::as_str).map(str::to_owned);
    let id = s("id").ok_or("plugin.json has no id")?;
    let exec = s("exec").ok_or("plugin.json has no exec")?;
    if !inside_folder(&exec) {
        return Err("exec must be a relative path inside the plugin folder".into());
    }
    // A whole number; anything else (3.5, "3", -1) asks for no version this build speaks.
    let api = v.get("api").and_then(json::Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
    let list = |v: Option<&json::Value>| -> Vec<String> {
        v.and_then(json::Value::as_array)
            .map(|a| a.iter().filter_map(json::Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    };
    let p = v.get("provides");
    let src = p.and_then(|p| p.get("source"));
    let connect = match src.and_then(|s| s.get("connect")) {
        None => None,
        Some(c) => Some(read_connect(c)?),
    };
    let provides = Provides {
        decode: list(p.and_then(|p| p.get("decode")).and_then(|d| d.get("ext")))
            .into_iter()
            .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
            .collect(),
        metadata: p.and_then(|p| p.get("metadata")).and_then(json::Value::as_bool).unwrap_or(false),
        thumbs: p.and_then(|p| p.get("thumbs")).and_then(json::Value::as_bool).unwrap_or(false),
        schemes: list(src.and_then(|s| s.get("schemes"))).into_iter().map(|s| s.to_ascii_lowercase()).collect(),
        links: list(src.and_then(|s| s.get("links"))),
        connect,
        lists: src.and_then(|s| s.get("lists")).and_then(json::Value::as_bool).unwrap_or(false),
        option: src.and_then(|s| s.get("option")).and_then(|o| {
            let id = o.get("id").and_then(json::Value::as_str)?.to_owned();
            let name = o.get("name").and_then(json::Value::as_str).unwrap_or(&id).to_owned();
            Some((id, name))
        }),
        style_res: src
            .and_then(|s| s.get("style"))
            .and_then(|s| s.get("res"))
            .and_then(json::Value::as_str)
            .map(str::to_owned),
    };
    Ok(Manifest {
        name: s("name").unwrap_or_else(|| id.clone()),
        version: s("version").unwrap_or_default(),
        id,
        api,
        exec,
        provides,
        network: list(v.get("network")),
        concurrent: v.get("concurrent").and_then(json::Value::as_bool).unwrap_or(false),
        controls: crate::plugins::controls::read_controls(dir, &v),
        strings: read_strings(v.get("strings")),
    })
}

/// A small JSON reader (manifests and protocol lines are small; no dependency needed).
#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("np-plugins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_plugin(root: &Path, id: &str, api: u32) -> PathBuf {
        let d = root.join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("plugin.json"),
            format!(
                r#"{{ "id": "{id}", "name": "示例 \u63d2\u4ef6", "api": {api}, "exec": "run.exe",
                     "provides": {{ "decode": {{ "ext": [".XYZ", "abc"] }}, "metadata": true,
                                    "source": {{ "schemes": ["Demo"], "links": ["https"],
                                                 "connect": {{ "flows": ["qr", "form"], "fields": [
                                                     {{ "id": "server", "label": "服务器", "kind": "url" }},
                                                     {{ "id": "password", "kind": "password" }} ] }},
                                                 "lists": true,
                                                 "option": {{ "id": "quality", "name": "音质" }},
                                                 "style": {{ "res": "{{res}} · {{option}}" }} }} }},
                     "network": ["api.example.org"] }}"#
            ),
        )
        .unwrap();
        std::fs::write(d.join("run.exe"), b"v1").unwrap();
        d
    }

    #[test]
    fn manifest_reads_capabilities() {
        let root = tmp("manifest");
        let d = write_plugin(&root, "t-demo", API);
        let m = read_manifest(&d).unwrap();
        assert_eq!(m.name, "示例 插件");
        assert_eq!(m.provides.decode, vec!["xyz", "abc"]);
        assert!(m.provides.metadata);
        assert_eq!(m.provides.links, vec!["https"]);
        assert_eq!(m.provides.schemes, vec!["demo"]);
        assert_eq!(m.provides.option, Some(("quality".into(), "音质".into())));
        assert_eq!(m.provides.style_res.as_deref(), Some("{res} · {option}"));
        let c = m.provides.connect.as_ref().unwrap();
        assert_eq!(c.flows, vec!["qr", "form"]);
        assert_eq!(c.fields.len(), 2);
        assert_eq!((c.fields[0].label.as_str(), c.fields[0].kind), ("服务器", FieldKind::Url));
        assert_eq!((c.fields[1].label.as_str(), c.fields[1].kind), ("password", FieldKind::Password));
        assert!(m.provides.lists);
        assert_eq!(m.network, vec!["api.example.org"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn exec_must_stay_inside() {
        let root = tmp("escape");
        let d = root.join("bad");
        std::fs::create_dir_all(&d).unwrap();
        // "D:x.exe" is not absolute and has no "..", but joined to the folder it is on drive D.
        for exec in ["../x.exe", "D:x.exe"] {
            let json = format!(r#"{{"id":"bad","api":2,"exec":"{exec}"}}"#);
            std::fs::write(d.join("plugin.json"), json).unwrap();
            assert!(read_manifest(&d).is_err(), "{exec} refused");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn enable_is_reconfirmed_after_change() {
        let root = tmp("enable");
        let d = write_plugin(&root, "t-demo", API);
        write_plugin(&root, "future", API + 1);
        let mut r = Registry::open(Some(root.clone()));
        assert_eq!(r.get("t-demo").unwrap().state, State::Off);
        assert!(matches!(r.get("future").unwrap().state, State::Broken(_)));
        assert!(!r.set_enabled("future", true));
        assert!(r.set_enabled("t-demo", true));
        r.settle();
        assert_eq!(Registry::open(Some(root.clone())).get("t-demo").unwrap().state, State::On);
        std::fs::write(d.join("run.exe"), b"v2, changed").unwrap();
        let mut again = Registry::open(Some(root.clone()));
        assert_eq!(again.get("t-demo").unwrap().state, State::Checking);
        again.settle();
        assert_eq!(again.get("t-demo").unwrap().state, State::Changed);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Regression: install deleted the old folder in place and copied over it; with
    /// the plugin running, Windows keeps its program locked, and the copy failed half-way.
    #[test]
    fn installing_over_a_running_plugin_swaps_it_whole() {
        crate::app::speak_zh_cn();
        let (root, pkg) = (tmp("live-root"), tmp("live-pkg"));
        std::fs::copy(crate::plugins::host::demo_plugin(), pkg.join("demo.exe")).unwrap();
        let json = format!(r#"{{"id":"t-live","api":{API},"exec":"demo.exe","concurrent":true}}"#);
        std::fs::write(pkg.join("plugin.json"), json).unwrap();
        let mut r = Registry::open(Some(root.clone()));
        r.install_package(&pkg).unwrap();
        let dir = root.join("t-live");
        let host = crate::plugins::pcm::host();
        host.register("t-live", dir.clone(), dir.join("demo.exe"), true);
        assert!(host.call("t-live", "hello", &[], std::time::Duration::from_secs(10)).is_ok());

        std::fs::write(pkg.join("v2.txt"), b"v2").unwrap();
        r.install_package(&pkg).expect("installed over the running one");
        assert!(dir.join("v2.txt").is_file() && dir.join("demo.exe").is_file());
        assert!(!host.registered("t-live"));
        r.settle();
        // Sorted: a directory's order is the file system's (NTFS sorts, ext4 does not).
        let mut left: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name()).collect();
        left.sort();
        assert_eq!(left, ["enabled.txt", "fingerprints.txt", "t-live"]);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(pkg);
    }

    #[test]
    fn json_edge_cases() {
        use json::{parse, Value};
        assert_eq!(parse(r#""\ud83c\udfb5""#).unwrap(), Value::Str("🎵".into()));
        assert!(parse("[1,]").is_err());
        assert!(parse(&"[".repeat(100)).is_err());
        assert_eq!(parse(" -1.5e2 ").unwrap(), Value::Num(-150.0));
        let v = parse(r#"{"a":[1,2.5,"x\"y\n"],"b":null,"c":true}"#).unwrap();
        assert_eq!(parse(&json::write(&v)).unwrap(), v);
        assert!(!json::write(&Value::Str("a\nb".into())).contains('\n'));
    }
}
