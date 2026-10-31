//! What a press, a key or a field asks the app to do: one `Command`, one text form (`FromStr` /
//! `Display`: the sheet's `on-click`, plugin fragments, rows built in Rust), one `run`.
use crate::app::Answer;
use crate::app::{Job, Landed, Player};
use crate::library::queue::Sort;
use crate::library::store::Origin;
use crate::player_core::{AudioOutputHost, AudioOutputRoute};
use crate::views::import_dialog::*;
use crate::views::input::{back_panel, row_place};
use crate::views::menus::*;
use crate::views::menus::{app_menu::*, output::*, playlist::*, sort::*};
use crate::views::settings_page::Level;
use crate::views::side_panel::{Narrow, Page};
use novaui::*;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

/// What of the playing song a 复制 puts on the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyWhat {
    Title,
    Artist,
    Album,
    /// `title - artist`.
    TitleArtist,
    /// A lyric group: its lines and the layers shown under them.
    Line(usize),
    /// Every group so.
    Lyrics,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    // Playback and the face.
    PlayPause,
    Prev,
    Next,
    Mute,
    Order,
    /// Seek by this many seconds from where it plays.
    SeekBy(i32),
    /// The volume by this many hundredths.
    VolumeBy(i32),
    Fullscreen,
    /// The desktop lyrics come up or go (the setting `desktop-lyrics`).
    DesktopLyrics,
    ToggleLyrics,
    ToggleQueue,
    ToggleInfo,
    /// The side panel opens or closes.
    TogglePanel,
    ExpandPanel,
    ClearSearch,
    Locate,
    PageBack,
    Minimize,
    Maximize,
    Close,
    /// Songs, or a folder's songs, chosen in a dialog join the open playlist (never a new one).
    AddFiles,
    AddFolder,
    /// A lyrics layer (`layer:<id>`) shows or goes.
    Layer(String),
    // A queue row's song (`row:<pos>` …; pressed on a row, the verb alone: its song).
    PlayRow(usize),
    ToTop(usize),
    Remove(usize),
    RowArtist(usize),
    RowAlbum(usize),
    /// The row menu's playlist picker beside it (`row-pick:<pos>:<1 moves>`): pressed again, it goes.
    RowPick(usize, bool),
    /// 添加到 / 移动到 a playlist (`add-row:<pos>:<id|new>`): `None`, a new one.
    AddRow(usize, Option<u32>),
    MoveRow(usize, Option<u32>),
    /// Multi-select on or off (`select`, the toggle beside the search; Shift+Ctrl+A).
    Select,
    /// Every row shown chosen, or none when they all are (`select-all`; Ctrl+A while selecting).
    SelectAll,
    /// The chosen rows' menu's playlist picker (`sel-pick:<1 moves>`): pressed again, it goes.
    SelPick(bool),
    /// The chosen songs 添加到 / 移动到 a playlist (`add-sel:<id|new>`): `None`, a new one.
    AddSel(Option<u32>),
    MoveSel(Option<u32>),
    /// The chosen songs leave the list (`remove-sel`): the first press asks, the second does it.
    RemoveSel,
    /// The row's song on the info page.
    RowInfo(usize),
    /// A lyric line (its group): playback goes to it.
    Lyric(usize),
    /// The song's words' menu: 复制 › opens what to copy beside it (`copy-pick`).
    CopyPick,
    /// Puts words of the playing song on the clipboard (`copy:<what>`).
    Copy(CopyWhat),
    // Flyouts.
    PlaylistMenu,
    SortMenu,
    AppMenu,
    OptionMenu,
    /// A settings dropdown's list (`drop:<key>`).
    Drop(String),
    // The playlist flyout.
    Sort(Sort, bool),
    Switch(u32),
    Delete(u32),
    /// A carried row's drop target: the playlist it joins (`menus::drag`; the drop, not a press).
    AddTo(u32),
    New,
    Rename,
    ImportLink,
    Import,
    Export,
    Rescan,
    // The import dialog.
    ImportCancel,
    ImportAll,
    ImportOk,
    ImportToggle(usize),
    // Plugins.
    /// Its 启用 switch: off, or on after a second press (`Confirm::EnablePlugin`).
    PluginSwitch(String),
    /// Its flyout beside the app flyout opens (or, open, closes).
    PluginFlyout(String),
    Connect(String),
    ConnectCancel(String),
    Disconnect(String),
    PluginDelete(String),
    PluginInstall,
    /// Install a lyric style package (`app::lyric_styles`) and choose it.
    LyricStyleInstall,
    /// Delete the chosen installed lyric style; the built-in one is chosen.
    LyricStyleDelete,
    PickLists(String),
    OnlineCacheClear,
    PluginReload,
    PluginFolder,
    /// A plugin control's press (`contrib:<plugin>:<verb> <arg>`, `plugins::controls`).
    Plugin {
        id: String,
        verb: String,
        arg: String,
    },
    // Output and the song's option.
    Route(AudioOutputRoute),
    OutSys(AudioOutputHost),
    /// DSD over DoP on or off (`out-dop:on`, `out-dop:off`).
    OutDop(bool),
    SetOption(String),
    // The artwork.
    CoverChange,
    CoverRestore,
    // The settings page.
    /// The page at `level` (opened there when it is not up).
    Settings(Level),
    /// An effect of the chain at its place, or a found one to add (`effect:<verb>:<at>`,
    /// `playback::effects`).
    Effect {
        verb: crate::playback::effects::EffectVerb,
        at: usize,
    },
    /// Scans for the effects that can be added again.
    EffectsScan,
    /// Parameter `id` of the effect at `at` in the chain set to `value`
    /// (`param:<at>:<id>:<value>`).
    Param {
        at: usize,
        id: u32,
        value: f64,
    },
    /// The choices of a parameter with a few, as a dropdown's list (`param-drop:<at>:<id>`).
    ParamDrop {
        at: usize,
        id: u32,
    },
    /// A dropdown setting's choice (`set:<key>:<value>`).
    Set {
        key: String,
        value: String,
    },
    // Fields: what their text is for (`on-edit`; carried by Edit / Submit / Blur).
    /// The search box.
    Search,
    /// Field `k` of the connection form.
    Form(usize),
    /// A dropdown box that takes typing.
    DropTyped(String),
    /// The playlist flyout's name box.
    ListName,
}

