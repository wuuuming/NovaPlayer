//! The user's chain of CLAP effects (player-core's host): which effects, in which order,
//! bypassed or not, and each one's own state.
//!
//! Kept in `<data>\effects.txt`, a line per effect in chain order: `on` or `off`, the plugin's
//! id and the file's path, tab-separated; the effect on line n keeps its state in
//! `<data>\effects\<n>.bin`. Kept (`app::kept`) after the chain, a parameter or an effect's state
//! changed (from Settings or its own window), on the store's thread, and at exit.
//!
//! The UI thread never waits on the plugin host thread: effects are made there
//! (`Effects::add_then`, the kept chain at start and one added), their states are asked by the
//! write's task, their parameters and windows answer on it.
//!
//! An effect's parameters are shown on its own level of the settings page (`设置 › 音效 ›
//! <name>`): read on the plugin host thread when it opens and whenever the chain's change count
//! moves ([`EffectsModel::read_params`]), set from there ([`EffectsModel::set_param`]).
//!
//! The effects that can be added are found by scanning: every `.clap` file in the CLAP folders
//! is opened by a child process (`novaplayer --scan-clap <file>`), so that a plugin that crashes
//! fails only its own scan. What a scan found is kept in `<data>\clap-scan.txt`, by the file's
//! path, size and time of change: an unchanged file is not opened again.

use crate::player_core::clap::{self, EffectKey, EffectParam, Effects};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// What can be done to an effect from the Effects page (`Command::Effect`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectVerb {
    /// Adds the found effect at this place in [`EffectsModel::found`].
    Add,
    Remove,
    Up,
    Down,
    /// Turns bypass on or off.
    Bypass,
    /// Opens the effect's own window.
    Window,
}

impl EffectVerb {
    pub const ALL: [EffectVerb; 6] = [Self::Add, Self::Remove, Self::Up, Self::Down, Self::Bypass, Self::Window];

    pub fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Up => "up",
            Self::Down => "down",
            Self::Bypass => "bypass",
            Self::Window => "window",
        }
    }

    pub fn of(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == name)
    }
}

/// An effect a scan found in a `.clap` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub vendor: String,
}

pub struct EffectsModel {
    /// What every session the engine opens plays through.
    pub chain: Effects,
    /// The effects that can be added, once scanned; none before.
    pub found: Option<Vec<Found>>,
    /// The scan running.
    pub scan: Option<novaui::Task>,
    /// Where the chain is kept; none: not kept (tests, no data folder).
    dir: Option<PathBuf>,
    /// The chain's change count as last written.
    written: u64,
    /// The change count once the kept chain had loaded (its effects are made after `load`).
    loaded: Arc<AtomicU64>,
    /// The change count as last looked at (`Player::turn`): when it moves, what is shown is read
    /// again and the chain is kept.
    pub seen: u64,
    /// The chain's effects as the Effects page last listed them (`Player::effects_turn`).
    pub listed: Vec<EffectKey>,
    /// The parameters of the effect whose level is shown, as last read: its key and them.
    pub params: Option<(EffectKey, Vec<EffectParam>)>,
    /// A read of them is under way; `again`: the chain changed meanwhile, read once more after.
    pub reading: bool,
    pub again: bool,
}

impl EffectsModel {
    /// The chain kept in `dir` (none: an empty chain kept nowhere). An effect that cannot be
    /// loaded any more (its file gone) is left out.
    pub fn load(dir: Option<PathBuf>) -> Self {
        let chain = Effects::new();
        let loaded: Arc<AtomicU64> = Arc::default();
        if let Some(dir) = &dir {
            let text = std::fs::read_to_string(dir.join("effects.txt")).unwrap_or_default();
            for (n, line) in text.lines().enumerate() {
                let Some((on, id, path)) = parse_line(line) else { continue };
                // A file that is gone (an unplugged drive) is not handed to the loader, which
                // may wait on it while holding the process's loader lock.
                if !Path::new(path).is_file() {
                    continue;
                }
                let state = std::fs::read(state_file(dir, n)).ok();
                // Made on the plugin host thread, in order: the UI never waits for a plugin's
                // load (a song opened meanwhile waits for the chain, player-core's `stage`).
                let (made, loaded) = (chain.clone(), loaded.clone());
                chain.add_then(Path::new(path), id, state, move |key| {
                    if let Ok(key) = key {
                        made.set_bypass(key, !on);
                    }
                    // What loading changed is what is kept already.
                    loaded.store(made.changes(), Ordering::Release);
                });
            }
        }
        let written = chain.changes();
        EffectsModel {
            chain,
            found: None,
            scan: None,
            dir,
            written,
            loaded,
            listed: Vec::new(),
            seen: written,
            params: None,
            reading: false,
            again: false,
        }
    }

