# NovaPlayer map

What exists and where. It is facts only. A change that
adds, moves or removes a module or a mechanism updates this file in the same commit.

NovaPlayer stands on two repositories with maps of their own: NovaUI's
[`docs/map.md`](https://github.com/wuuuming/NovaUI/blob/main/docs/map.md) (the window, the turn,
elements, `.nui`, timers, tasks, images, text) and Player-Core's
[`docs/map.md`](https://github.com/wuuuming/Player-Core/blob/main/docs/map.md) (the sound). What
the product does is in [`novaplayer-spec.md`](novaplayer-spec.md); the three-way split of what
belongs where is in [`extensions.md`](extensions.md); the plugin protocol is in
[`plugins.md`](plugins.md). Paths are under `crates/novaplayer/src/` unless named otherwise.

## 1. Layers

```
novaui::platform::run ──▶ Player (app/mod.rs, implements novaui::App)
  event(ui, Event) ─▶ views::input::handle ─▶ app::command (Command::of ▸ run)
                    ─▶ every message but Present notifies `presentation`
  Msg::Present (the flush) ─▶ Player::turn: jobs drained, engine snapshot read, session
                               decisions, views present, deadlines kept
  after_layout ─▶ placements against laid-out boxes; NovaUI then settles every Clock
   ├─ app/        Player, Msg, Command, jobs, settings, lyric styles
   ├─ views/      writers of the tree: layout inputs, transport, now playing, side panel
   │              (queue, lyrics, info), menus (flyouts), settings page, toast, import dialog
   ├─ library/    songs, lists (Queue), playlists store, row cache, covers, lyrics, CUE, M3U
   ├─ playback/   Engine (player-core's AudioPlayer on its own thread), session (opening
   │              songs, the hand-over), output choice, kept volume, CLAP effects
   └─ plugins/    registry (installed, enabled), host (JSON-lines child processes), client
                  (calls by purpose), accounts (the vault), connect, controls, pcm, prefetch
ui/player.nui    every layout rule and placement; the app sets state bindings only
ui/*.txt         string tables (zh-CN, en the fallback)
crates/plugin-protocol   JSON value, parser (depth ≤ 64), line limits, API version
crates/plugins/          kit (pipe loop, HTTP, M3U8, util) and the service plugins (ncm, qqmusic)
tools/                   ci.py (fmt, build, clippy, unit tests), bump.py (pins), portable.py
                         (the Windows or Linux zip), readings.py (search readings from Unihan)
.github/workflows/       ci.yml (main, PRs), release.yml (after CI on a new version: the release; by hand: artifacts)
```

Threads: the UI thread (everything under `Player`, every view, the tree); `novaplayer-engine`
(the player-core `AudioPlayer`, commands in, a snapshot and a wake out); `novaplayer-store` (playlist
writes and reads) and `novaplayer-store-watch` (polls the playlists folder); NovaUI's workers (the
`assets` serial queue: row tags; a local song's face, as a task); NovaUI's `novaui-wait`
threads, one per task that waits (`Ui::spawn_waiting`: a plugin call, a link fetch, a rescan, a
batch of tags with `read_tags`' scoped pool of 2–8 threads, a batch of thumbnails, a cover
choice, a sign-in's call, the effects scan); the engine's own threads for the next song's resolve
and a stream probe; per running plugin a reader (`plugin-<id>`) and a stderr logger (`plugin-<id>-err`);
`plugin-fingerprint` per plugin folder hashed; one play-event reporter; the effects scan, which
runs `novaplayer --scan-clap <file>` as a child per CLAP file; and player-core's own threads
(its map §1).

## 2. Flows

**Start.** `main` (a `--scan-clap` child exits at once) → crash hook → `platform::run` →
`Player::new`: the sheet (`NOVAPLAYER_UI_FILE` or the built-in `player.nui`; a debug build watches
that file, or the checkout's `ui/player.nui`, and applies its edits live: NovaUI `Doc::watch`), string tables,
`settings.txt`, the locale, the engine (with the kept volume, ReplayGain mode and the effects
chain loaded from `effects.txt`), the output choice (`output.txt`), the playlist store, the plugin
registry (scan, host registrations, face-cache stamps). `init`: the tree built (`init_view`),
the kept settings applied (`SETTINGS`), the active list read on the store's thread, then shown and its song opened paused
where it was, plugin controls mounted.

**A press.** novaui delivers `Event::Click { action }`; `views::input` → `command::of_press`
(a row verb takes its row's place) → `Command` → `command::run`. Flyouts close unless the command
keeps them; a confirm (`Confirm`) waits for its second press. Keys: novaui's field first, then
`input::key` (dropdown keys, page keys, `KEYS`, Esc's chain).

**A turn's present.** Any message notifies `presentation`; its subscription delivers
`Msg::Present` once in the flush → `Player::turn`: `drain_jobs` (the inbox), the engine's snapshot (a followed song, a seek not yet carried out, an error, the
end), `hand_next`, the stand-in, `present` (now playing, side panel, menus, layout inputs,
transport, toast). `after_layout`: the queue's rows against the list's height, the lyric scroll,
flyout placement, the settings page; NovaUI then settles every owner's `Clock` (its asks of the
turn become its ticker and timer).

**A song opens.** `session::open_current` → new version, `Snapshot { opening }`, `show_song`
(the face, a task of the song's: a local file probed, its cover from the file, the chosen cover or the folder; an
online song's words and lyrics from its plugin's `metadata`, the picture after) and
`Cmd::Open` → the engine: `source_for` (a decode plugin's PCM, a plugin's `resolve`, an address,
a file) → `AudioPlayer::open_with` → seek → play. The engine serves the player
(`service`, waiting until `next_wake_at`) and publishes the snapshot and a wake when what shows
changed; the position is not pushed (the UI extrapolates it).

**The next song.** `hand_next` gives the engine the list's next (`Cmd::Next`, a version of its
own, the crossfade setting unless the two share an album). Thirty seconds before the end the
engine resolves it off its thread, then `set_next`. When the core says it followed, the snapshot
takes the handed version; `turn` moves the list there (`show_followed`).

**A stream breaks.** The engine's `Recover`: loading, reopened where it was after 0, 1, 2, 3, 5,
8, 10 s; then the error. A lost device holds the song paused (`hold`); play opens it again.

**A list.** Added paths are scanned on a task (`songs_for`: folders, M3U, CUE, files; a new add
calls off the one before) → `Job::Added`; the list's unread tags are read by `fill_tags`, a task
per 250 ms batch, each landing as `Job::Tags` and asking the next; a save is asked two seconds after the last change (`schedule_save`) and written by
the store's thread; edits outside the process come back through the store's watcher
(`Job::StoreChanged`).

**A row comes into view.** The queue's rows are NovaUI's `VirtualList` (rows by place, parked and
reused, a carried row kept live, the rows it passes giving way), synced in `SidePanel::sync`; a row
without tags or thumbnail asks the `assets` queue → `row_cache::load` (the disk cache, else the
file or the plugin's face) → `Job::RowTags`; a row out of view before its turn is skipped
(`wanted`).

**A plugin call.** `plugins::client` names the call by purpose (`resolve`, `metadata`,
`playlist`, `options`, `lists`, `thumbs`, `command`, `play_event`) with the account's credential
→ `plugins::host::Host::call`: one resident child per plugin, JSON lines, answers matched by id,
one call at a time unless the plugin is `concurrent`, a timeout per call, a hung plugin killed,
three restarts a minute.

**Sign-in.** `connect::start`: `connect.begin` as a task of the `Connection` (what to show: a QR
code, a page, a code → `Job::ConnectBegun`), then `connect.poll` as a task each time the
connection's clock says (`Msg::Connect`, at the plugin's pace) → `Job::ConnectPolled`; the
credential into the vault (`accounts`) on the poll that says done. A connection dropped (another,
a cancel, a sign-out) tells the plugin (`connect.cancel`) as an errand.

**Plugin controls.** The registry reads each manifest's controls (`controls::read_controls`:
the fragment under `PROFILE`, its commands, its `when`); `sync_plugin_controls` mounts those
whose condition holds into their slot (`views::plugin_controls`) and pushes the context keys
they read. A press reaches `Command::Plugin` (`option.open`, `plugin.<name>`).

**Kept state.** The playlists, `settings.txt`, `options.txt`, `output.txt`, `lyrics-layers.txt`,
`volume.txt`, `effects.txt` and each effect's state are each a `Kept` (`app/kept.rs`): marked
changed where they change, written by the store's thread a fixed time after the first change
(a choice at the turn's end, the volume 1 s, the effects and the lists 2 s), the text made from
the state when due. On close and on a panic `Player::rescue` writes every one pending, the
volume, the effects and the lists, and waits for the store. Chosen covers, the row cache, the
online face cache, fingerprints, `enabled.txt` and `clap-scan.txt` are written by the work that
makes them.

## 3. Modules

| Module | Owns |
|---|---|
| `lib.rs` | the app as a library: `run` (the `--scan-clap` child, the crash hook, `--backend`, files to add, `platform::run`); `data_root`, `data_dir`; with `test-support`, the modules the scenarios drive are public and `source_root` names this repository |
| `main.rs` | the program: `run` (and the `count-allocs` allocator) |
| `crash.rs` | the panic hook: `crash.log`, the save `Drop` would have made (on the UI thread, or asked of it) |
| `files.rs` | `write_atomic`, `read_kept` (an undecodable file set aside) |
| `sync.rs` | `Locked` (a poisoned lock's data as it is), `wait` |
| `app/mod.rs` | `Player` (all state), `Views`/`Models` split, `ViewAsk`, `App` impl, `turn`, `rescue` |
| `app/msg.rs` | `Msg`: one message per cause and owner; `Answer`: the system's answer to a picker or clipboard read, finished by `command::answered` |
| `app/command.rs` | `Command` (text form both ways), `Confirm`, `KEYS`, `run` |
| `app/jobs.rs` | `Job`, `Tasks` (a scope's), `Landed`, `JobSender` (the inbox), `land`, `drain_jobs` |
| `app/kept.rs` | `Kept`, `Keeping` (what is changed and when it is due), `Player::keep` |
| `app/settings.rs` | `SettingsFile` (`settings.txt`), `Player::set_setting`, the lyrics layers' file |
| `app/lyric_styles.rs` | lyric style packages: install, list, find, remove |
| `app/query/` | `Query`: every typed word found in one of the names, as text, by reading or with a typo (`Found` ranks); `Signature` (a thing's names as bit masks: most things are passed over unfolded), `Query::narrows` (typed on: only what was found before can be found); `fold` (folding, pieces), `kana`, `hangul`, `han` (Unihan readings, `readings.z` by `tools/readings.py`), `typo` (bit-parallel edit distance) |
| `library/mod.rs` | `Song`, `Tags`, lyrics (`Lyrics`, `Group`, layers), CUE, M3U, folder scan, `read_tags`, the colour field, cover decode, format texts |
| `library/queue.rs` | `Queue` (a list, its position, play order, shuffle round, sort, filter), `Order`, `Sort` |
| `library/store.rs` | playlists on disk: index and lists, `Origin`, `Entry`, `List`, the writer thread, the watcher |
| `library/lists.rs` | `Lists` (shown and playing list), add, import from links, sync, rescan, sort, tags, save, switch |
| `library/disk_cache.rs` | `DiskCache`: files kept by a source's key under a budget, grouped (a folder, a plugin), shared blobs by content; the index in memory from one listing |
| `library/row_cache.rs` | row words and thumbnails on the disk cache |
| `library/covers.rs` | folder artwork, chosen covers |
| `playback/engine.rs` | `Engine`, `Cmd`, `Snapshot`, `NextSong`, `Recover`; probes |
| `playback/session.rs` | opening songs, the hand-over, the face's arrival and stand-in, transport actions |
| `playback/output.rs` | `OutputChoice` (`output.txt`), device rows |
| `playback/gain.rs` | `Gain`, `KeptGain` (`volume.txt`) |
| `playback/effects.rs` | `EffectsModel` (the CLAP chain kept), the scan and its cache |
| `plugins/registry.rs` | manifests, `State`, enable with fingerprints, install (zip/tar reader), remove, reload |
| `plugins/host.rs` | `Host`: child processes, calls, timeouts, restarts |
| `plugins/client.rs` | calls by purpose, the plugin option, the face cache (`online-v3`), a cover's fetch (`fetch_url`, ureq: SChannel on Windows, rustls elsewhere), play events |
| `plugins/pcm.rs` | the global `Host`, decoders and schemes by plugin, `PcmSource` into the core (`factory`) |
| `plugins/accounts.rs` | the vault (Credential Manager, Secret Service) |
| `plugins/connect.rs` | sign-in sessions |
| `plugins/controls.rs` | control manifests, `PROFILE`, `SLOTS`, `Context` |
| `plugins/prints.rs` | plugin folder fingerprints |
| `plugins/prefetch.rs` | online thumbnails for the shown list |
| `views/present.rs` | `init_view`, `present`, `relabel`, `after_layout_phase`, the search box |
| `views/layout.rs` | the layout's state bindings, the title's line count, `Scale` |
| `views/input.rs` | events to commands, drags, wheels, keys |
| `views/transport.rs` | the control bar, the reveal over the artwork, the volume tip |
| `views/now_playing.rs` | artwork slots and crossfade, the words' two blocks, the backdrop effect (its GPU program `views/field.wgsl`, compiled by build.rs) |
| `views/side_panel/` | `SidePanel`: open/close and page turns (`mod.rs`), the queue's rows, scroll, drag, rails (`queue.rs`), lyric groups, styles, layers (`lyrics.rs`), the info page (`info.rs`) |
| `views/menus/` | `Menus` on two novaui `Flyout`s: playlist, app, plugin, row, sort, cover, option, output, dropdown lists, the drag target |
| `views/settings_page.rs` | the settings page: levels, cards, sliders, switches, `SETTINGS` |
| `views/plugin_view.rs`, `plugin_controls.rs` | one plugin's description for both containers; mounted controls |
| `views/import_dialog.rs`, `toast.rs` | the import dialog; the toast |
| `views/desktop_lyrics.rs`, `ui/desktop-lyrics.nui` | desktop lyrics: what the player feeds them each present, and the overlay (its own thread) drawing it |

## 4. Mechanisms and the semantics they carry

A new semantic first looks here, and in NovaUI's and Player-Core's maps, for the mechanism that
can carry it (principles D2).

| Mechanism | Where | Scope · thread | Semantics it carries | The part a semantic hands it |
|---|---|---|---|---|
| Message per cause | `app/msg.rs`, `Player::event` | the app · UI | every waker, timer, ticker and subscription names its owner; one present per turn | a `Msg` variant |
| Questions to the system | `Ui::pick`, `Ui::read_clipboard` → `Answer` → `command::answered` | a request · UI | file pickers and clipboard reads, answered in a later turn, never waited for | an `Answer` variant |
| Presentation | `Player::presentation` (novaui `Model`) | the app · UI | the views present once per turn, after everything delivered | — |
| Clock | NovaUI `Clock` (`Ui::clock`), one per owner, made with its `Msg` | a view or model · UI | frames while something moves, deadlines (exact or a window), cancel; settled by NovaUI at the end of the turn | `frame`, `at`, `within` |
| Command | `app/command.rs` | a press · UI | every press, key, field and plugin control's request; its text form for sheets and rows; confirms | a variant and its text |
| Views / Models split | `Player::split` | a turn · UI | a view writes the tree and its own state, reads the models | a view function |
| View asks | `ViewAsk`, `present_pending` | a turn · UI | model and job code asking a flyout to open or rebuild | a variant |
| Tasks | `app/jobs.rs` `Tasks` on NovaUI `Task` (`Ui::spawn`, `Ui::spawn_waiting`), landing as `Landed(Job)` → `Player::land` | the scope that holds them: the open song (`Playback::tasks`), the shown list (`Lists::tasks`, `Lists::adding`), the accounts (`Plugins::account_tasks`), a connection, the app (`Player::tasks`, `Plugins::errands`) · worker or `novaui-wait` → UI | off-thread work and its result; a scope that moves on (`show_song`, `Lists::moved`, `Plugins::accounts_moved`) drops its tasks, so what lands is about what is shown | a closure → `Job` |
| Inbox | `JobSender`, `Msg::Jobs`, `drain_jobs` | the app · any thread → UI | what another thread reports on its own: a row read on the assets queue (with its list's version), the effects' host thread, the playlists folder's watcher | a `Job` |
| Assets queue | `Player::assets` (novaui `Queue`) | the app · novaui worker | row tags and thumbnails, one at a time, bounded | a closure |
| Engine | `playback/engine.rs` | the app · engine thread | open, play, pause, seek, volume, ReplayGain, next song, recovery, device loss | a `Cmd` |
| Snapshot | `Engine::state`, `Snapshot` | a song · engine → UI | what the sound does now, by version; counters for held and no-device | — |
| Song version | `Playback::new_version`, `handed` | a song · UI | which open or handed song a snapshot, job or flyout is about | a number |
| Kept file | `files::write_atomic`, `read_kept` | a file · caller | a file whole or not at all; a damaged one set aside | bytes |
| Kept state | `app/kept.rs` `Keeping`, `Player::keep`, `Msg::Keep`; `Store::keep`, `Store::run` | the app · UI → store thread | the user's state on disk: written a fixed time after its first change, in order with the lists, all at once by `rescue` | a `Kept` |
| Store | `library/store.rs` | the playlists · its thread | lists and index on disk, written and read on its thread in the order asked (a read lands as `Job::ListRead` with what it was for, `ListFor`), outside edits read back | a `List` |
| Disk cache | `library/disk_cache.rs`, used by `library/row_cache.rs` and `plugins/client.rs` `cache` | the user's data · workers | row words and thumbnails (a song's folder's group, a thumbnail shared by content); online faces and thumbnails (a plugin's group); least recently used evicted to nine tenths of the budget, a group forgotten at once, from the index alone | a group, a key, a file |
| Queue (list) | `library/queue.rs` | a list · UI | play order, shuffle rounds, sort, filter, current song | songs |
| Plugin host | `plugins/host.rs`, `pcm::host` | the process · callers' threads | every plugin call, its timeout, the plugin's life | a call and params |
| Registry | `plugins/registry.rs`; `Player::plugins_changed` | the process · UI | what is installed and enabled, what each provides, trust by fingerprint; a scan that finds a change syncs the host and the face cache's stamps once, and the app's one owner remounts the controls, reads the accounts and refreshes the views | a manifest |
| Vault | `plugins/accounts.rs` | the user · caller | a plugin's credential and account name | a credential |
| Fragment mount | `views/plugin_controls.rs`, `side_panel/lyrics.rs` (novaui `Doc::adopt`/`mount`) | a slot · UI | plugin controls, lyric styles | a fragment and a profile |
| Desktop lyrics | `views/desktop_lyrics.rs` `DesktopLyrics` (player), `Overlay` (novaui `spawn_overlay`) | the setting · player UI → overlay thread | the overlay opened and closed as the setting says; a `Feed` (lyrics with their layers, the layer shown, the song's clock, the look) handed over when it changed, by a lock and the overlay's waker; its presses back as `Command` text (`Msg::DesktopLyrics`) | a `Feed` |
| Setting | `views/settings_page.rs` `SETTINGS`, `menus/dropdown.rs` | the app · UI | a kept choice: its key, its choices, what it does | `Setting` |
| Flyouts | `views/menus/mod.rs` on novaui `Flyout` | the window · UI | every menu and dropdown list; one main, one beside it | rows |
| Queue rows | `views/side_panel/queue.rs` on NovaUI `VirtualList` | the shown list · UI | the rows in view, reused as they scroll; a dragged row and the rows giving way to it | `QueueRow` (the sheet's), its fill |
| Fade between two | `now_playing.rs` slots and word blocks | the face · UI | artwork, backdrop and words changing songs, in step with the sound's crossfade | a cover, words |

## 5. Features → where → tests

Unit tests sit beside their code; the L3 scenarios (one per area: `playback`, `lyrics`, `queue`,
`window`, `product`, `plugins`, `manual`), the screenshots and the perf check are in the private
suite, named here by scenario.

| Feature | Where | Tests |
|---|---|---|
| Playing, end, errors, hand-over, crossfade | `playback/session.rs`, `engine.rs`, `app/mod.rs` `turn` | scenario `playback`; `engine.rs` `failure_tests`; `engine/http_tests.rs` (Linux, L5) |
| Lyrics: groups, words, layers, styles, motion | `library/mod.rs`, `views/side_panel/lyrics.rs` | scenario `lyrics`; `library` unit tests (`lrc_groups_and_offset`, `layers_sit_under_their_lines_in_order`); `lyric_styles.rs` test |
| Queue: rows, scroll, search (ranked on a worker: `SidePanel::positions`, `Job::Found`; the list's signatures kept while it is typed, a search typed on looks only at what the last one found), keys, autoscroll, list switching | `views/side_panel/queue.rs`, `library/queue.rs`, `lists.rs`, `app/query/` | scenario `queue`; `queue.rs`, `app/query` unit tests |
| Window sizes and layout | `ui/player.nui`, `views/layout.rs` | scenario `window`, screenshots |
| Product regressions, settings page, menus | `views/` | scenario `product`, `search_style_tests.rs`, `settings_page.rs` test |
| Desktop lyrics | `views/desktop_lyrics.rs` | its unit tests (lines, sweep, sizes, place, the painted plate) |
| Playlists on disk | `library/store.rs` | `list_roundtrip`, `files.rs` tests |
| Row cache, faces kept, covers | `library/disk_cache.rs`, `row_cache.rs`, `plugins/client.rs` `cache`, `covers.rs` | their unit tests; `artwork_cache_shares_content_not_album_metadata` |
| Plugins: host, calls, sign-in, packages, controls, PCM | `plugins/` | `host.rs`, `client.rs`, `registry.rs`, `controls.rs`, `pcm.rs`, `prints.rs` unit tests (with `examples/demo_plugin.rs`); scenario `plugins` (`plugins_check`, run by `product_regressions`) |
| Online thumbnails | `plugins/prefetch.rs`, `client.rs` | `online_thumbnails_prefetch` (L5 manual, scenario `manual`: a real plugin) |
| Output choice, devices | `playback/output.rs`, `menus/output.rs` | `choice_round_trips_and_routes`; `output_device_inventory` (L5 manual) |
| Volume kept, effects kept and scanned | `playback/gain.rs`, `effects.rs` | their unit tests |
| Files whole, crash save | `files.rs`, `crash.rs` | `files.rs` tests |
| Performance (idle, turns, stalls, frame cost, resize) | whole app | measured outside this repository (Windows) |