fn num<T: FromStr>(s: &str) -> Option<T> {
    s.parse().ok()
}

/// A press that waits for its second: the first asked, the second does it. Every other command
/// forgets it (`run`).
#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    DeletePlaylist(u32),
    /// The chosen songs leave the list (`RemoveSel`).
    RemoveSelected,
    EnablePlugin(String),
}

impl Command {
    /// The command of an element's action; `tag` is the pressed element's place (a queue row's
    /// position, a lyric's group) for the verbs that name a row without saying which.
    pub fn of(a: &Action, tag: impl FnOnce() -> Option<usize>) -> Option<Self> {
        use Command::*;
        let (verb, arg) = (a.name.as_str(), a.arg.as_str());
        let at = |tag: Option<usize>| if arg.is_empty() { tag } else { arg.parse().ok() };
        let s = || arg.to_owned();
        Some(match verb {
            "play-pause" => PlayPause,
            "prev" => Prev,
            "next" => Next,
            "mute" => Mute,
            "order" => Order,
            "seek-by" => SeekBy(num(arg)?),
            "volume-by" => VolumeBy(num(arg)?),
            "fullscreen" => Fullscreen,
            "desktop-lyrics" => DesktopLyrics,
            "toggle-lyrics" => ToggleLyrics,
            "toggle-queue" => ToggleQueue,
            "toggle-info" => ToggleInfo,
            "toggle-panel" => TogglePanel,
            "expand-panel" => ExpandPanel,
            "clear-search" => ClearSearch,
            "locate" => Locate,
            "page-back" => PageBack,
            "minimize" => Minimize,
            "maximize" => Maximize,
            "close" => Close,
            "add-files" => AddFiles,
            "add-folder" => AddFolder,
            "layer" if !arg.is_empty() => Layer(s()),
            "row" => PlayRow(at(tag())?),
            "to-top" => ToTop(at(tag())?),
            "row-remove" | "remove" => Remove(at(tag())?),
            "row-artist" => RowArtist(at(tag())?),
            "row-album" => RowAlbum(at(tag())?),
            "row-pick" => {
                let (k, mv) = arg.split_once(':')?;
                RowPick(num(k)?, mv == "1")
            }
            "add-row" | "move-row" => {
                let (k, to) = arg.split_once(':')?;
                let to = if to == "new" { None } else { Some(num(to)?) };
                if verb == "add-row" {
                    AddRow(num(k)?, to)
                } else {
                    MoveRow(num(k)?, to)
                }
            }
            "select" => Select,
            "select-all" => SelectAll,
            "sel-pick" => SelPick(arg == "1"),
            "add-sel" | "move-sel" => {
                let to = if arg == "new" { None } else { Some(num(arg)?) };
                if verb == "add-sel" {
                    AddSel(to)
                } else {
                    MoveSel(to)
                }
            }
            "remove-sel" => RemoveSel,
            "row-info" => RowInfo(at(tag())?),
            "lyric" => Lyric(at(tag())?),
            "copy-pick" => CopyPick,
            "copy" => Copy(match arg.split_once(':') {
                Some(("line", g)) => CopyWhat::Line(num(g)?),
                _ => match arg {
                    "title" => CopyWhat::Title,
                    "artist" => CopyWhat::Artist,
                    "album" => CopyWhat::Album,
                    "title-artist" => CopyWhat::TitleArtist,
                    "lyrics" => CopyWhat::Lyrics,
                    _ => return None,
                },
            }),
            "playlist-menu" => PlaylistMenu,
            "sort-menu" => SortMenu,
            "app-menu" => AppMenu,
            "option-menu" => OptionMenu,
            "drop" if !arg.is_empty() => Drop(s()),
            "sort" => {
                let (k, d) = arg.split_once(':').unwrap_or((arg, "0"));
                Sort(crate::library::queue::Sort::from_key(k)?, d == "1")
            }
            "switch" => Switch(num(arg)?),
            "delete" => Delete(num(arg)?),
            "add-to" => AddTo(num(arg)?),
            "new" => New,
            "rename" => Rename,
            "import-link" => ImportLink,
            "import" => Import,
            "export" => Export,
            "rescan" => Rescan,
            "import-cancel" => ImportCancel,
            "import-all" => ImportAll,
            "import-ok" => ImportOk,
            "import-toggle" => ImportToggle(num(arg)?),
            "plugin-switch" => PluginSwitch(s()),
            "plugin-flyout" => PluginFlyout(s()),
            "connect" => Connect(s()),
            "connect-cancel" => ConnectCancel(s()),
            "disconnect" => Disconnect(s()),
            "plugin-delete" => PluginDelete(s()),
            "plugin-install" => PluginInstall,
            "lyric-style-install" => LyricStyleInstall,
            "lyric-style-delete" => LyricStyleDelete,
            "pick-lists" => PickLists(s()),
            "online-cache-clear" => OnlineCacheClear,
            "plugin-reload" => PluginReload,
            "plugin-folder" => PluginFolder,
            "effect" => {
                let (verb, at) = arg.split_once(':')?;
                Effect { verb: crate::playback::effects::EffectVerb::of(verb)?, at: at.parse().ok()? }
            }
            "effects-scan" => EffectsScan,
            "param" => {
                let mut parts = arg.splitn(3, ':');
                let (at, id, value) = (parts.next()?, parts.next()?, parts.next()?);
                Param { at: at.parse().ok()?, id: id.parse().ok()?, value: value.parse().ok()? }
            }
            "param-drop" => {
                let (at, id) = arg.split_once(':')?;
                ParamDrop { at: at.parse().ok()?, id: id.parse().ok()? }
            }
            "contrib" => {
                let whole = a.to_string();
                let (id, verb, arg) = crate::plugins::controls::parse_action(&whole)?;
                Plugin { id: id.to_owned(), verb: verb.to_owned(), arg: arg.to_owned() }
            }
            "route" => Route(match arg.split_once(':') {
                Some(("wasapi", id)) => AudioOutputRoute::Wasapi(id.to_owned()),
                Some(("asio", id)) => AudioOutputRoute::Asio(id.to_owned()),
                None if arg == "default" => AudioOutputRoute::WasapiDefault,
                _ => return None,
            }),
            "out-sys" => OutSys(match arg {
                "asio" => AudioOutputHost::Asio,
                "wasapi" => AudioOutputHost::Wasapi,
                _ => return None,
            }),
            "out-dop" => OutDop(match arg {
                "on" => true,
                "off" => false,
                _ => return None,
            }),
            "option" => SetOption(s()),
            "cover-change" => CoverChange,
            "cover-restore" => CoverRestore,
            "settings" => Settings(match arg.split_once(':') {
                Some(("plugin", id)) => Level::Plugin(id.to_owned()),
                Some(("effect", at)) => Level::Effect(at.parse().ok()?),
                None if arg == "plugins" => Level::Plugins,
                None if arg == "effects" => Level::Effects,
                None if arg == "desktop-lyrics" => Level::DesktopLyrics,
                None if arg.is_empty() => Level::Root,
                _ => return None,
            }),
            "set" => {
                let (key, value) = arg.split_once(':')?;
                Set { key: key.to_owned(), value: value.to_owned() }
            }
            "search" => Search,
            "form" => Form(num(arg)?),
            "drop-typed" if !arg.is_empty() => DropTyped(s()),
            "list-name" => ListName,
            _ => return None,
        })
    }

