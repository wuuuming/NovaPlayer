//! NovaPlayer (music) on NovaUI: the app as a library, started by `main.rs` through [`run`].
//!
//! With the `test-support` feature the modules the scenarios drive (`app`, `library`, `playback`,
//! `plugins`, `views`) are public, and the app's test seams with them (an in-memory account
//! store, the demo plugin, row-cache counts); never in a product build.

use ::player_core;

#[cfg(feature = "test-support")]
pub mod app;
#[cfg(not(feature = "test-support"))]
mod app;
mod crash;
mod files;
#[cfg(feature = "test-support")]
pub mod library;
#[cfg(not(feature = "test-support"))]
mod library;
#[cfg(feature = "test-support")]
pub mod playback;
#[cfg(not(feature = "test-support"))]
mod playback;
#[cfg(feature = "test-support")]
pub mod plugins;
#[cfg(not(feature = "test-support"))]
mod plugins;
#[cfg(test)]
mod search_style_tests;
mod sync;
#[cfg(feature = "test-support")]
pub mod views;
#[cfg(not(feature = "test-support"))]
mod views;

/// Where the app keeps its data: `%LOCALAPPDATA%`, else `$XDG_DATA_HOME`, else `~/.local/share`.
pub fn data_root() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local").join("share")))
}

/// The program's data folder under `data_root`.
pub const DATA_DIR: &str = "NovaPlayer";

/// `data_root()/DATA_DIR`, or `None` with no data root.
pub fn data_dir() -> Option<std::path::PathBuf> {
    data_root().map(|d| d.join(DATA_DIR))
}

/// The NovaPlayer repository this crate was built from, for tests outside it (the example lyric
/// style, the plugins' builds).
#[cfg(feature = "test-support")]
pub fn source_root() -> std::path::PathBuf {
    std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).to_path_buf()
}

/// The program: its command line, then the window until it closes.
pub fn run() {
    // The effects scan's child (playback::effects): one CLAP file described, before the crash
    // handler: a plugin that crashes here fails its scan, nothing more.
    if let [flag, file] = std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        if flag == "--scan-clap" {
            playback::effects::scan_clap_main(std::path::Path::new(file));
        }
    }
    crash::install();
    let mut args = std::env::args().skip(1);
    let mut backend = novaui::platform::BackendKind::Default;
    let mut files = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--backend" {
            backend = args
                .next()
                .and_then(|v| novaui::platform::BackendKind::parse(&v))
                .unwrap_or_else(|| std::process::exit(2));
        } else if arg.starts_with("--") {
            // Removed diagnostic commands must not accidentally import their output directories.
            std::process::exit(2);
        } else {
            files.push(std::path::PathBuf::from(arg));
        }
    }
    // Screen readers reach the window only when the setting asks for it (read once, at start).
    let accessibility =
        crate::views::settings_page::screen_reader_of(app::settings::SettingsFile::load().get("screen-reader"));
    novaui::platform::run(
        novaui::platform::Options {
            title: "NovaPlayer".into(),
            width: 1280,
            height: 720,
            min_width: 360,
            min_height: 300,
            backend,
            accessibility,
            // Where the window stood when the player last closed (`app::settings::window_state`).
            bounds: app::settings::window_state::load().and_then(|s| s.bounds),
        },
        move |ui| {
            let mut p = Box::new(app::Player::new(ui));
            crash::watch(&mut p, ui.waker(app::Msg::Rescue));
            if !files.is_empty() {
                p.add_paths(ui, files.clone(), false);
            }
            p
        },
    )
    .unwrap_or_else(|e| {
        eprintln!("novaplayer: no window: {e}");
        std::process::exit(1);
    });
}
