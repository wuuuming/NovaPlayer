//! The playback engine: player-core's `AudioPlayer` on a worker thread. The UI sends commands and
//! reads a snapshot; the worker wakes the UI thread when something it shows changed (position
//! ticks are not pushed: the UI computes the position from the snapshot's clock).

use crate::player_core::{
    AudioNext, AudioOpenOptions, AudioPlayer, AudioSourceFactory, MediaTags, PathAudioSourceFactory, ReplayGain,
};
use crate::sync::Locked;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A song's source, the address it streams from, and the plugin option it plays in.
type Source = (Arc<dyn AudioSourceFactory>, Option<String>, Option<(String, String)>);

/// The source a song plays from: a decode plugin's PCM for a file it claims, else the file.
/// For an online song, also the address its plugin gave (probed once for the info page).
fn source_for(acc: &crate::plugins::accounts::Accounts, path: &std::path::Path) -> Result<Source, String> {
    if let Some(id) = crate::plugins::pcm::decoder_for(path) {
        let s = crate::plugins::pcm::PcmSource::open(&id, path)?;
        return Ok((crate::plugins::pcm::factory(s), None, None));
    }
    // An online song: ask its plugin for the address now (used once, never stored).
    if let Some((plugin, uri)) = crate::plugins::client::parse(path) {
        let st = crate::plugins::client::resolve(acc, &plugin, &uri)?;
        let (url, q) = (st.url, st.option);
        return Ok((address_source(&url), Some(url), q));
    }
    // A plugin URI with no enabled resolver is not a local file. In particular this happens
    // after a package update requires trust confirmation again. Keep the song selected and
    // give a useful error rather than handing the URI to the file decoder.
    if let Some(sc) = path.to_str().and_then(crate::plugins::client::scheme) {
        if !["http", "https", "file"].iter().any(|s| sc.eq_ignore_ascii_case(s)) {
            return Err(novaui::i18n::t("err.no-play-plugin", &[("scheme", sc)]));
        }
    }
    if let Some(u) = path.to_str().filter(|u| crate::files::is_web_address(u)) {
        return Ok((address_source(u), Some(u.to_owned()), None));
    }
    Ok((Arc::new(PathAudioSourceFactory::new(path)), None, None))
}

/// Both resolved plugin addresses and ordinary HTTP(S) playlist entries use this path.
fn address_source(url: &str) -> Arc<dyn AudioSourceFactory> {
    if crate::plugins::client::is_url(url) {
        return Arc::new(crate::player_core::UrlAudioSourceFactory::new(url, Default::default()));
    }
    Arc::new(PathAudioSourceFactory::new(std::path::Path::new(url)))
}

/// What the stream an online song plays from says of itself: codec, rate, depth, bit rate,
/// container, size. Read once over its own connection, off the engine thread.
fn probe_stream(url: &str) -> Option<MediaTags> {
    if !crate::plugins::client::is_url(url) {
        return probe(std::path::Path::new(url));
    }
    #[cfg(windows)]
    {
        let src = crate::player_core::UrlAudioSourceFactory::new(url, Default::default()).open().ok()?;
        crate::player_core::probe_media_tags(src).ok()
    }
    #[cfg(not(windows))]
    crate::player_core::probe_path(std::path::Path::new(url))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: Duration,
    pub end: Option<Duration>,
}

impl Span {
    /// Where a song starts in its file: its span's start, else the file's (positions are the
    /// song's own; the player's are the file's).
    pub fn offset(span: Option<Span>) -> Duration {
        span.map_or(Duration::ZERO, |s| s.start)
    }

    /// How long the span lasts: its end less its start; unknown when it runs to the file's end.
    pub fn length(&self) -> Option<Duration> {
        self.end.map(|e| e.saturating_sub(self.start))
    }
}

/// The song to go on into when the one playing ends (the queue's next, the same one on repeat
/// one), as the UI hands it over: the version it will have, and what `Cmd::Open` would carry.
#[derive(Clone, Debug, PartialEq)]
pub struct NextSong {
    pub version: u64,
    pub path: PathBuf,
    pub span: Option<Span>,
    pub duration: Option<Duration>,
    /// How long it overlaps the end of the one playing (`AudioNext::crossfade`): zero follows
    /// gaplessly. A song left with less than that to play (a seek near its end) is followed
    /// gaplessly: the fade does not start under the listener.
    pub crossfade: Duration,
}

/// How long before the playing song's end the next one is resolved and handed to the player: a
/// plugin's address is asked for no sooner, so it has not expired when it plays.
const NEXT_LEAD: Duration = Duration::from_secs(30);