    pub fn player() -> Self {
        Self::load(crate::data_dir())
    }

    /// Does `verb` to the effect at `at` (its place in the chain; for `Add`, in `found`), with
    /// `parent` the window an effect's window floats above. The error says why it failed;
    /// `Window` does not wait for the plugin to build its window: `opened` gets the outcome, on
    /// another thread.
    pub fn apply(
        &mut self,
        verb: EffectVerb,
        at: usize,
        parent: isize,
        report: impl FnOnce(Result<(), String>) + Send + 'static,
    ) -> Result<(), String> {
        if verb == EffectVerb::Add {
            let found = self.found.as_ref().and_then(|f| f.get(at)).ok_or_else(String::new)?;
            // Made on the plugin host thread; its window is built there too, hidden, once it is.
            let chain = self.chain.clone();
            self.chain.add_then(&found.path, &found.id, None, move |key| match key {
                Ok(_) => chain.prepare_windows(parent),
                Err(why) => report(Err(why)),
            });
            return Ok(());
        }
        let list = self.chain.list();
        let Some(effect) = list.get(at) else { return Ok(()) };
        match verb {
            EffectVerb::Add => {}
            EffectVerb::Remove => {
                self.chain.remove(effect.key);
            }
            EffectVerb::Up => self.chain.move_to(effect.key, at.saturating_sub(1)),
            EffectVerb::Down => self.chain.move_to(effect.key, at + 1),
            EffectVerb::Bypass => self.chain.set_bypass(effect.key, !effect.bypass),
            EffectVerb::Window => self.chain.open_window_then(effect.key, parent, report),
        }
        Ok(())
    }

    /// Builds the effects' windows, hidden, in the background (a plugin may take most of a
    /// second): done when the Effects page opens, so that its Open buttons answer at once.
    pub fn prepare_windows(&self, parent: isize) {
        self.chain.prepare_windows(parent);
    }

    /// What writes the chain and each effect's state as they are now (the states asked of the
    /// plugin host thread where it runs: the store's thread), unless it is written already.
    pub fn writer(&mut self) -> Option<impl FnOnce() + Send + 'static> {
        self.written = self.written.max(self.loaded.swap(0, Ordering::AcqRel));
        let changes = self.chain.changes();
        if changes == self.written {
            return None;
        }
        self.written = changes;
        let dir = self.dir.clone()?;
        let (chain, list) = (self.chain.clone(), self.chain.list());
        Some(move || write(&dir, &chain, &list))
    }

    /// Reads the parameters of the effect at `at` without waiting: `done` gets its key and them,
    /// on another thread. One read at a time: asked during one, it is made again after
    /// (`again`). False: there is no effect at `at`.
    pub fn read_params(
        &mut self,
        at: usize,
        done: impl FnOnce(EffectKey, Result<Vec<EffectParam>, String>) + Send + 'static,
    ) -> bool {
        let Some(key) = self.chain.list().get(at).map(|e| e.key) else { return false };
        if self.reading {
            self.again = true;
            return true;
        }
        self.reading = true;
        self.chain.params_then(key, move |params| done(key, params));
        true
    }

    /// A read's result: kept when it is of the effect at `at` (none: it is gone). True: read
    /// again, the chain changed while it was made.
    pub fn params_read(&mut self, at: usize, key: EffectKey, params: Result<Vec<EffectParam>, String>) -> bool {
        self.reading = false;
        if self.chain.list().get(at).map(|e| e.key) == Some(key) {
            self.params = Some((key, params.unwrap_or_default()));
        }
        std::mem::take(&mut self.again)
    }

    /// Sets parameter `id` of the effect at `at` (the plugin takes it with its next block, or at
    /// once while nothing plays): its window shows it, the chain's change count moves.
    pub fn set_param(&mut self, at: usize, id: u32, value: f64) {
        let Some(key) = self.chain.list().get(at).map(|e| e.key) else { return };
        let _ = self.chain.set_param(key, id, value);
        // Shown at once; the next read confirms it.
        if let Some((shown, params)) = &mut self.params {
            if *shown == key {
                if let Some(p) = params.iter_mut().find(|p| p.id == id) {
                    p.value = value;
                }
            }
        }
    }

    /// Finds the effects that can be added, as a task (a child process per file it has not
    /// seen): they land as `Job::EffectsScanned`.
    pub fn scan(&mut self, ui: &mut novaui::Ui) {
        if self.scan.as_ref().is_some_and(novaui::Task::pending) {
            return;
        }
        let cache = self.dir.as_ref().map(|d| d.join("clap-scan.txt"));
        self.scan = Some(ui.spawn_waiting(
            move |_| scan(cache.as_deref(), &clap::find_plugins(), &scan_in_child),
            |found| crate::app::Landed(crate::app::Job::EffectsScanned(found)),
        ));
    }

    /// A scan's result.
    pub fn scanned(&mut self, found: Vec<Found>) {
        self.scan = None;
        self.found = Some(found);
    }
}

