# Design principles

The rules NovaUI, NovaPlayer and Player-Core are held to. Each one was found the hard way; the
example given with it is the mistake that taught it.
Check a design against them before writing code, and a diff against them before committing.

Rules are numbered by part (`D` design, `R` runtime and performance, `B` robustness, `N` names and
code). Refer to a rule by its number and name ("R3 A skip compares every
input"), not by a section number of this file.

Ideas that are not rules yet (proposed, not done) do not live here.

---

## Part D — Design: what to build and where

### D1 Simplest first

The simplest design that works usually turns out best. Before adding a type, a cache or a flag,
ask what the plain version costs. A first pass once added a view-model struct and a cached copy per
view; the simple version was to write the value and let the setter drop an equal one.

### D2 One idea, one implementation, in the layer that owns it

An idea is a *semantic*: what a thing means and promises, as its user or caller sees it ("the next
song starts with no gap"). Specs write semantics down; *mechanisms* are the code that realises
them. Each semantic has one mechanism in the layer that owns it, but a mechanism may carry many
semantics: ReplayGain, effects, gapless handover and crossfade are four semantics on one DSP
chain. A mechanism carries many semantics well when each comes as a part it is handed (a stage,
a factory, data) and it never branches on which one it serves; a branch per semantic is D3's
`mode` flag. A new semantic first asks which existing mechanism can carry it. Hosts differ in
mechanism only where the platform forces it, and keep the same semantics (B5).

Where a mechanism goes follows from its semantic's role, not from where it was first needed:
its *scope* (what it lives as long as: a song, the stream, a device binding, the process), its
*place in the flow* (before or after what, on which thread: real-time, decode, host loop) and its
*layer* (below). ReplayGain belongs to a song, so it runs per song before songs are summed;
effects and volume belong to the listener, so they run once on the sum, volume last before the
device; a crossfade is between songs, so it sits where songs meet. A part placed where it was
first needed (effects built per song because the chain was) pays for the mismatch in guards and
special cases; those guards are how a review finds it.

The layer is decided by `extensions.md` §1: what another product would use goes to novaui; what
another service would use goes to the app; only what is specific to one service goes to a plugin.
If two views need the same mechanism it belongs in novaui or a shared module, not in each view.
If the framework already does it, use that (`Doc::set` and the tree's setters compare before
they mark; the text cache keys by text, size, line, weight and width).

Unify, reuse and simplify are one move. The design language, the modules, the APIs
and the data are unified; **the entry points are not**. A thing can have many ways in (the side
panel opens from the queue and lyrics buttons, Q and the button beside the artwork); each one
leads to the same thing. In order:

1. **Name the thing, not the place.** Say what it is apart from where it shows ("a plugin as the
   user sees it", "a choice kept in settings"). If the only name you find is a place ("the
   panel's rows"), you are about to model a view and copy it for the next place.
2. **Look before you build.** Search the code, novaui (widgets, `.nui` components, styles) and the
   app's helpers. Read the nearest existing instance completely; extend it (a parameter, a
   generalised name) instead of writing a sibling.
3. **Model the data once, where the facts are.** One plain struct holds what the user is shown;
   one function derives it from the owners. Views read it and work out nothing on their own.
4. **Entries are thin.** An entry turns input into the one shared `Command` or the one model. A
   container (flyout rows, cards, a dialog) only lays the model out in its visual language.
5. **The second instance generalises the first.** When a second copy appears, turn the first into
   the mechanism before the copy lands (`drop:<key>` / `set:<key>:<value>` with one table of
   choices). A third instance costs only data.
6. **Test: can two entries ever disagree? Can the next one be added by data alone?** Write code
   once both answers are right.

Worked examples: `PluginView` drawn by the app menu's panel and by the settings page; the font box
built from `Query`, the dropdown flyout and the `lit` state.

### D3 How far to merge

Merge two things only when all four hold:

1. **They change for the same reason** (fixing one means fixing the other).
2. **The user calls them by the same name.**
3. **What differs is data, not behaviour.**
4. **The merged code never asks who called it.** A `kind` / `mode` flag that changes what it does
   means two things share one part: share that part, keep the two.

Stop where the differences are behaviour. The queue search and the font box share the matching
rule (`Query`), not the box. Two `36`s merge only if both mean "a row of body text".

### D4 One owner per fact; set the cause, derive the result

1. **Every fact has one owner.** Looks (sizes, weights, gaps) are the sheet's; facts (a plugin's
   state, the queue) the model's; the user's choices `settings.txt`'s; what is playing the
   engine's. Everything else reads the owner and keeps no copy. The copies of 28, 700 and 36 are
   what drifted.
2. **Set the cause, derive the result.** Rust sets `single-column`; the sheet derives the columns.
   The window gives `room`; the heading rule derives every size.
3. **The UI shows the user's intent, derived from the one owner of it.** A transient backend
   state is not what the user asked for: the cover does not dip on a repeat-one restart because
   it follows `wants_play`, not the decoder; a recovery keeps the published play intent; an
   engine self-pause counts as held.
4. **The fewest controls that reach every outcome, each one a designer would name.** Two controls
   that always move together are one; one control that must move two things in opposite
   directions is two.
5. **A rule over a list.** A rule with named inputs, checked against a list of target cases, controls
   every case. An exception lives next to the rule with its reason; a second exception means the
   rule is wrong.

Every review item takes both tests: *merge* (D3) and *control* (D4).

### D5 Layout belongs to layout

Sizes that depend on content or on the window are written in `.nui` (`let`s, expressions,
percent lengths, shape conditions), not measured and pushed from Rust. Rust reads a laid-out
rectangle only after `resolve_layout`, once, for geometry that depends on another element (a tip
over a button). Code that measures text reads the sheet's size and weight, never a copy of the
numbers; a row count times 36 is such a copy.

Text under a scale law keeps its *rank*, not only its size: headings compress toward the body as
the room shrinks and get lighter as text grows (the numbers: `novaplayer-spec.md` §1.2).

### D6 Text fits the layout, not the other way

- **The interface's own text is never cut.** Labels, buttons, settings rows and menu items take
  the room their words need in every language.
- **Wording is never shortened to fit**; a shorter text is right only when it is also clearer.
- **Only content is truncated** (titles, artists, paths, playlist names), and its whole text is
  reachable elsewhere (the tip, the info panel).

### D7 Boundaries are opaque

- **Opaque across a boundary.** The vault stores a `credential` string and hands it back to the one
  plugin it came from; the player never parses it. When the app reads a value a plugin gave (a
  cookie name, a JSON field), a service detail has leaked into the app.
- **The owner of the fact sets the knob.** Poll cadence, session lifetime and what to show come
  from the plugin; the player only clamps them.
- **Wait by ticket, not by order.** Match answers to callers by `id`, so nobody waits for the
  slowest.
- **Every exit path tells the other side.** One helper per exit (`connect.cancel`, `connect.end`),
  not a copy at each `return`.
- **Unpublished means no adapter.** Migrate the in-tree users and delete the old names.
- **Generic widgets live a layer down** (`novaui::qr::image`, not a sign-in feature).

### D8 Lifetimes are values

- **A dropped value is a free nobody writes.** Content holds the resource itself (`ImageHandle` in
  `Content::Image`); replacing it or removing the element is the free. Test for a new resource
  API: can the caller leak or double-free with it? Then it wants a handle.
- **Identity is content where content is immutable.** Interning equal pixels into one handle made
  texture aliasing unnecessary. Prefer an identity that makes a mechanism disappear over a
  mechanism that recovers the identity.
- **Sweep on a counter, not on a frame.** State that names elements is cleaned when
  `Tree::removal_version` moved since the last look; idle it is one compare.
- **A state machine carries its table** on the type. Check for chords before merging flags into
  one state.

---

## Part R — Runtime and performance

### R1 Work comes from causes, not from frames or turns

- **Idle is zero frames and zero wakes.** Nothing runs at vsync unless something moves. Each
  time-driven visual has its own rate (backdrop drift 10 Hz, clock labels 1 Hz, crossfade and
  scroll at display rate only while they run).
- **Fewer wakes matter as much as cheaper ones.** A wake lifts a CPU/GPU to its high state however
  little it does; timers that can share a tick share it. A paused WASAPI stream woke 100/s until
  the patched cpal (player-core `vendor/cpal/PATCHED.md`).
- **A frame that changes nothing is not presented.** The driver's cost is per present, not per
  pixel. A turn that changes nothing is not followed at once by another.
- **One message per cause, one present per turn.** Every cause notifies one `presentation`
  model; its subscription presents once in the flush.
- **Ask during the turn, settle once at its end.** A view asks its own `Clock` for another frame or
  a deadline while it presents; a view that stops asking stops its ticker by itself.
- **A wake names its owner** (`Msg::SidePanel`, `NOVAUI_TRACE` shows the line that armed it).
- **A deadline that cannot matter is not asked for** (a paused song's next lyric line, a hidden
  view).
- **A change is pushed once, not compared every turn** (subscriptions, not version polls).
- **Time belongs to whoever drives the work.** A caret blink is novaui's deadline; a widget the app
  syncs returns its next wake (`#[must_use]`) to its owner's clock.

### R2 Cut redundant work; measure

Work that produces nothing is removed, not hidden. A per-frame "derive, then compare with last
time" only hides the cost: compute a fact where it changes (the mutator), on a wake (a clock),
or in the framework's expressions. **No speedup is claimed without a measurement**, and a
measurement says where the cost went, not only that the first one disappeared (one `Arc` around
20 000 songs moved the copy to the first change; an `Arc` per song removed it).

### R3 A skip compares every input of the work it skips

A dirty check, a cache key or a "same box" test claims the output cannot have changed.

- **List what the work reads and compare exactly that**, at the precision it reads. `record` reads
  `f32` dips, so the check compares dips, not the pixel rect they round to (3-in-7 missing glyphs);
  the ellipsis must use the same width the layout gave (149.99998).
- **Flaky means an input you did not list.** A deterministic renderer that fails some runs has an
  input varying by run reaching a skip that does not compare it. Look for the skip, not the race.
- **Movement is not paint: text stays still when its box moves.** A box that only moves (same
  size, opacity, target and visibility) keeps what it recorded and is damaged at its old and new
  place (`MOVE`, not `PAINT`); a scrolled layer's rows move with it. So every content kind
  records in its own box's pixels and reads nothing of where the box is: not its window
  position, not an ancestor's clip, not the pixel phase of its origin. The one input that is
  position is deferred text (text outside its clip records nothing, `text_pending`), and a move
  that brings it into view records it. A new content kind, or a new input to `record`, is
  reviewed against this; the damage audit (`NOVAUI_AUDIT`, on in debug and CI) re-records
  elements no flag reached and fails on any difference. Text, Icon, Path, Image, ImageCrossfade
  and Effect record from size, scale, visual and state only.
- **The last writer settles.** The turn's last phase ends with one `doc.update`.

### R4 Share at the grain it changes; the UI thread never waits

- **Share what another thread reads, at the grain it changes** (`Arc` per song, not a deep copy).
- **The UI thread never blocks on I/O.** No file-system calls, network, process spawns or encodes
  on the turn: a `Path::exists` per song on an SMB share froze the rescan; a list save encoded on
  the UI thread (both fixed since). Hand it to the owning worker and receive the result as a
  message.
- **Work grows with what changed, not with what exists.** A 20 000-song list, a 1 000-line lyric,
  a 10 000-element tree: a per-turn cost proportional to the total is a bug unless measured
  harmless.

### R5 What "changed" means, by layer

Each layer owns its own change signal; nobody else keeps one for it. In code the words are
`invalidate_*` / `needs_*` and `version` (`runtime.md` §2), not "dirty".

| Layer | Owner | Meaning |
|---|---|---|
| Element | novaui tree | layout / paint / move flags; set by setters that changed something, cleared by layout and paint. |
| Binding | novaui `Doc` | a `$name` changed; only the expressions and properties that read it re-apply. |
| Model | `novaui::Model<T>` | app state changed; `notify` delivers once to each subscription in the flush. |
| Store | the app's worker | `unsaved` / `save_due`: state must reach disk; never a frame. |

Ownership and lifecycle (who creates, who frees, what ends with an element) are the framework's
job. App code that keeps its own slot ids, free queues or copies of rendered values is doing the
framework's work and is a sign the framework lacks something.

---

## Part B — Robustness

### B1 Failure is a state the user can see and leave

- A failing source, device, plugin or file becomes a message the user sees (a toast, a row
  state) and an action that recovers (retry, rebind, skip). It never becomes a panic: release
  builds use `panic = "abort"`, so one `unwrap` on a production path is a crash with no report.
- `unwrap`/`expect` are for invariants the code itself established, with the invariant in the
  `expect` text. Input from outside (files, plugins, OS, network, devices) is handled as `Result`.
- A lock that can be poisoned, a channel that can close and a thread that can die are handled at
  the one place that owns them (a helper), not at each call.

### B2 Untrusted input is bounded

Plugin messages, packages, `.nui` sheets from styles, playlists, tags and cue sheets come from
outside. Each reader caps sizes (line length, depth, count), refuses paths that leave their
directory, and times out the other side. A plugin cannot stall, crash or write outside its folder.

### B3 Persistence survives a crash

A save writes a temporary file and renames it over the old one; a load treats a corrupt or
newer-version file as missing (with a log line and the old file kept aside), never as a panic.
Tests never touch the real data directory (`CONTRIBUTING.md`).

### B4 `unsafe` states its contract

Every `unsafe` block or `unsafe impl` has a `// SAFETY:` comment naming what holds and who keeps
it (the pointer's owner, the thread, the lifetime). FFI wrappers keep raw pointers private and
expose owned types that free in `Drop`.

### B5 Every host behaves the same

A behaviour fixed on one platform is fixed on all of them in the same change (caption press
first on win32, X11 and Wayland; English errors on Linux). A host that cannot do
something says so in the spec; it does not silently differ.

### B6 Liveness has a bound

Anything that waits (a plugin call, a sign-in poll, a device open, a decode thread, a busy
idle) has a deadline and a defined outcome when it passes (timeout kill, rebind, error). A
liveness check (`runtime.md` §9) catches a turn that does not end and a wake that keeps
repeating.

---

## Part N — Names and code

### N1 One name per concept, one concept per name

A name says what the thing is to someone reading the code for the first time; private metaphors
go. One word per concept, the same in Rust, `.nui` and the docs. `size` meant a square box, a
text size and an icon size, and an `Edit` became 12×12: now `size`, `font-size`, `icon-size`.
The framework's reserved words (`turn`, `frame`, `version`, `invalidate`…) are in
`novaui/docs/runtime.md` §2; the product's glossary:

| Term | Means | Was |
|---|---|---|
| side panel | the right-hand panel: queue, lyrics, song info (`SidePanel`, `#side-panel`, `panel-open`, `panel_w`) | lane |
| face | the now-playing page: artwork, words, controls (`views/now_playing.rs`) | — |
| track | what is known of the song now playing (`NowPlayingTrack`); an online song's from its plugin is `client::TrackInfo` | face |
| layout | `ui/player.nui`'s rules sizing everything from the window (`views/layout.rs` feeds them) | law, size law |
| drag | carrying a queue row onto a playlist (`menus/drag.rs`, `RowDrag`) | carry |
| plugin controls | controls a plugin adds to the face (`contrib:` stays: protocol) | contrib |
| pending confirm | a destructive or trusting command waiting for its second click (`command::Confirm`) | armed |
| flyout | a menu that opens from a button (`#flyout`, `#plugin-flyout`) | menu, panel |
| command | what a press, a key or a field asks the app to do (`app::command::Command`) | action, menu action |
| scales | `room`, `text_scale`, `chrome_scale`, `frame_scale`, `cover_scale`, `caption_scale`, `control_squeeze`; headings `subtitle_size` / `heading_size` / `title_size`, `heading_squeeze`, `heading_lighten`; Rust reads them once (`layout::Scale`) | q, s_t, c, … |
| registry | the installed plugins (`Plugins.registry`) | plugins.plugins |
| sung | the lyric group being sung now (`sung`, `sung_at`) | lit |
| lyric scroll / motion | the lyrics page's scroll and its eased motion | lscroll, lmotion |
| band | the window less its margins, where everything is laid; `safe_*` is its capped, centred part | (kept) |
| wash / ink | `.nui` colour tokens: a translucent fill under a control / a text or icon colour | (kept) |
| lit | novaui's element state for a toggle that is on (`set_lit`, `bg-lit`) | (kept) |

- **Short names only where their scope is a few lines**: loop indices, closure arguments,
  geometry (`x y w h`), and the conventional `ui`, `id`, `v`, `m`, `p`.
- **Thread names say whose they are**: `novaplayer-*`, `novaui-*`, `player-core-*`.
- **A name is said once.** The data folder is `DATA_DIR`; a sheet number is read from the sheet.

### N2 Code says why; docs say what; neither cites a section number

- A comment explains a reason or a constraint the code cannot show. It names the rule or doc by
  title ("R3", "runtime.md, Damage"), not by section number: numbers move, titles stay
  greppable.
- A public item of novaui has a doc comment; a module starts with a paragraph saying what it
  owns.
- A file holds one responsibility. Past ~800 lines, look for the second one (`dsl.rs` parses and
  applies; `element.rs` stores and records; `library/mod.rs` scans, tags and stores).
- No public fields across a crate boundary for state with an invariant; accessors and commands
  instead (`Ui`'s fields are the known debt).
- Dead code, commented-out code and `allow(dead_code)` are deleted, not kept "for later": git
  keeps it.

### N3 Code and its doc change in the same commit

When code and a spec disagree, fix one of them in the same commit (`runtime.md`, opening note). A doc that
describes code that is not there (a module tree, a type, a switch) is a bug of the same weight as
code that breaks the doc.