    /// The flyouts stay up for these: they open one, rebuild it in place, or close it later.
    fn keeps_flyout(&self) -> bool {
        use Command::*;
        matches!(
            self,
            PlaylistMenu
                | SortMenu
                | AppMenu
                | OptionMenu
                | Drop(_)
                | ParamDrop { .. }
                | Delete(_)
                | Rename
                | New
                | OutSys(_)
                | OutDop(_)
                | Route(_)
                | PluginSwitch(_)
                | PluginFlyout(_)
                | RowPick(..)
                | SelPick(_)
                | RemoveSel
                | CopyPick
                | Connect(_)
                | ConnectCancel(_)
                | Disconnect(_)
                | PluginDelete(_)
                | PluginReload
                | Search
                | Form(_)
                | DropTyped(_)
                | ListName
        )
    }
}

impl FromStr for Command {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Command::of(&Action::parse(s), || None).ok_or_else(|| format!("not a command: {s}"))
    }
}

impl fmt::Display for Command {
    /// The command's text, as `Command::of` reads it back (`a_command_reads_back_as_written`).
    /// Every variant is written here: a new one does not compile until it is.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Command::*;
        match self {
            PlayPause => f.write_str("play-pause"),
            Prev => f.write_str("prev"),
            Next => f.write_str("next"),
            Mute => f.write_str("mute"),
            Order => f.write_str("order"),
            Fullscreen => f.write_str("fullscreen"),
            DesktopLyrics => f.write_str("desktop-lyrics"),
            ToggleLyrics => f.write_str("toggle-lyrics"),
            ToggleQueue => f.write_str("toggle-queue"),
            ToggleInfo => f.write_str("toggle-info"),
            TogglePanel => f.write_str("toggle-panel"),
            ExpandPanel => f.write_str("expand-panel"),
            ClearSearch => f.write_str("clear-search"),
            Locate => f.write_str("locate"),
            PageBack => f.write_str("page-back"),
            Minimize => f.write_str("minimize"),
            Maximize => f.write_str("maximize"),
            Close => f.write_str("close"),
            AddFiles => f.write_str("add-files"),
            AddFolder => f.write_str("add-folder"),
            PlaylistMenu => f.write_str("playlist-menu"),
            SortMenu => f.write_str("sort-menu"),
            AppMenu => f.write_str("app-menu"),
            OptionMenu => f.write_str("option-menu"),
            New => f.write_str("new"),
            Rename => f.write_str("rename"),
            ImportLink => f.write_str("import-link"),
            Import => f.write_str("import"),
            Export => f.write_str("export"),
            Rescan => f.write_str("rescan"),
            ImportCancel => f.write_str("import-cancel"),
            ImportAll => f.write_str("import-all"),
            ImportOk => f.write_str("import-ok"),
            PluginInstall => f.write_str("plugin-install"),
            LyricStyleInstall => f.write_str("lyric-style-install"),
            LyricStyleDelete => f.write_str("lyric-style-delete"),
            OnlineCacheClear => f.write_str("online-cache-clear"),
            PluginReload => f.write_str("plugin-reload"),
            PluginFolder => f.write_str("plugin-folder"),
            OutDop(on) => f.write_str(if *on { "out-dop:on" } else { "out-dop:off" }),
            CoverChange => f.write_str("cover-change"),
            CoverRestore => f.write_str("cover-restore"),
            Search => f.write_str("search"),
            ListName => f.write_str("list-name"),
            SeekBy(s) => write!(f, "seek-by:{s}"),
            VolumeBy(v) => write!(f, "volume-by:{v}"),
            Layer(l) => write!(f, "layer:{l}"),
            PlayRow(k) => write!(f, "row:{k}"),
            ToTop(k) => write!(f, "to-top:{k}"),
            Remove(k) => write!(f, "remove:{k}"),
            RowArtist(k) => write!(f, "row-artist:{k}"),
            RowAlbum(k) => write!(f, "row-album:{k}"),
            RowPick(k, mv) => write!(f, "row-pick:{k}:{}", u8::from(*mv)),
            AddRow(k, to) | MoveRow(k, to) => {
                let verb = if matches!(self, AddRow(..)) { "add-row" } else { "move-row" };
                match to {
                    Some(id) => write!(f, "{verb}:{k}:{id}"),
                    None => write!(f, "{verb}:{k}:new"),
                }
            }
            Select => f.write_str("select"),
            SelectAll => f.write_str("select-all"),
            SelPick(mv) => write!(f, "sel-pick:{}", u8::from(*mv)),
            AddSel(to) | MoveSel(to) => {
                let verb = if matches!(self, AddSel(_)) { "add-sel" } else { "move-sel" };
                match to {
                    Some(id) => write!(f, "{verb}:{id}"),
                    None => write!(f, "{verb}:new"),
                }
            }
            RemoveSel => f.write_str("remove-sel"),
            RowInfo(k) => write!(f, "row-info:{k}"),
            Lyric(k) => write!(f, "lyric:{k}"),
            CopyPick => f.write_str("copy-pick"),
            Copy(c) => match c {
                CopyWhat::Title => f.write_str("copy:title"),
                CopyWhat::Artist => f.write_str("copy:artist"),
                CopyWhat::Album => f.write_str("copy:album"),
                CopyWhat::TitleArtist => f.write_str("copy:title-artist"),
                CopyWhat::Line(g) => write!(f, "copy:line:{g}"),
                CopyWhat::Lyrics => f.write_str("copy:lyrics"),
            },
            Drop(k) => write!(f, "drop:{k}"),
            Sort(s, d) => write!(f, "sort:{}:{}", s.key(), u8::from(*d)),
            Switch(id) => write!(f, "switch:{id}"),
            Delete(id) => write!(f, "delete:{id}"),
            AddTo(id) => write!(f, "add-to:{id}"),
            ImportToggle(k) => write!(f, "import-toggle:{k}"),
            PluginSwitch(id) => write!(f, "plugin-switch:{id}"),
            PluginFlyout(id) => write!(f, "plugin-flyout:{id}"),
            Connect(id) => write!(f, "connect:{id}"),
            ConnectCancel(id) => write!(f, "connect-cancel:{id}"),
            Disconnect(id) => write!(f, "disconnect:{id}"),
            PluginDelete(id) => write!(f, "plugin-delete:{id}"),
            PickLists(id) => write!(f, "pick-lists:{id}"),
            Plugin { id, verb, arg } if arg.is_empty() => write!(f, "contrib:{id}:{verb}"),
            Plugin { id, verb, arg } => write!(f, "contrib:{id}:{verb} {arg}"),
            Route(AudioOutputRoute::WasapiDefault) => f.write_str("route:default"),
            Route(AudioOutputRoute::Wasapi(id)) => write!(f, "route:wasapi:{id}"),
            Route(AudioOutputRoute::Asio(id)) => write!(f, "route:asio:{id}"),
            OutSys(AudioOutputHost::Asio) => f.write_str("out-sys:asio"),
            OutSys(_) => f.write_str("out-sys:wasapi"),
            SetOption(v) => write!(f, "option:{v}"),
            Settings(Level::Root) => f.write_str("settings"),
            Settings(Level::Plugins) => f.write_str("settings:plugins"),
            Settings(Level::Effects) => f.write_str("settings:effects"),
            Settings(Level::DesktopLyrics) => f.write_str("settings:desktop-lyrics"),
            Effect { verb, at } => write!(f, "effect:{}:{at}", verb.name()),
            EffectsScan => f.write_str("effects-scan"),
            Settings(Level::Effect(at)) => write!(f, "settings:effect:{at}"),
            Param { at, id, value } => write!(f, "param:{at}:{id}:{value}"),
            ParamDrop { at, id } => write!(f, "param-drop:{at}:{id}"),
            Settings(Level::Plugin(id)) => write!(f, "settings:plugin:{id}"),
            Set { key, value } => write!(f, "set:{key}:{value}"),
            Form(k) => write!(f, "form:{k}"),
            DropTyped(k) => write!(f, "drop-typed:{k}"),
        }
    }
}