pub enum Cmd {
    /// Open `path` (optionally a span of it), start at `at` and play if `play`.
    Open {
        version: u64,
        path: PathBuf,
        span: Option<Span>,
        at: Duration,
        play: bool,
        duration: Option<Duration>,
        rate: Option<u32>,
    },
    Play,
    Pause,
    Seek(Duration),
    Volume(f32),
    /// Which ReplayGain value songs play at (the setting `replay-gain`), from now on.
    ReplayGain(ReplayGain),
    Stop,
    /// The song to go on into without a gap when this one ends (gapless), or none. When the
    /// sound goes on into it, the snapshot's `version` becomes the one it was handed with.
    Next(Option<NextSong>),
    /// The next song's source, resolved off the engine thread: sent by the engine to itself.
    NextResolved {
        version: u64,
        source: Result<Source, String>,
    },
    /// The player asked to be serviced (its device moved): sent by the player's own wake.
    Service,
    /// The sound device played on went (player-core's endpoint watch): the song is held paused
    /// where it was, and opened again on whatever device there is when play is pressed.
    DeviceLost,
    Quit,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// The version of the open the rest describes.
    pub version: u64,
    pub opening: bool,
    pub playing: bool,
    pub buffering: bool,
    /// Position at `at` (span-relative); advances with wall time while playing.
    pub position: Duration,
    pub at: Option<Instant>,
    pub duration: Option<Duration>,
    pub ended: bool,
    pub error: Option<String>,
    /// Seeks the worker has carried out: a snapshot with fewer than were sent predates the
    /// latest seek, and its position is not the one the user asked for.
    pub seeks: u64,
    /// An online song's stream as probed (resolution, container, size), once known.
    pub stream: Option<MediaTags>,
    /// The value of its plugin's option the song resolved at: (id, name).
    pub option: Option<(String, String)>,
    /// Plays asked for that found no sound device to play on (the song stays held paused where
    /// it was): counts up over the engine's life, each one a toast.
    pub no_device: u64,
    /// Times the engine paused the song by itself (its device went, or there was none): counts
    /// up over the engine's life; the UI's play intent follows each one.
    pub held: u64,
    /// The crossfade the song came in under: its first `fade_in` overlaps the end of the song
    /// before (zero: none, or a seek ended it).
    pub fade_in: Duration,
}

impl Snapshot {
    /// The engine waits for sound: opening the song, or buffering it.
    pub fn waiting(&self) -> bool {
        self.opening || self.buffering
    }

    /// A snapshot of a new song (`version`) carrying the engine's life counters.
    fn fresh(version: u64, seeks: u64, no_device: u64, held: u64) -> Snapshot {
        Snapshot { version, seeks, no_device, held, ..Default::default() }
    }

    /// Failure is terminal, but is never successful EOF. The UI must not interpret it as
    /// permission to advance the queue. Preserve the song, position and open version for retry.
    fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.still();
        self.at = None;
    }

    /// What the engine is doing with the song (`opening`, `playing`, `buffering`, `ended`)
    /// changes through these three and an open's start and end, never field by field.
    /// Nothing: not opening, not playing or loading, not at the end.
    fn still(&mut self) {
        self.opening = false;
        self.playing = false;
        self.buffering = false;
        self.ended = false;
    }

    /// Loading the song to play (`play`), or holding it where it is: not loading, nothing to
    /// hear.
    fn loading(&mut self, play: bool) {
        self.playing = play;
        self.buffering = play;
    }

    /// What the open player says it does; a song at its end does not play.
    fn doing(&mut self, playing: bool, buffering: bool, ended: bool) {
        self.playing = playing && !ended;
        self.buffering = buffering;
        self.ended = ended;
    }

    /// How far the crossfade the song came in under is (0 to 1): `None` once it is over, or
    /// without one.
    pub fn fade_progress(&self) -> Option<f32> {
        if self.fade_in.is_zero() {
            return None;
        }
        Some(self.position_now().as_secs_f32() / self.fade_in.as_secs_f32()).filter(|k| *k < 1.0)
    }

    pub fn position_now(&self) -> Duration {
        match (self.playing && !self.buffering, self.at) {
            (true, Some(at)) => {
                let p = self.position + at.elapsed();
                self.duration.map_or(p, |d| p.min(d))
            }
            _ => self.position,
        }
    }
}

