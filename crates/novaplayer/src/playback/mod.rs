pub mod effects;
pub mod engine;
pub mod gain;
pub mod output;
pub mod session;

/// `Player` state owned by this area.
pub struct Playback {
    /// The end of the stand-in's window (`stand_in_track`).
    pub stand_in_clock: novaui::Clock,
    pub engine: engine::Engine,
    /// The open song's requests (its face and picture, its option values): another song drops
    /// them.
    pub tasks: crate::app::Tasks,
    pub snap: engine::Snapshot,
    pub song_version: u64,
    /// The last version given out, to a song opened or to one handed over to follow: one counter,
    /// so no two songs ever share one.
    pub last_version: u64,
    /// The songs handed to the engine to follow the one playing (`Cmd::Next`), newest last, with
    /// their place in the playing list: the sound may go on into one the list has since moved
    /// past, and it is still shown as it plays. Cleared when a song opens.
    pub handed: Vec<(usize, engine::NextSong)>,
    /// Whether the last of `handed` is still the one the engine has (not taken back).
    pub handing: bool,
    /// A song is open (shown, handed to the engine). With `wants_play` and `snap.playing`, it
    /// changes only through the transitions below (`open_as`, `close`, `rest`, `resume`).
    pub opened: bool,
    /// Notified when what plays or whether it plays changes (a song opens, `opened`,
    /// `snap.playing`): whoever shows it follows (the plugin controls' context, `Msg::Controls`).
    /// `snap` itself is taken from the engine every turn; this is its change.
    pub state: novaui::Model<()>,
    /// The listener's word: the open song is to play (it may still be opening).
    pub wants_play: bool,
    /// Length known from tags when the engine does not report one.
    pub duration_hint: Option<std::time::Duration>,
    /// Seeks sent to the engine (see `Snapshot::seeks`).
    pub seeks_sent: u64,
    /// The open whose error was already shown.
    pub error_shown: Option<u64>,
    /// Songs given at another value of their plugin's option than the user chose, told once.
    pub fallback: crate::plugins::client::option::Fallback,
    pub volume: f32,
    pub kept_gain: crate::playback::gain::KeptGain,
    /// The CLAP effects the sound plays through (Settings › Effects).
    pub effects: crate::playback::effects::EffectsModel,
    /// Set by set_volume; the next update stamps it with the clock and schedules the write.
    pub gain_touched: bool,
    pub output: crate::playback::output::Output,
    pub output_note: Option<String>,
    /// The engine's `Snapshot::no_device` already told.
    pub no_device_told: u64,
    /// The engine's `Snapshot::held` already followed.
    pub held_seen: u64,
    /// The engine's `devices` the output flyout was last listed at.
    pub devices_listed: u64,
    /// What the views show instead of the engine's position, in the L3 scenarios: a screenshot
    /// must not depend on how long a step took.
    #[cfg(any(test, feature = "test-support"))]
    pub pinned_position: Option<std::time::Duration>,
}

impl Playback {
    /// A song opened, to play or not as asked.
    pub(crate) fn open_as(&mut self, play: bool) {
        self.opened = true;
        self.wants_play = play;
        self.state.notify();
    }

    /// Nothing is open any more: stopped, or its open failed.
    pub(crate) fn close(&mut self) {
        self.opened = false;
        self.snap.playing = false;
        self.wants_play = false;
        self.state.notify();
    }

    /// The open song rests: paused by the listener, at the list's end, or held by the engine
    /// (its device went, there was none).
    pub(crate) fn rest(&mut self) {
        self.snap.playing = false;
        self.wants_play = false;
        self.state.notify();
    }

    /// The open song plays again at the listener's word.
    pub(crate) fn resume(&mut self) {
        self.snap.playing = true;
        self.wants_play = true;
        self.state.notify();
    }

    /// The engine's latest snapshot is about the song open now (not one before it).
    pub fn snap_is_open(&self) -> bool {
        self.snap.version == self.song_version
    }

    /// A version no song has had: past the last given out and the one showing.
    pub fn new_version(&mut self) -> u64 {
        self.last_version = self.last_version.max(self.song_version) + 1;
        self.last_version
    }

    /// The position the views show (the transport, the lyrics): the engine's, extrapolated to
    /// now. What is saved or sent (a play event, the restore point) reads `snap` itself.
    pub fn shown_position(&self) -> std::time::Duration {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(pinned) = self.pinned_position.filter(|_| self.opened) {
            return pinned;
        }
        self.snap.position_now()
    }
}