/// The keys (spec, "Transport under the art": not in a text field, no modifiers) and what each asks.
pub const KEYS: &[(u32, Command)] = {
    use novaui::input::key;
    &[
        (key::SPACE, Command::PlayPause),
        (key::LEFT, Command::SeekBy(-5)),
        (key::RIGHT, Command::SeekBy(5)),
        (key::UP, Command::VolumeBy(5)),
        (key::DOWN, Command::VolumeBy(-5)),
        (key::letter('M'), Command::Mute),
        (key::letter('L'), Command::ToggleLyrics),
        (key::letter('F'), Command::Fullscreen),
        (key::function(11), Command::Fullscreen),
        (key::letter('Q'), Command::TogglePanel),
        // Tab reaches the app only while keyboard navigation is off (the setting).
        (key::TAB, Command::TogglePanel),
    ]
};

/// Carries out `c`. A pending confirmation is forgotten unless `c` is its second press; the
/// flyouts close unless `c` keeps them (`keeps_flyout`).
pub fn run(p: &mut Player, ui: &mut Ui, c: Command) {
    use Command as C;
    let pending = p.pending.take();
    if !c.keeps_flyout() {
        p.views(|v, _| close_menus(v, ui));
    }
    let now = ui.now();
    match c {
        C::PlayPause => p.play_pause(ui),
        C::Prev => p.previous(ui),
        C::Next => p.next(ui),
        C::Mute => p.mute(),
        C::Order => {
            let pq = p.playing_queue_mut();
            let o = pq.order_mode.next(pq.len());
            pq.set_order(o);
            p.schedule_save();
        }
        C::SeekBy(s) => {
            let pos = p.position();
            let d = Duration::from_secs(s.unsigned_abs() as u64);
            let to = if s < 0 { pos.saturating_sub(d) } else { pos + d };
            p.seek(to);
        }
        C::VolumeBy(v) => p.set_volume(p.playback.volume + v as f32 / 100.0),
        C::Fullscreen => ui.window.set_fullscreen(!ui.window.fullscreen()),
        C::DesktopLyrics => {
            use crate::views::desktop_lyrics::SHOWN;
            let on = p.settings.get(SHOWN) == Some("on");
            p.choose_setting(ui, SHOWN, Some(if on { "off" } else { "on" }));
        }
        C::ToggleLyrics => p.side_panel.toggle(now, Page::Lyrics),
        C::ToggleQueue => p.side_panel.toggle(now, Page::Queue),
        C::ToggleInfo => {
            // A queue row's song on the page: the button brings the playing one's back.
            let other = p.side_panel.info_of.take().is_some();
            if other {
                p.fill_playing_info();
            }
            if !(other && p.side_panel.shown.shows_info()) {
                p.side_panel.toggle_info(now);
            }
        }
        C::TogglePanel => {
            let open = p.side_panel.shown.open;
            p.side_panel.set_open(now, !open);
        }
        C::ExpandPanel => p.side_panel.set_open(now, true),
        C::ClearSearch => {
            let s = p.id("search");
            ui.set_edit_text(s, "");
            ui.set_focus(Some(s));
            p.side_panel.set_narrow(None);
        }
        C::Locate => {
            // The song that plays, wherever it plays from: its list is shown first.
            if let Some(id) = p.lists.playing_list.as_ref().and_then(|q| q.id) {
                p.switch_list(ui, id);
            }
            p.side_panel.locate = true;
        }
        C::PageBack => back_panel(p),
        C::Minimize => ui.window.minimize(),
        C::Maximize => ui.window.toggle_maximize(),
        C::Close => ui.window.close(),
        C::AddFiles => {
            let filters = vec![(ui.t("filter.music", &[]), crate::library::open_filter())];
            ui.pick(FilePick::Open { multi: true, filters }, |paths| Answer::Add { paths, lists: false });
        }
        C::AddFolder => ui.pick(FilePick::Folder, |paths| Answer::Add { paths, lists: false }),
        C::Layer(l) => p.toggle_layer(&l),
        C::PlayRow(pos) => p.play_row(ui, pos),
        C::ToTop(pos) => {
            p.lists.shown_list.move_to(pos, 0);
            p.schedule_save();
        }
        C::Remove(pos) => {
            p.lists.shown_list.remove(pos);
            p.schedule_save();
        }
        C::RowPick(pos, mv) => p.views(|v, m| crate::views::menus::row::open_pick(v, ui, m, Some(pos), mv)),
        C::Select => {
            let on = !p.side_panel.selecting();
            p.side_panel.set_selecting(on);
            // Its rows are the queue's: turned on from elsewhere, the queue comes up.
            if on && !p.side_panel.shown.shows(Page::Queue) {
                p.side_panel.toggle(now, Page::Queue);
            }
        }
        C::SelectAll => p.side_panel.select_all(&p.lists.shown_list),
        C::SelPick(mv) => p.views(|v, m| crate::views::menus::row::open_pick(v, ui, m, None, mv)),
        C::AddSel(to) => p.put_selected(ui, to, false),
        C::MoveSel(to) => p.put_selected(ui, to, true),
        C::RemoveSel => {
            if pending == Some(Confirm::RemoveSelected) {
                p.remove_selected(ui);
                p.views(|v, _| close_menus(v, ui));
            } else {
                p.pending = Some(Confirm::RemoveSelected);
                p.views(|v, m| crate::views::menus::row::ask_remove(v, ui, m));
            }
        }
        C::CopyPick => p.views(|v, _| crate::views::menus::copy::open_copy_pick(v, ui)),
        C::Copy(what) => p.copy(ui, what),
        C::AddRow(pos, to) => p.put_row(ui, pos, to, false),
        C::MoveRow(pos, to) => p.put_row(ui, pos, to, true),
        C::RowInfo(pos) => p.row_info(ui, pos),
        C::RowArtist(pos) | C::RowAlbum(pos) => {
            if let Some(s) = p.lists.shown_list.song_at(pos) {
                let n = if matches!(c, C::RowArtist(_)) {
                    Narrow { artist: s.tags.artist.clone(), ..Default::default() }
                } else {
                    Narrow { album: s.tags.album.clone(), ..Default::default() }
                };
                p.side_panel.set_narrow(Some(n));
            }
        }
        C::Lyric(g) => {
            if let Some(at) = p.side_panel.lyric_at(g) {
                p.side_panel.lyric_pressed();
                p.seek(at);
            }
        }
        C::PlaylistMenu => p.views(|v, m| open_playlist_menu(v, ui, m, None)),
        C::SortMenu => {
            let button = p.id("q-sort");
            p.views(|v, m| open_sort_menu(v, ui, m, button));
        }
        // While the settings page is up, the menu button is its back button.
        C::AppMenu if p.settings_page.open => p.settings_page.show(ui, &mut p.doc, false),
        C::AppMenu => {
            // Plugins copied into the folder by hand show once the scan lands.
            p.rescan_plugins(ui);
            p.views(|v, m| open_app_menu(v, ui, m));
        }
        C::OptionMenu => p.open_option(ui),
        C::Drop(key) => match crate::views::menus::output::DROPS.into_iter().find(|k| *k == key) {
            // The outputs are listed again each time; the list shows at once when they were
            // listed before, else when they land.
            Some(k) => {
                p.list_outputs(ui);
                if p.playback.output.listed {
                    p.views(|v, m| crate::views::menus::dropdown::open(v, ui, m, k, ""));
                } else {
                    p.playback.output.asked = Some(crate::playback::output::Asked::Drop(k));
                }
            }
            None => p.views(|v, m| crate::views::menus::dropdown::open(v, ui, m, &key, "")),
        },
        C::Sort(k, desc) => p.sort_by(ui, k, desc),
        C::Switch(id) => p.switch_list(ui, id),
        C::Delete(id) => {
            if pending == Some(Confirm::DeletePlaylist(id)) {
                delete_list(p, ui, id);
            } else {
                p.pending = Some(Confirm::DeletePlaylist(id));
            }
            p.views(|v, m| open_playlist_menu(v, ui, m, None));
        }
        C::New => {
            p.save_now();
            let id = p.lists.store.create(&ui.t("list.new", &[]), Origin::Made);
            p.show_new_list(ui, id);
            p.views(|v, m| open_playlist_menu(v, ui, m, Some(id)));
        }
        C::Rename => {
            let id = p.lists.shown_list.id;
            p.views(|v, m| open_playlist_menu(v, ui, m, id));
        }
        C::ImportLink => ui.read_clipboard(Answer::ImportLink),
        C::Import => {
            let filters = vec![(ui.t("filter.playlists", &[]), "*.m3u;*.m3u8;*.cue".to_owned())];
            ui.pick(FilePick::Open { multi: false, filters }, |paths| Answer::Add { paths, lists: true });
        }
        C::Export => {
            let name = format!("{}.m3u8", p.lists.shown_list.list.name);
            let filters = vec![(ui.t("filter.m3u8", &[]), "*.m3u8".to_owned())];
            let songs: Vec<crate::library::Song> =
                (0..p.lists.shown_list.len()).filter_map(|i| p.lists.shown_list.song_at(i).cloned()).collect();
            ui.pick(FilePick::Save { name, filters }, move |paths| Answer::Export {
                path: paths.into_iter().next(),
                songs,
            });
        }
        C::Rescan => {
            let path = match &p.lists.shown_list.list.origin {
                Origin::Folder(x) | Origin::File(x) => Some(x.clone()),
                Origin::Link(_) => {
                    p.toast(ui, ui.t("toast.syncing", &[]));
                    p.sync_link(ui, true);
                    None
                }
                Origin::Made => None,
            };
            if let Some(path) = path {
                p.rescan(ui, path);
            }
        }
        C::ImportCancel => p.views(|v, _| close_import_dialog(v, ui)),
        C::ImportAll => {
            // A tri-state 全选: any unpicked picks all; all picked clears.
            if let Some(ImportPick { items, .. }) = &mut p.import.pick {
                let all = items.iter().filter(|i| !i.done).all(|i| i.on);
                for i in items.iter_mut().filter(|i| !i.done) {
                    i.on = !all;
                }
            }
            p.views(|v, _| fill_import_list(v, ui));
        }
        C::ImportOk => {
            let urls: Vec<String> = p
                .import
                .pick
                .as_ref()
                .map(|x| x.items.iter().filter(|i| i.on && !i.done).map(|i| i.url.clone()).collect())
                .unwrap_or_default();
            p.views(|v, _| close_import_dialog(v, ui));
            // Each url imports as its own playlist, exactly as 从链接导入 (a known one is not
            // imported twice).
            for u in urls {
                if !p.link_known(&u) {
                    p.import_link(ui, u);
                }
            }
        }
        C::ImportToggle(k) => {
            if let Some(ImportPick { items, .. }) = &mut p.import.pick {
                if let Some(it) = items.get_mut(k).filter(|i| !i.done) {
                    it.on = !it.on;
                }
            }
            p.views(|v, _| fill_import_list(v, ui));
        }
        C::PluginSwitch(id) => {
            let on = p.plugin.registry.get(&id).is_some_and(|x| x.is_on());
            if on {
                p.plugin.registry.disable(&id);
                p.rescan_plugins(ui);
            } else if pending == Some(Confirm::EnablePlugin(id.clone())) {
                // Second press: the capabilities were on screen; enable, as its files are (a
                // task hashes them when they are not as last hashed).
                if let Some((dir, prints)) = p.plugin.registry.enabling(&id) {
                    let task = ui
                        .spawn_waiting(move |_| prints.now(&dir), move |print| Landed(Job::PluginHashed { id, print }));
                    p.tasks.hold(Some(task));
                }
            } else {
                p.pending = Some(Confirm::EnablePlugin(id));
            }
            refresh_plugins(p);
        }
        C::PluginFlyout(id) => {
            // A row of the flyout opens its plugin's; the same row again closes it.
            p.menus.plugin = if p.menus.plugin.as_deref() == Some(id.as_str()) { None } else { Some(id) };
            p.views(|v, m| open_app_menu(v, ui, m));
        }
        C::Connect(id) => {
            let fields = crate::views::menus::plugin_flyout::connect_fields(p, &id);
            p.plugin.accounts_moved();
            crate::plugins::connect::start(p, ui, &id, fields);
            refresh_plugins(p);
        }
        C::ConnectCancel(id) => {
            p.plugin.drop_connection(ui, &id);
            refresh_plugins(p);
        }
        C::Disconnect(id) => {
            p.plugin.sign_out(ui, &id);
            refresh_plugins(p);
        }
        C::PluginDelete(id) => {
            let name = p.plugin.registry.name_of(&id);
            p.plugin.sign_out(ui, &id);
            match p.plugin.registry.removal(&id) {
                Ok(dir) => {
                    let task = ui.spawn_waiting(
                        move |_| crate::plugins::registry::remove_files(&id, &dir).map(|()| id),
                        move |result| Landed(Job::PluginRemoved { name, result }),
                    );
                    p.tasks.hold(Some(task));
                }
                Err(e) => p.toast_error(ui, e),
            }
            p.menus.plugin = None;
            refresh_plugins(p);
        }
        C::PluginInstall => {
            // The package as distributed: no unpacking first.
            let filters = vec![(ui.t("filter.plugins", &[]), crate::plugins::registry::archive::PICK.to_owned())];
            ui.pick(FilePick::Open { multi: false, filters }, |paths| Answer::PluginPackage(paths.into_iter().next()));
        }
        C::LyricStyleInstall => {
            let filters = vec![(
                ui.t("filter.lyric-styles", &[]),
                format!("{};style.json", crate::plugins::registry::archive::PICK),
            )];
            ui.pick(FilePick::Open { multi: false, filters }, |paths| Answer::LyricStyle(paths.into_iter().next()));
        }
        C::LyricStyleDelete => {
            let Some(id) = p.settings.get("lyric-style").filter(|id| !id.is_empty()).map(str::to_owned) else { return };
            let Some(root) = crate::app::lyric_styles::root() else {
                return p.toast_error(ui, ui.t("err.no-data-dir", &[]));
            };
            let task = ui.spawn_waiting(
                move |_| crate::app::lyric_styles::remove(&root, &id).map(|()| id),
                |result| Landed(Job::LyricStyleRemoved(result)),
            );
            p.tasks.hold(Some(task));
            p.choose_setting(ui, "lyric-style", None);
        }
        C::PickLists(id) => {
            p.toast(ui, ui.t("toast.reading-lists", &[]));
            p.pick_lists(ui, &id);
        }
        C::OnlineCacheClear => {
            let task = ui.spawn_waiting(|_| crate::plugins::client::clear_cache(), |()| ());
            p.plugin.errands.hold(Some(task));
            p.toast(ui, ui.t("toast.cache-cleared", &[]));
        }
        C::PluginReload => {
            let reload = p.plugin.registry.reloader();
            let task =
                ui.spawn_waiting(move |_| reload(), |(enabled, list)| Landed(Job::PluginsReloaded(enabled, list)));
            p.tasks.hold(Some(task));
        }
        C::PluginFolder => {
            if let Some(root) = p.plugin.registry.root.clone() {
                let _ = std::fs::create_dir_all(&root);
                novaui::platform::open(root);
            }
        }
        // A plugin's control (controls.rs): only the commands it may name.
        C::Plugin { id, verb, .. } => match verb.as_str() {
            "option.open" => p.open_option(ui),
            v if v.starts_with("plugin.") => p.plugin_command(ui, &id, &v["plugin.".len()..]),
            _ => {}
        },
        // The flyout stays up and shows the choice made; a settings dropdown's list closes.
        C::Route(route) => {
            p.choose_output(ui, |o| o.choose_route(route));
            output_chosen(p, ui);
        }
        C::OutSys(h) => {
            p.choose_output(ui, |o| o.choose_system(h));
            output_chosen(p, ui);
        }
        C::OutDop(on) => {
            p.choose_output(ui, |o| o.set_dop(on));
            output_chosen(p, ui);
        }
        C::SetOption(v) => p.set_option(ui, &v),
        C::CoverChange => p.choose_cover(ui),
        C::CoverRestore => p.restore_cover(ui),
        C::Settings(level) => {
            if level == Level::Effects {
                p.scan_effects(ui);
                p.playback.effects.prepare_windows(ui.window.handle());
            }
            let effect = matches!(level, Level::Effect(_));
            if effect {
                p.playback.effects.params = None;
            }
            if p.settings_page.open {
                p.views(|v, md| crate::views::settings_page::go(v, ui, md, level))
            } else {
                // The outputs are listed as the page opens: the interface's card (shown when
                // there is a choice of interfaces) is up with the page, not pushed in under the
                // pointer once the device list is first opened.
                if !p.playback.output.listed {
                    p.list_outputs(ui);
                }
                p.views(|v, md| crate::views::settings_page::open(v, ui, md, level))
            }
            if effect {
                p.read_effect_params();
            }
        }
        C::Effect { verb, at } => {
            let (inbox, window) = (p.inbox.clone(), verb == crate::playback::effects::EffectVerb::Window);
            let report = move |outcome: Result<(), String>| {
                if let Err(why) = outcome {
                    inbox.send(crate::app::jobs::Job::EffectFailed { window, why });
                }
            };
            if let Err(why) = p.playback.effects.apply(verb, at, ui.window.handle(), report) {
                if !why.is_empty() {
                    p.toast(ui, ui.t("effects.failed", &[("why", &why)]));
                }
            }
            p.views(|v, md| crate::views::settings_page::refresh(v, ui, md));
        }
        C::Param { at, id, value } => {
            p.playback.effects.set_param(at, id, value);
            p.views(|v, md| crate::views::settings_page::params_read(v, ui, md));
        }
        C::ParamDrop { at, id } => p.views(|v, md| crate::views::menus::dropdown::open_param(v, ui, md, at, id)),
        C::EffectsScan => {
            p.playback.effects.found = None;
            p.scan_effects(ui);
            p.views(|v, md| crate::views::settings_page::refresh(v, ui, md));
        }
        C::Set { key, value } => {
            p.choose_setting(ui, &key, Some(&value));
            if key == "replay-gain" {
                p.playback.engine.send(crate::playback::engine::Cmd::ReplayGain(
                    crate::views::settings_page::replay_gain_of(Some(&value)),
                ));
            }
        }
        // Fields name these for their events (`views::input`); a press does not.
        C::AddTo(_) | C::Search | C::Form(_) | C::DropTyped(_) | C::ListName => {}
    }
}