pub struct Engine {
    tx: Sender<Cmd>,
    pub state: Arc<Mutex<Snapshot>>,
    /// Counts the sound devices' changes (one came, went, or the default moved): a device list
    /// shown is stale when it moved.
    pub devices: Arc<std::sync::atomic::AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    pub fn new(
        wake: Arc<dyn Fn() + Send + Sync>,
        accounts: Arc<crate::plugins::accounts::Accounts>,
        effects: crate::player_core::clap::Effects,
    ) -> Self {
        let (tx, rx) = channel();
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let st = state.clone();
        let devices: Arc<std::sync::atomic::AtomicU64> = Default::default();
        {
            // On Windows' notification thread: only a message and a wake.
            let (tx, devices, wake) = (Mutex::new(tx.clone()), devices.clone(), wake.clone());
            crate::player_core::watch_outputs(Arc::new(move |e| match e {
                crate::player_core::OutputEvent::DeviceLost => {
                    let _ = tx.locked().send(Cmd::DeviceLost);
                }
                crate::player_core::OutputEvent::DevicesChanged => {
                    devices.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    wake();
                }
            }));
        }
        #[allow(clippy::expect_used, reason = "at start, a process that cannot start a thread cannot run")]
        let thread = std::thread::Builder::new()
            .name("novaplayer-engine".into())
            .spawn({
                let poke = tx.clone();
                move || worker(rx, poke, st, wake, (accounts, effects))
            })
            .expect("engine thread");
        Engine { tx, state, devices, thread: Some(thread) }
    }
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.locked().clone()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Open {
    player: AudioPlayer,
    span: Option<Span>,
    /// What it was opened from, to open it again where it was (`Recover`).
    path: PathBuf,
    full: Option<Duration>,
    rate: Option<u32>,
    /// Read over a network (an address, a plugin's song, a share): a stream that breaks may come
    /// back.
    remote: bool,
    /// Whether this player has the next song (`AudioPlayer::set_next`), and the overlap it was
    /// given with.
    handed: Option<Duration>,
    /// The last song given to it, with its address and option: what plays once the player says
    /// it went on into it (`AudioPlayer::followed`), even if the UI has taken it back since.
    given: Option<Given>,
    /// `AudioPlayer::followed`, as last seen.
    followed: Option<u64>,
}

impl Open {
    fn new(
        player: AudioPlayer,
        path: PathBuf,
        span: Option<Span>,
        (full, rate, remote): (Option<Duration>, Option<u32>, bool),
    ) -> Self {
        Open { player, span, path, full, rate, remote, handed: None, given: None, followed: None }
    }

    /// What is left of the song, when its length is known.
    fn left(&self, now: Instant) -> Option<Duration> {
        let end = self.span.and_then(|s| s.end).or(self.player.duration())?;
        Some(end.saturating_sub(self.player.position(now)))
    }

