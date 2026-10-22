//! What a panic leaves behind (principles B1, B3). Release builds abort on a panic, so nothing
//! unwinds and `Drop for Player` never runs: the hook installed here writes the panic to
//! `crash.log` and saves what `Drop` would have saved (the queue and its position, the volume,
//! the lists not yet written) before the process ends.
//!
//! The player lives on the UI thread. A panic there saves it in place; a panic on any other
//! thread asks the UI thread to save (`Msg::Rescue`) and waits at most `RESCUE_WAIT` for it.

use crate::app::Player;
use crate::sync::Locked;
use std::cell::Cell;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// How long a panic off the UI thread waits for the UI thread's save (B6).
const RESCUE_WAIT: Duration = Duration::from_secs(2);

/// `crash.log` starts over past this size: it keeps the latest crashes, not a history.
const LOG_LIMIT: u64 = 256 * 1024;

thread_local! {
    /// The player, on the UI thread only, from when it is boxed until it drops.
    static PLAYER: Cell<*mut Player> = const { Cell::new(std::ptr::null_mut()) };
}

/// Turns the UI thread with `Msg::Rescue`; set once the player is watched.
static WAKE: OnceLock<novaui::Waker> = OnceLock::new();
/// Where the UI thread says it has saved, while a panic elsewhere waits for it.
static SAVED: Mutex<Option<Sender<()>>> = Mutex::new(None);
/// One rescue per process: a panic during the rescue (or a second thread's) only logs.
static RESCUING: AtomicBool = AtomicBool::new(false);

/// Installs the hook. Called once, first thing in `main`; tests never install it.
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        log(info);
        rescue();
    }));
}

/// From now until `forget`, a panic saves `player`. On the UI thread, where `player` stays (it is
/// boxed and owned by the window's runner); `wake` delivers `Msg::Rescue` to it.
pub fn watch(player: &mut Player, wake: novaui::Waker) {
    PLAYER.with(|p| p.set(player));
    let _ = WAKE.set(wake);
}

/// `player` is going (its `Drop`): a panic no longer reaches it.
pub fn forget(player: &mut Player) {
    PLAYER.with(|p| {
        if std::ptr::eq(p.get(), player) {
            p.set(std::ptr::null_mut());
        }
    });
}

/// The UI thread has saved on `Msg::Rescue`: the panic that asked may end the process.
pub fn rescued() {
    if let Some(saved) = SAVED.locked().take() {
        let _ = saved.send(());
    }
}

fn rescue() {
    if RESCUING.swap(true, Ordering::AcqRel) {
        return;
    }
    let here = PLAYER.with(Cell::get);
    if !here.is_null() {
        // SAFETY: `here` was registered by `watch` on this thread and is cleared by `forget`
        // before the player is freed, so it points at a live `Player`. The panic may have come
        // from inside a `&mut Player` call further up this stack, which this reborrows: those
        // frames never run again, since the process aborts when this hook returns, and a panic
        // leaves the player between statements (Rust's checks fire before the write they guard).
        // A rescue that panics in turn aborts at once (a panic inside the panic hook).
        unsafe { (*here).rescue() };
        return;
    }
    let Some(wake) = WAKE.get() else { return };
    let (saved, done) = channel();
    *SAVED.locked() = Some(saved);
    wake.wake();
    let _ = done.recv_timeout(RESCUE_WAIT);
}

fn log(info: &std::panic::PanicHookInfo<'_>) {
    let thread = std::thread::current();
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let text = format!(
        "[{at}] novaplayer panicked on thread '{}': {info}\n{}\n",
        thread.name().unwrap_or("unnamed"),
        std::backtrace::Backtrace::force_capture(),
    );
    eprint!("{text}");
    let Some(dir) = crate::data_dir() else { return };
    let file = dir.join("crash.log");
    let fresh = std::fs::metadata(&file).is_ok_and(|m| m.len() > LOG_LIMIT);
    let _ = std::fs::create_dir_all(&dir);
    let opened = std::fs::OpenOptions::new().create(true).append(!fresh).write(true).truncate(fresh).open(&file);
    if let Ok(mut f) = opened {
        let _ = f.write_all(text.as_bytes());
    }
}