/// The second press of a playlist's delete: it goes; when it was shown, the first other one
/// shows (or a new empty one, when it was the last).
/// An output was chosen: the output flyout, up, shows it; a settings dropdown's list closes (its
/// box names the choice at the next present).
fn output_chosen(p: &mut Player, ui: &mut Ui) {
    if p.views(|v, _| crate::views::menus::output::is_up(v)) {
        p.views(|v, m| open_output_menu(v, ui, m));
    } else {
        p.views(|v, _| close_menus(v, ui));
    }
}

fn delete_list(p: &mut Player, ui: &mut Ui, id: u32) {
    let was_open = p.lists.shown_list.id == Some(id);
    if let Some(q) = p.lists.playing_list.as_mut().filter(|q| q.id == Some(id)) {
        q.id = None;
    }
    p.lists.store.delete(id);
    if !was_open {
        return;
    }
    match p.lists.store.entries.first().map(|e| e.id) {
        Some(n) => {
            p.lists.shown_list.id = None;
            p.switch_list(ui, n);
        }
        None => {
            p.stop(ui);
            p.lists.show_default(ui);
            p.shown_list_moved();
        }
    }
}

/// A plugin changed (its state, its connection): what shows the plugins shows it again, at
/// this turn's presentation (`ViewAsk::RefreshPlugins`).
fn refresh_plugins(p: &mut Player) {
    p.view_asks.push(crate::app::ViewAsk::RefreshPlugins);
}