    /// The overlap a next song asking for `crossfade` is given now: none when less than that is
    /// left of this one (it plays out, and the next follows gaplessly).
    fn overlap(&self, crossfade: Duration, now: Instant) -> Duration {
        if self.left(now).is_none_or(|left| left >= crossfade) {
            crossfade
        } else {
            Duration::ZERO
        }
    }
}

/// A next song given to a player: the song, the address it resolved to, its option.
type Given = (NextSong, Option<String>, Option<(String, String)>);

/// The next song, as the engine holds it: what the UI handed over, and its source once resolved.
struct Following {
    song: NextSong,
    asked: bool,
    source: Option<Source>,
}

/// A play found no device: counted, and the count published.
fn count_no_device(s: &mut Snapshot, n: &mut u64) {
    *n += 1;
    s.no_device = *n;
}

/// A song's length as the snapshot shows it: its span's, else the container's less the start.
fn shown_length(span: Option<Span>, full: Option<Duration>) -> Option<Duration> {
    span.and_then(|s| s.length()).or(full.map(|d| span.map_or(d, |s| d.saturating_sub(s.start)))).or(full)
}

/// Whether `path` is read over a network, `url` the address it was resolved to.
fn is_remote(path: &std::path::Path, url: Option<&String>) -> bool {
    url.is_some() || crate::player_core::SourceLocality::of_path(path) == crate::player_core::SourceLocality::Network
}

/// Probes an online song's stream for the info page, once, and publishes it if the song is still
/// the one at `version`.
fn probe_stream_for(st: &Arc<Mutex<Snapshot>>, wake: &Arc<dyn Fn() + Send + Sync>, version: u64, url: String) {
    let (st, wake) = (st.clone(), wake.clone());
    std::thread::spawn(move || {
        let Some(t) = probe_stream(&url) else { return };
        let mut s = st.locked();
        if s.version == version {
            s.stream = Some(t);
            drop(s);
            wake();
        }
    });
}

/// A song whose stream broke while it played (the network dropped, a packet was lost, a share
/// went away for a moment): the player is loading, not failed. It is opened again where it was,
/// after a growing pause, until it comes back or the tries are spent; then it fails as any open
/// does.
struct Recover {
    version: u64,
    path: PathBuf,
    span: Option<Span>,
    /// Span-relative, as `Cmd::Open`'s.
    at: Duration,
    play: bool,
    full: Option<Duration>,
    rate: Option<u32>,
    /// Read over a network: a play after a hold may wait out the connection as a broken stream does.
    remote: bool,
    tries: usize,
    /// The next try; `None` held: no device to play on, waiting for play to be pressed.
    next: Option<Instant>,
}

/// The pause before each try at coming back (the first is at once: an address may have only
/// expired): about half a minute in all.
const RECOVER_PAUSES_MS: [u64; 7] = [0, 1000, 2000, 3000, 5000, 8000, 10000];

/// Whether a playing stream's failure is the connection's (so: load and try again), rather than
/// the file's or the device's. Windows' untyped worker failures need the locality fallback;
/// Linux retains the error kind, so an HTTP input alone must not make a failure retryable.
fn connection_broke(e: &crate::player_core::AudioError, _remote: bool) -> bool {
    #[cfg(windows)]
    {
        use crate::player_core::AudioError as E;
        match e {
            E::Native { .. } | E::Worker(_) | E::WorkerStopped => _remote || e.is_network_failure(),
            _ => e.is_network_failure(),
        }
    }
    #[cfg(not(windows))]
    {
        e.is_network_failure()
    }
}

/// Resolves and opens `path`, seeks to `at` and plays when asked: the player, and whether it
/// reads over a network. `first`: the open the user asked for (an online song's stream is probed
/// once, for the info page). A failure says whether it was the sound device's (`Failed::device`).
#[allow(clippy::too_many_arguments)]
fn start(
    st: &Arc<Mutex<Snapshot>>,
    wake: &Arc<dyn Fn() + Send + Sync>,
    (accounts, effects): (&crate::plugins::accounts::Accounts, &crate::player_core::clap::Effects),
    player_wake: &Arc<dyn Fn() + Send + Sync>,
    (volume, replay_gain): (f32, ReplayGain),
    (version, path, span): (u64, &std::path::Path, Option<Span>),
    (at, play, full, rate): (Duration, bool, Option<Duration>, Option<u32>),
    first: bool,
) -> Result<(AudioPlayer, bool), Failed> {
    let (factory, url, q) = source_for(accounts, path).map_err(Failed::other)?;
    st.locked().option = q;
    let remote = is_remote(path, url.as_ref());
    if let (Some(url), true) = (url, first) {
        probe_stream_for(st, wake, version, url);
    }
    let effects = Some(effects.clone());
    let options = AudioOpenOptions { duration: full, source_rate: rate, replay_gain, effects, ..Default::default() };
    let mut p = AudioPlayer::open_with(factory, player_wake.clone(), options)
        .map_err(|e| Failed { device: e.is_device_failure(), text: e.to_string() })?;
    p.set_volume(volume);
    if let Some(s) = span {
        p.set_stop(s.end);
    }
    let from = Span::offset(span) + at;
    let started = (|| {
        if !from.is_zero() {
            p.seek(from).map_err(Failed::of)?;
        }
        if play {
            p.play().map_err(Failed::of)?;
        }
        Ok::<(), Failed>(())
    })();
    if let Err(e) = started {
        let _ = p.dispose();
        return Err(e);
    }
    Ok((p, remote))
}

/// Why `start` failed: the words, and whether the sound device was at fault (none, or one that
/// went) rather than the song.
struct Failed {
    text: String,
    device: bool,
}

impl Failed {
    fn other(text: String) -> Self {
        Failed { text, device: false }
    }
    fn of(e: crate::player_core::AudioError) -> Self {
        Failed { device: e.is_device_failure(), text: e.to_string() }
    }
}

/// Holds `s` paused at `at`: a held `Recover`'s snapshot.
/// An open and what was queued behind it → the commands to carry out, in order, and how many
/// seeks were let go. When a newer open is queued, the open and what came before the newest
/// are about songs already gone: of those only what is not about a song is kept (the volume,
/// ReplayGain, a lost device, the player's service, quitting); a seek let go is still counted,
/// as the UI counts the seeks it sent (`Snapshot::seeks`).
fn newest_open(open: Cmd, queued: Vec<Cmd>) -> (std::collections::VecDeque<Cmd>, u64) {
    let Some(last) = queued.iter().rposition(|c| matches!(c, Cmd::Open { .. })) else {
        return (std::iter::once(open).chain(queued).collect(), 0);
    };
    let mut seeks = 0;
    let mut out = std::collections::VecDeque::new();
    for (k, c) in queued.into_iter().enumerate() {
        let keep =
            k >= last || matches!(c, Cmd::Volume(_) | Cmd::ReplayGain(_) | Cmd::DeviceLost | Cmd::Service | Cmd::Quit);
        if keep {
            out.push_back(c);
        } else if matches!(c, Cmd::Seek(_)) {
            seeks += 1;
        }
    }
    (out, seeks)
}

fn hold(s: &mut Snapshot, at: Duration, held: &mut u64) {
    *held += 1;
    s.held = *held;
    s.position = at;
    s.at = Some(Instant::now());
    s.still();
}

fn worker(
    rx: Receiver<Cmd>,
    poke: Sender<Cmd>,
    st: Arc<Mutex<Snapshot>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    (accounts, effects): (Arc<crate::plugins::accounts::Accounts>, crate::player_core::clap::Effects),
) {
    let mut cur: Option<Open> = None;
    let mut recover: Option<Recover> = None;
    let mut following: Option<Following> = None;
    let resolved_tx = poke.clone();
    let mut volume = 1.0f32;
    let mut replay_gain = ReplayGain::Off;
    let mut last_pub: Option<(bool, bool, bool)> = None;
    let mut seeks = 0u64;
    let mut no_device = 0u64;
    let mut held = 0u64;
    let player_wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = poke.send(Cmd::Service);
    });
    // Commands taken off the channel ahead of their turn (`newest_open`).
    let mut backlog: std::collections::VecDeque<Cmd> = Default::default();
    loop {
        // Wait for a command or the player's wake (it announces its end, buffering, failures and
        // seeks landing), else only until the player names a deadline (a lost device's ladder,
        // the drained tail) or a broken stream's next try. Playing, nothing else wakes it.
        let now = Instant::now();
        // The next song is resolved once the playing one is within `NEXT_LEAD` of its end.
        let arm_at = match (&cur, &following) {
            (Some(o), Some(f)) if !f.asked && o.player.is_playing() => {
                Some(o.left(now).map_or(now, |left| now + left.saturating_sub(NEXT_LEAD)))
            }
            _ => None,
        };
        let deadline = cur.as_ref().and_then(|o| o.player.next_wake_at(now));
        let deadline = [deadline, recover.as_ref().and_then(|r| r.next), arm_at].into_iter().flatten().min();
        let cmd = match backlog.pop_front() {
            Some(c) => Ok(c),
            None => match deadline {
                Some(at) => rx.recv_timeout(at.saturating_duration_since(now)),
                None => rx.recv().map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected),
            },
        };
        // A burst of song changes: only the newest is opened (a plugin's resolve may take
        // seconds; the ones before it are already gone from the face).
        let cmd = match cmd {
            Ok(open @ Cmd::Open { .. }) => {
                let (carry, dropped_seeks) = newest_open(open, backlog.drain(..).chain(rx.try_iter()).collect());
                seeks += dropped_seeks;
                backlog = carry;
                backlog.pop_front().ok_or(std::sync::mpsc::RecvTimeoutError::Timeout)
            }
            other => other,
        };
        let mut changed = false;
        match cmd {
            Ok(Cmd::Quit) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Cmd::Open { version, path, span, at, play, duration, rate }) => {
                if let Some(mut o) = cur.take() {
                    let _ = o.player.dispose();
                }
                {
                    let mut s = st.locked();
                    *s = Snapshot { opening: true, duration, ..Snapshot::fresh(version, seeks, no_device, held) };
                }
                wake();
                recover = None;
                // The UI hands the new song's next over after this.
                following = None;
                let full = duration;
                let opened = start(
                    &st,
                    &wake,
                    (&accounts, &effects),
                    &player_wake,
                    (volume, replay_gain),
                    (version, &path, span),
                    (at, play, full, rate),
                    true,
                );
                match opened {
                    Ok((player, remote)) => {
                        cur = Some(Open::new(player, path, span, (full, rate, remote)));
                        let mut s = st.locked();
                        s.opening = false;
                        s.duration = shown_length(span, full);
                    }
                    Err(e) if e.device => {
                        // No device to play on: held where it was asked to start, opened again
                        // when play is pressed; a play asked for now says there is none.
                        let mut s = st.locked();
                        s.duration = shown_length(span, full);
                        hold(&mut s, at, &mut held);
                        if play {
                            count_no_device(&mut s, &mut no_device);
                        }
                        recover = Some(Recover {
                            version,
                            path,
                            span,
                            at,
                            play: false,
                            full,
                            rate,
                            remote: false,
                            tries: 0,
                            next: None,
                        });
                    }
                    Err(e) => st.locked().fail(e.text),
                }
                changed = true;
                last_pub = None;
            }
            Ok(Cmd::Service) => {}
            Ok(Cmd::Next(song)) => {
                if following.as_ref().map(|f| &f.song) != song.as_ref() {
                    following = song.map(|song| Following { song, asked: false, source: None });
                    if let Some(o) = cur.as_mut().filter(|o| o.handed.is_some()) {
                        o.player.set_next(None);
                        o.handed = None;
                    }
                }
            }
            Ok(Cmd::NextResolved { version, source }) => {
                if let Some(f) = following.as_mut().filter(|f| f.song.version == version) {
                    // One that does not resolve is not followed: the ordinary switch resolves it
                    // again, and says why if it still fails.
                    f.source = source.ok();
                }
            }
            Ok(Cmd::DeviceLost) => {
                // Whether the sound goes on elsewhere is the listener's to say: paused where it was.
                if let Some(mut o) = cur.take() {
                    let now = Instant::now();
                    let base = Span::offset(o.span);
                    let at = o.player.position(now).saturating_sub(base);
                    let _ = o.player.dispose();
                    let mut s = st.locked();
                    hold(&mut s, at, &mut held);
                    recover = Some(Recover {
                        version: s.version,
                        path: o.path,
                        span: o.span,
                        at,
                        play: false,
                        full: o.full,
                        rate: o.rate,
                        remote: o.remote,
                        tries: 0,
                        next: None,
                    });
                } else if let Some(r) = &mut recover {
                    r.play = false;
                    r.next = None;
                    hold(&mut st.locked(), r.at, &mut held);
                }
                changed = true;
                last_pub = None;
            }
            Ok(Cmd::Play) => {
                if let Some(o) = &mut cur {
                    if o.player.ended() {
                        let _ = o.player.seek(Span::offset(o.span));
                    }
                    let _ = o.player.play();
                    changed = true;
                } else if let Some(r) = &mut recover {
                    // Loading still, and wanting to play when it is back. Held (no device): one
                    // try now; a stream read over a network may wait out its connection too.
                    r.play = true;
                    if r.next.is_none() {
                        r.next = Some(Instant::now());
                        r.tries = if r.remote { 0 } else { RECOVER_PAUSES_MS.len() - 1 };
                    }
                    st.locked().loading(true);
                    changed = true;
                }
            }
            Ok(Cmd::Pause) => {
                if let Some(o) = &mut cur {
                    let _ = o.player.pause();
                    changed = true;
                } else if let Some(r) = &mut recover {
                    // Held where it was: not loading, nothing to hear.
                    r.play = false;
                    st.locked().loading(false);
                    changed = true;
                }
            }
            Ok(Cmd::Seek(p)) => {
                seeks += 1;
                if let Some(o) = &mut cur {
                    let base = Span::offset(o.span);
                    let _ = o.player.seek(base + p);
                    // A seek ends the crossfade the song came in under: the going one is not
                    // heard any more.
                    let mut s = st.locked();
                    s.ended = false;
                    s.fade_in = Duration::ZERO;
                    drop(s);
                    // A seek into the overlap with the next song, or back out of it: the next is
                    // given again, gaplessly or overlapping as is left now.
                    let now = Instant::now();
                    let asked = following.as_ref().map(|f| f.song.crossfade);
                    if let (Some(handed), Some(asked)) = (o.handed, asked) {
                        if o.overlap(asked, now) != handed {
                            o.handed = None;
                        }
                    }
                } else if let Some(r) = &mut recover {
                    r.at = p;
                    let mut s = st.locked();
                    s.position = p;
                    s.at = Some(Instant::now());
                }
                st.locked().seeks = seeks;
                changed = true;
            }
            Ok(Cmd::Volume(v)) => {
                volume = v;
                if let Some(o) = &cur {
                    o.player.set_volume(v);
                }
            }
            Ok(Cmd::ReplayGain(mode)) => {
                replay_gain = mode;
                if let Some(o) = &mut cur {
                    o.player.set_replay_gain(mode);
                }
            }
            Ok(Cmd::Stop) => {
                recover = None;
                following = None;
                if let Some(mut o) = cur.take() {
                    let _ = o.player.dispose();
                }
                let version = st.locked().version;
                *st.locked() = Snapshot::fresh(version, seeks, no_device, held);
                changed = true;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        // A stream that broke is opened again where it was (`Recover`).
        if let Some(r) = &mut recover {
            let now = Instant::now();
            if r.next.is_some_and(|t| now >= t) {
                let again = start(
                    &st,
                    &wake,
                    (&accounts, &effects),
                    &player_wake,
                    (volume, replay_gain),
                    (r.version, &r.path, r.span),
                    (r.at, r.play, r.full, r.rate),
                    false,
                );
                match again {
                    Ok((player, remote)) => {
                        let (at, play) = (r.at, r.play);
                        let path = std::mem::take(&mut r.path);
                        cur = Some(Open::new(player, path, r.span, (r.full, r.rate, remote)));
                        recover = None;
                        let mut s = st.locked();
                        s.position = at;
                        s.at = Some(Instant::now());
                        s.loading(play);
                        last_pub = None;
                    }
                    Err(e) if e.device => {
                        // No device: held where it was; a play that asked says there is none.
                        let mut s = st.locked();
                        hold(&mut s, r.at, &mut held);
                        if std::mem::take(&mut r.play) {
                            count_no_device(&mut s, &mut no_device);
                        }
                        r.next = None;
                    }
                    Err(e) => {
                        r.tries += 1;
                        if r.tries >= RECOVER_PAUSES_MS.len() {
                            recover = None;
                            st.locked().fail(e.text);
                        } else {
                            r.next = Some(Instant::now() + Duration::from_millis(RECOVER_PAUSES_MS[r.tries]));
                        }
                    }
                }
                changed = true;
            }
        }
        // The next song: resolved off this thread near the end, then handed to the player.
        if let (Some(o), Some(f)) = (&mut cur, &mut following) {
            let now = Instant::now();
            if !f.asked && o.player.is_playing() && o.left(now).is_none_or(|left| left <= NEXT_LEAD) {
                f.asked = true;
                let (tx, accounts, path, version) =
                    (resolved_tx.clone(), accounts.clone(), f.song.path.clone(), f.song.version);
                std::thread::spawn(move || {
                    let source = source_for(&accounts, &path);
                    let _ = tx.send(Cmd::NextResolved { version, source });
                });
            }
            if let (Some((factory, url, q)), None) = (&f.source, o.handed) {
                let crossfade = o.overlap(f.song.crossfade, now);
                o.given = Some((f.song.clone(), url.clone(), q.clone()));
                o.player.set_next(Some(AudioNext {
                    id: f.song.version,
                    factory: factory.clone(),
                    track: Default::default(),
                    duration: f.song.duration,
                    start: Span::offset(f.song.span),
                    stop: f.song.span.and_then(|s| s.end),
                    crossfade,
                }));
                o.handed = Some(crossfade);
            }
        }
        if let Some(o) = &mut cur {
            let now = Instant::now();
            if let Err(e) = o.player.service(now) {
                // The connection dropping is loading, not failing; anything else is final. Do
                // not overwrite a failed service with the decoder's ended flag below.
                let broke = connection_broke(&e, o.remote);
                let device = e.is_device_failure();
                let (path, span, full, rate, remote) = (o.path.clone(), o.span, o.full, o.rate, o.remote);
                if let Some(mut failed) = cur.take() {
                    let _ = failed.player.dispose();
                }
                let mut s = st.locked();
                if device && !broke {
                    // The device went and none took it: held paused where it was.
                    let at = s.position_now();
                    hold(&mut s, at, &mut held);
                    recover = Some(Recover {
                        version: s.version,
                        path,
                        span,
                        at,
                        play: false,
                        full,
                        rate,
                        remote,
                        tries: 0,
                        next: None,
                    });
                } else if broke {
                    // A failed backend may already have stopped itself. Keep the last published
                    // play intent so recovery immediately shows loading instead of a paused bar.
                    let play = s.playing;
                    let at = s.position_now();
                    recover = Some(Recover {
                        version: s.version,
                        path,
                        span,
                        at,
                        play,
                        full,
                        rate,
                        remote,
                        tries: 0,
                        next: Some(now),
                    });
                    s.position = at;
                    s.at = Some(now);
                    s.loading(play);
                    s.ended = false;
                } else {
                    s.fail(e.to_string());
                }
                drop(s);
                wake();
                continue;
            }
            // The sound went on into the next song: it is the one playing, under the version the
            // UI handed it over with.
            if o.player.followed() != o.followed {
                o.followed = o.player.followed();
                let overlap = o.handed.take().unwrap_or_default();
                let given = o.given.take().filter(|(song, _, _)| Some(song.version) == o.followed);
                if let Some((song, url, q)) = given {
                    if following.as_ref().is_some_and(|f| f.song == song) {
                        following = None;
                    }
                    o.remote = is_remote(&song.path, url.as_ref());
                    (o.path, o.span, o.full, o.rate) = (song.path, song.span, song.duration, None);
                    let mut s = st.locked();
                    s.version = song.version;
                    s.duration = shown_length(o.span, o.full);
                    s.option = q;
                    s.stream = None;
                    s.ended = false;
                    s.fade_in = overlap;
                    drop(s);
                    if let Some(url) = url {
                        probe_stream_for(&st, &wake, song.version, url);
                    }
                    last_pub = None;
                    changed = true;
                }
            }
            let base = Span::offset(o.span);
            let pos = o.player.position(now).saturating_sub(base);
            let (playing, buffering, ended) = (o.player.is_playing(), o.player.buffering(), o.player.ended());
            let mut s = st.locked();
            s.position = pos;
            s.at = Some(now);
            s.doing(playing, buffering, ended);
            if last_pub != Some((s.playing, buffering, ended)) {
                last_pub = Some((s.playing, buffering, ended));
                changed = true;
            }
        }
        if changed {
            wake();
        }
    }
    if let Some(mut o) = cur.take() {
        let _ = o.player.dispose();
    }
}

