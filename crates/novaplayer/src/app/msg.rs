//! The player's messages (`novaui` `Event::Message`): what a waker, timer, ticker or
//! subscription of the player delivers. A message names the owner that asked for it
//! (`novaui/docs/runtime.md`, "Kinds of work, and who owns them").
//!
//! Every message but `Present` marks the presentation changed (`Player::event`), so the views
//! present once, in the same turn, after everything the turn delivered (the flush). A message
//! that only names a view's frame or deadline therefore needs no handler of its own: that view's
//! present does the work and arms the handle again while work is left.

/// One message per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
    /// The presentation changed: the views present (a subscription, delivered in the flush).
    Present,
    /// The face (`now_playing.track`) changed: the layout's facts are read again.
    Track,
    /// The plugin controls' context moved: the side panel's shown page (`side_panel.shown`),
    /// what plays or whether it plays (`playback.state`), the face (`now_playing.track`). The
    /// controls follow at once (`Player::sync_plugin_controls`).
    Controls,
    /// Another thread reported something (`app::jobs`, the inbox): the job channel has it.
    Jobs,
    /// A sign-in's next poll is due (`plugins::connect`).
    Connect,
    /// The engine published a new snapshot (end, error, a stream, a device move...).
    Engine,
    /// A kept file is due to be written (`kept::Keeping`).
    Keep,
    /// The effects chain changed (`Effects::watch`: a parameter, an effect's state, a failure,
    /// on any thread).
    Effects,
    /// A panic on another thread asks for the save `Drop` would have made (`crate::crash`).
    Rescue,
    /// A plugin's files have been hashed on a worker (`plugins::prints`): the registry rescans.
    PluginsChecked,
    /// The stand-in's window after a song change is over (`stand_in_track`).
    StandIn,
    /// The document's tweens (a layout tween) move: one frame each.
    Layout,
    /// The side panel: its opening, page turns, the queue's, the lyrics' and the info page's
    /// glides, the give-way, autoscroll, lyric motion and sweep (frames); the next lyric line, the end of browsing, a thumbnail retry
    /// (deadlines).
    SidePanel,
    /// The now-playing face: the crossfade (frames), the backdrop's drift (10 Hz).
    NowPlaying,
    /// The control bar: the loading sweep (frames), the clock (1 Hz), the pointer reveal and the
    /// volume tip (deadlines).
    Transport,
    /// The toast's end.
    Toast,
    /// The flyouts: an exit's end, the drag target's hide, their scroll's and the import list's
    /// glides (frames).
    Menus,
    /// The settings page: the crossfade with the face, its scroll's glide (frames).
    Settings,
}

/// What the system answered to a question the player asked it (`Ui::pick`,
/// `Ui::read_clipboard`): each says who asked, with what the answer finishes.
#[derive(Clone, Debug)]
pub enum Answer {
    /// Songs or folders to add (`AddFiles`, `AddFolder`), or playlists to import (`Import`:
    /// `lists`).
    Add { paths: Vec<std::path::PathBuf>, lists: bool },
    /// Where to export `songs`, the list shown when it was asked.
    Export { path: Option<std::path::PathBuf>, songs: Vec<crate::library::Song> },
    /// A plugin package to install.
    PluginPackage(Option<std::path::PathBuf>),
    /// A lyric style to install (a package, or a folder picked by its `style.json`).
    LyricStyle(Option<std::path::PathBuf>),
    /// A picture for `song`'s folder.
    Cover { song: Box<crate::library::Song>, image: Option<std::path::PathBuf> },
    /// The clipboard's text for `ImportLink`.
    ImportLink(Option<String>),
    /// The clipboard's text for the playlist menu's "import link" row.
    MenuLink(Option<String>),
}