/// Finishes what a command asked the system for (`Answer`), once it has answered.
pub(crate) fn answered(p: &mut Player, ui: &mut Ui, a: Answer) {
    match a {
        Answer::Add { paths, lists } => {
            if !paths.is_empty() {
                p.add_paths(ui, paths, lists);
            }
        }
        Answer::Export { path: Some(path), songs } => {
            let task = ui.spawn_waiting(
                move |_| {
                    let songs: Vec<&crate::library::Song> = songs.iter().collect();
                    crate::library::write_m3u8(&path, &songs).map(|()| songs.len()).map_err(|e| e.to_string())
                },
                |result| Landed(Job::Exported(result)),
            );
            p.tasks.hold(Some(task));
        }
        Answer::PluginPackage(Some(file)) => {
            let Some(root) = p.plugin.registry.root.clone() else {
                return p.toast_install_failed(ui, &ui.t("err.no-data-dir", &[]));
            };
            let task = ui.spawn_waiting(
                move |_| crate::plugins::registry::install_package(&root, &file),
                |result| Landed(Job::PluginInstalled(result)),
            );
            p.tasks.hold(Some(task));
        }
        Answer::LyricStyle(Some(file)) => {
            // A folder is picked by its style.json.
            let from = if file.file_name().is_some_and(|n| n == "style.json") {
                file.parent().map(std::path::Path::to_path_buf).unwrap_or(file)
            } else {
                file
            };
            let Some(root) = crate::app::lyric_styles::root() else {
                return p.toast_install_failed(ui, &ui.t("err.no-data-dir", &[]));
            };
            let locale = ui.strings().locale();
            let task = ui.spawn_waiting(
                move |_| crate::app::lyric_styles::install(&root, &from, &locale),
                |result| Landed(Job::LyricStyleInstalled(result)),
            );
            p.tasks.hold(Some(task));
        }
        Answer::Cover { song, image: Some(image) } => p.set_chosen_cover(ui, *song, image),
        Answer::ImportLink(Some(text)) => p.import_link(ui, text.trim().to_owned()),
        Answer::MenuLink(text) => p.views(|v, m| crate::views::menus::playlist::show_link(v, ui, m, text.as_deref())),
        // Cancelled, or nothing to read.
        Answer::Export { path: None, .. }
        | Answer::PluginPackage(None)
        | Answer::LyricStyle(None)
        | Answer::Cover { image: None, .. }
        | Answer::ImportLink(None) => {}
    }
}