/// Reads tags (blocking; run off the UI thread).
#[cfg(windows)]
pub fn probe(path: &std::path::Path) -> Option<MediaTags> {
    if let Some(t) = plugin_probe(path) {
        return Some(t);
    }
    let src = crate::player_core::FileMediaByteSource::open(path).ok()?;
    crate::player_core::probe_media_tags(Box::new(src)).ok()
}
#[cfg(not(windows))]
pub fn probe(path: &std::path::Path) -> Option<MediaTags> {
    if let Some(t) = plugin_probe(path) {
        return Some(t);
    }
    crate::player_core::probe_path(path)
}

/// Reads only the words and the length (no picture): what a sort needs.
#[cfg(windows)]
pub fn probe_tags(path: &std::path::Path) -> Option<MediaTags> {
    probe(path)
}
#[cfg(not(windows))]
pub fn probe_tags(path: &std::path::Path) -> Option<MediaTags> {
    if let Some(t) = plugin_probe(path) {
        return Some(t);
    }
    crate::player_core::probe_tags_only(path)
}

/// What a decode plugin says of a file it claims (its `open`).
fn plugin_probe(path: &std::path::Path) -> Option<MediaTags> {
    let id = crate::plugins::pcm::decoder_for(path)?;
    let s = crate::plugins::pcm::PcmSource::open(&id, path).ok()?;
    Some(MediaTags::from(&s.info.tags))
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    /// Five quick presses of Next queue five opens: only the newest is carried out, after what
    /// is not about a song; what came after it stays; the seeks let go are counted.
    #[test]
    fn a_burst_of_opens_carries_out_the_newest_only() {
        let open = |version| Cmd::Open {
            version,
            path: PathBuf::from(format!("song{version}")),
            span: None,
            at: Duration::ZERO,
            play: true,
            duration: None,
            rate: None,
        };
        let queued = vec![
            open(2),
            Cmd::Seek(Duration::from_secs(3)),
            Cmd::Volume(0.5),
            open(3),
            open(4),
            Cmd::Pause,
            open(5),
            Cmd::Seek(Duration::ZERO),
        ];
        let (carry, seeks) = newest_open(open(1), queued);
        let what: Vec<String> = carry
            .iter()
            .map(|c| match c {
                Cmd::Open { version, .. } => format!("open {version}"),
                Cmd::Volume(_) => "volume".into(),
                Cmd::Seek(_) => "seek".into(),
                _ => "other".into(),
            })
            .collect();
        assert_eq!(what, ["volume", "open 5", "seek"]);
        assert_eq!(seeks, 1, "the seek before the newest open is counted");
        let (alone, _) = newest_open(open(1), vec![Cmd::Pause]);
        assert_eq!(alone.len(), 2, "a single open is carried out as asked");
    }

    #[test]
    fn failure_is_not_end_of_track() {
        let mut s = Snapshot {
            version: 7,
            opening: true,
            playing: true,
            buffering: true,
            ended: true,
            position: Duration::from_secs(11),
            at: Some(Instant::now()),
            ..Default::default()
        };
        s.fail("decoder failed".into());
        assert_eq!(s.version, 7);
        assert_eq!(s.position_now(), Duration::from_secs(11));
        assert!(!s.ended && !s.opening && !s.playing && !s.buffering);
        assert_eq!(s.error.as_deref(), Some("decoder failed"));
    }

    #[cfg(not(windows))]
    #[test]
    fn linux_http_addresses_open_without_a_local_file() {
        for url in ["https://audio.invalid/song.mp3?token=a%2Fb+cd&v=1", "http://127.0.0.1:9/song.wav?token=abc"] {
            // The same factory used after plugin resolve; no external network needed when
            // the playlist already supplied duration, as in the reported failure.
            let resolved = address_source(url);
            assert!(
                matches!(resolved.input(), Some(crate::player_core::AudioInput::Url(u, _)) if u == url),
                "the address goes to ffmpeg verbatim"
            );
            let direct = source_for(&Default::default(), std::path::Path::new(url)).unwrap();
            assert_eq!(direct.1.as_deref(), Some(url));
            for factory in [resolved, direct.0] {
                let p = AudioPlayer::open(factory, Default::default(), Arc::new(|| {}), Some(Duration::from_secs(3)));
                assert!(p.is_ok(), "HTTP(S) is not a filesystem path: {:?}", p.err());
            }
        }
        let dir = std::env::temp_dir().join(format!("np-missing-http-test-{}", std::process::id()));
        let missing = dir.join("missing.wav");
        assert!(!missing.exists());
        let e = AudioPlayer::open(
            Arc::new(PathAudioSourceFactory::new(&missing)),
            Default::default(),
            Arc::new(|| {}),
            Some(Duration::from_secs(3)),
        )
        .err()
        .expect("missing local files must still fail");
        assert!(e.to_string().contains("file not found"));
    }

    #[test]
    fn an_unregistered_uri_is_not_a_local_file() {
        crate::app::speak_zh_cn();
        for p in ["unregistered-test:song:1", "plugin:unregistered-test/unregistered-test:song:1"] {
            let e = match source_for(&Default::default(), std::path::Path::new(p)) {
                Err(e) => e,
                Ok(_) => panic!("must reject a URI with no resolver: {p}"),
            };
            assert!(e.contains("插件"), "{e}");
        }
        assert!(
            source_for(&Default::default(), std::path::Path::new(r"C:\Music\song.flac")).is_ok(),
            "a drive is not a URI scheme"
        );
    }
}

#[cfg(all(test, not(windows)))]
mod http_tests;