fn state_file(dir: &Path, n: usize) -> PathBuf {
    dir.join("effects").join(format!("{n}.bin"))
}

/// A line of `effects.txt`: whether it is on, the plugin's id, the file.
fn parse_line(line: &str) -> Option<(bool, &str, &str)> {
    let mut parts = line.splitn(3, '\t');
    let on = match parts.next()? {
        "on" => true,
        "off" => false,
        _ => return None,
    };
    let (id, path) = (parts.next()?, parts.next()?);
    (!id.is_empty() && !path.is_empty()).then_some((on, id, path))
}

/// What a `.clap` file holds: each plugin's id, name, vendor and whether it is an effect.
pub type Described = Vec<(String, String, String, bool)>;

/// A file's size and time of change, as the scan cache names them.
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let time = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some((meta.len(), time))
}

/// The effects in `files`: each file's from the cache in `cache` when its size and time match,
/// otherwise from `open` (what the file holds: id, name, vendor, whether an effect; or why it
/// could not be opened). The cache is written again with what was found.
pub fn scan(cache: Option<&Path>, files: &[PathBuf], open: &dyn Fn(&Path) -> Result<Described, String>) -> Vec<Found> {
    // Cache lines: `path \t size \t time \t id \t name \t vendor \t effect`; a file that held
    // nothing (or could not be opened) has one line with an empty id.
    let old = cache.and_then(|c| std::fs::read_to_string(c).ok()).unwrap_or_default();
    let mut found = Vec::new();
    let mut text = String::new();
    for file in files {
        let Some((size, time)) = stamp(file) else { continue };
        let key = format!("{}\t{size}\t{time}\t", file.display());
        let cached: Vec<&str> = old.lines().filter_map(|l| l.strip_prefix(key.as_str())).collect();
        let plugins: Described = if cached.is_empty() {
            open(file).unwrap_or_default()
        } else {
            cached
                .iter()
                .filter_map(|rest| {
                    let mut f = rest.split('\t');
                    let (id, name, vendor, effect) = (f.next()?, f.next()?, f.next()?, f.next()?);
                    (!id.is_empty()).then(|| (id.into(), name.into(), vendor.into(), effect == "1"))
                })
                .collect()
        };
        if plugins.is_empty() {
            text.push_str(&format!("{key}\t\t\t0\n"));
        }
        for (id, name, vendor, effect) in plugins {
            text.push_str(&format!("{key}{id}\t{name}\t{vendor}\t{}\n", u8::from(effect)));
            if effect {
                found.push(Found { path: file.clone(), id, name, vendor });
            }
        }
    }
    if let Some(cache) = cache {
        let _ = crate::files::write_atomic(cache, text.as_bytes());
    }
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    found
}

/// Opens `file` in a child process (`--scan-clap`): a plugin that crashes there fails only its
/// own scan.
fn scan_in_child(file: &Path) -> Result<Described, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = std::process::Command::new(exe)
        .arg("--scan-clap")
        .arg(file)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("the scan of {} failed", file.display()));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (id, name, vendor, effect) = (f.next()?, f.next()?, f.next()?, f.next()?);
            Some((id.to_owned(), name.to_owned(), vendor.to_owned(), effect == "1"))
        })
        .collect())
}