/// A press on `id` with its action: the command it names (a row verb takes the song of the row
/// it is in, a lyric its group).
pub fn of_press(p: &Player, ui: &Ui, id: Id, a: &Action) -> Option<Command> {
    Command::of(a, || match a.name.as_str() {
        "lyric" => Some(ui.tree.el(id).tag() as usize),
        _ => row_place(p, ui, id).and_then(|place| p.side_panel.pos_of(place)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command's text reads back as the same command: one of each variant (the match
    /// below does not compile while one is left out).
    #[test]
    fn a_command_reads_back_as_written() {
        use crate::playback::effects::EffectVerb;
        use Command::*;
        let s = || "x".to_owned();
        let all = vec![
            PlayPause,
            Prev,
            Next,
            Mute,
            Order,
            SeekBy(-5),
            VolumeBy(5),
            Fullscreen,
            DesktopLyrics,
            ToggleLyrics,
            ToggleQueue,
            ToggleInfo,
            TogglePanel,
            ExpandPanel,
            ClearSearch,
            Locate,
            PageBack,
            Minimize,
            Maximize,
            Close,
            AddFiles,
            AddFolder,
            Layer("trans".into()),
            PlayRow(3),
            ToTop(3),
            Remove(3),
            RowArtist(3),
            RowAlbum(3),
            RowPick(3, true),
            AddRow(3, Some(2)),
            MoveRow(3, None),
            Select,
            SelectAll,
            SelPick(true),
            AddSel(Some(2)),
            MoveSel(None),
            RemoveSel,
            RowInfo(3),
            Lyric(4),
            CopyPick,
            Copy(CopyWhat::Title),
            Copy(CopyWhat::Artist),
            Copy(CopyWhat::Album),
            Copy(CopyWhat::TitleArtist),
            Copy(CopyWhat::Line(3)),
            Copy(CopyWhat::Lyrics),
            PlaylistMenu,
            SortMenu,
            AppMenu,
            OptionMenu,
            Drop("locale".into()),
            Sort(crate::library::queue::Sort::Album, true),
            Switch(2),
            Delete(2),
            AddTo(2),
            New,
            Rename,
            ImportLink,
            Import,
            Export,
            Rescan,
            ImportCancel,
            ImportAll,
            ImportOk,
            ImportToggle(1),
            PluginSwitch(s()),
            PluginFlyout(s()),
            Connect(s()),
            ConnectCancel(s()),
            Disconnect(s()),
            PluginDelete(s()),
            PluginInstall,
            LyricStyleInstall,
            LyricStyleDelete,
            PickLists(s()),
            OnlineCacheClear,
            PluginReload,
            PluginFolder,
            Plugin { id: s(), verb: "option.open".into(), arg: String::new() },
            Plugin { id: s(), verb: "plugin.like".into(), arg: "1".into() },
            Route(AudioOutputRoute::WasapiDefault),
            Route(AudioOutputRoute::Wasapi("d".into())),
            Route(AudioOutputRoute::Asio("a".into())),
            OutSys(AudioOutputHost::Asio),
            OutSys(AudioOutputHost::Wasapi),
            OutDop(true),
            OutDop(false),
            SetOption("hq".into()),
            CoverChange,
            CoverRestore,
            Settings(Level::Root),
            Settings(Level::Plugins),
            Settings(Level::Effects),
            Settings(Level::DesktopLyrics),
            Settings(Level::Plugin(s())),
            Settings(Level::Effect(1)),
            Effect { verb: EffectVerb::Bypass, at: 1 },
            EffectsScan,
            Param { at: 1, id: 7, value: 0.25 },
            ParamDrop { at: 1, id: 7 },
            Set { key: "locale".into(), value: "en".into() },
            Search,
            Form(2),
            DropTyped("font".into()),
            ListName,
        ];
        for c in &all {
            match c {
                PlayPause
                | Prev
                | Next
                | Mute
                | Order
                | SeekBy(_)
                | VolumeBy(_)
                | Fullscreen
                | DesktopLyrics
                | ToggleLyrics
                | ToggleQueue
                | ToggleInfo
                | TogglePanel
                | ExpandPanel
                | ClearSearch
                | Locate
                | PageBack
                | Minimize
                | Maximize
                | Close
                | AddFiles
                | AddFolder
                | Layer(_)
                | PlayRow(_)
                | ToTop(_)
                | Remove(_)
                | RowArtist(_)
                | RowAlbum(_)
                | RowPick(..)
                | AddRow(..)
                | MoveRow(..)
                | Select
                | SelectAll
                | SelPick(_)
                | AddSel(_)
                | MoveSel(_)
                | RemoveSel
                | RowInfo(_)
                | Lyric(_)
                | CopyPick
                | Copy(_)
                | PlaylistMenu
                | SortMenu
                | AppMenu
                | OptionMenu
                | Drop(_)
                | Sort(..)
                | Switch(_)
                | Delete(_)
                | AddTo(_)
                | New
                | Rename
                | ImportLink
                | Import
                | Export
                | Rescan
                | ImportCancel
                | ImportAll
                | ImportOk
                | ImportToggle(_)
                | PluginSwitch(_)
                | PluginFlyout(_)
                | Connect(_)
                | ConnectCancel(_)
                | Disconnect(_)
                | PluginDelete(_)
                | PluginInstall
                | LyricStyleInstall
                | LyricStyleDelete
                | PickLists(_)
                | OnlineCacheClear
                | PluginReload
                | PluginFolder
                | Plugin { .. }
                | Route(_)
                | OutSys(_)
                | OutDop(_)
                | SetOption(_)
                | CoverChange
                | CoverRestore
                | Settings(_)
                | Effect { .. }
                | EffectsScan
                | Param { .. }
                | ParamDrop { .. }
                | Set { .. }
                | Search
                | Form(_)
                | DropTyped(_)
                | ListName => {}
            }
            let text = c.to_string();
            assert_eq!(text.parse::<Command>().as_ref(), Ok(c), "{text}");
        }
    }
}