/// `novaplayer --scan-clap <file>`: prints each plugin in `file` as `id \t name \t vendor \t
/// effect` and exits; 1 when it cannot be opened.
pub fn scan_clap_main(file: &Path) -> ! {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    match clap::describe(file) {
        Ok(plugins) => {
            for p in plugins {
                println!("{}\t{}\t{}\t{}", clean(&p.id), clean(&p.name), clean(&p.vendor), u8::from(p.is_effect()));
            }
            std::process::exit(0)
        }
        Err(_) => std::process::exit(1),
    }
}

/// Writes `list`, the chain's effects in order, and each one's state into `dir`.
fn write(dir: &Path, chain: &Effects, list: &[crate::player_core::clap::EffectInfo]) {
    let states = dir.join("effects");
    let _ = std::fs::create_dir_all(&states);
    let mut text = String::new();
    for (n, effect) in list.iter().enumerate() {
        let on = if effect.bypass { "off" } else { "on" };
        text.push_str(&format!("{on}\t{}\t{}\n", effect.plugin.id, effect.path.display()));
        match chain.state(effect.key) {
            Ok(state) => {
                let _ = crate::files::write_atomic(&state_file(dir, n), &state);
            }
            Err(_) => {
                let _ = std::fs::remove_file(state_file(dir, n));
            }
        }
    }
    // The states of places no longer in the chain.
    for n in list.len().. {
        if std::fs::remove_file(state_file(dir, n)).is_err() {
            break;
        }
    }
    let _ = crate::files::write_atomic(&dir.join("effects.txt"), text.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("novaplayer-effects-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn effects_lines_name_on_id_and_path() {
        assert_eq!(
            parse_line("off\tnova.test.gain\tC:\\a b\\x.clap"),
            Some((false, "nova.test.gain", "C:\\a b\\x.clap"))
        );
        assert_eq!(parse_line("on\tid\t/x.clap"), Some((true, "id", "/x.clap")));
        assert_eq!(parse_line("maybe\tid\t/x.clap"), None);
        assert_eq!(parse_line("on\t\t/x.clap"), None);
    }

    /// A file is opened once; an unchanged one comes from the cache after, a changed one is
    /// opened again; one that failed is not listed and not retried while unchanged.
    #[test]
    fn a_scan_opens_each_file_once_until_it_changes() {
        let dir = temp("scan");
        let (good, bad) = (dir.join("good.clap"), dir.join("bad.clap"));
        std::fs::write(&good, b"one").unwrap();
        std::fs::write(&bad, b"bad").unwrap();
        let cache = dir.join("clap-scan.txt");
        let opened = std::cell::RefCell::new(Vec::new());
        let open = |f: &Path| {
            opened.borrow_mut().push(f.to_path_buf());
            if f.ends_with("bad.clap") {
                return Err("crashed".to_string());
            }
            Ok(vec![
                ("b.fx".into(), "Beta".into(), "V".into(), true),
                ("a.synth".into(), "Alpha".into(), "V".into(), false),
                ("a.fx".into(), "alpha fx".into(), "V".into(), true),
            ])
        };
        let files = [good.clone(), bad.clone()];
        let first = scan(Some(&cache), &files, &open);
        let names: Vec<&str> = first.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["alpha fx", "Beta"], "effects only, by name");
        assert_eq!(opened.borrow().len(), 2);
        assert_eq!(scan(Some(&cache), &files, &open), first, "from the cache");
        assert_eq!(opened.borrow().len(), 2, "nothing opened again");
        std::fs::write(&good, b"changed").unwrap();
        assert_eq!(scan(Some(&cache), &files, &open), first);
        assert_eq!(opened.borrow().last(), Some(&good), "the changed file only");
        assert_eq!(opened.borrow().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verbs_round_trip_their_names() {
        for v in EffectVerb::ALL {
            assert_eq!(EffectVerb::of(v.name()), Some(v));
        }
    }

    /// A chain kept in a folder with an effect whose file is gone loads empty, and an empty
    /// chain writes nothing until it changes.
    #[test]
    fn a_missing_effect_file_is_left_out() {
        let dir = temp("load");
        let line = format!("on\tnova.test.gain\t{}\n", dir.join("gone.clap").display());
        std::fs::write(dir.join("effects.txt"), line).unwrap();
        let mut m = EffectsModel::load(Some(dir.clone()));
        assert!(m.chain.list().is_empty());
        if let Some(write) = m.writer() {
            write();
        }
        assert!(std::fs::read_to_string(dir.join("effects.txt")).unwrap().contains("gone"), "untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
