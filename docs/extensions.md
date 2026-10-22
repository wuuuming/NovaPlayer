# Three layers and how plugins extend the player

## 1 Who owns what

Judge a new feature by who else would use it.

| Layer | Owns | Test |
|---|---|---|
| novaui (framework) | elements, layout, text, the `.nui` DSL, images, tasks, platform, generic widgets (a QR image, `qr::image`) | another product would use it |
| novaplayer (app) | queue, library, playback, the plugin panel, the credential vault, the plugin host, the places plugins may add to | another service would use it |
| plugin | one service: its API, its sign-in flow, its metadata, its streams, its controls' looks | only this service has it |

The app contains no detail of any service (NetEase, QQ, …): no host name, no endpoint, no
cookie name, no id format. A service is named only in its own plugin. What a plugin hands the
player is data the player understands (a playlist as M3U8, an address to play, a string to draw
as a QR code) or opaque data the player only stores and returns (the credential).

The protocol is `plugins.md`. A plugin never reaches into the player; it answers calls, and
declares in `plugin.json` what it provides and contributes.

## B Plugins declare, the player draws

A plugin never changes the player's layout. What a plugin can show is chosen by the player from
what the plugin declares in `plugin.json` and drawn by the player from data the plugin answers:
the sign-in (a QR, a link, a code, or form fields the plugin names), the lists it offers, its
settings. A plugin's looks reach the face only through a slot (§A), never a panel of its own.
If a flow needs more than the player's panel shows, it finishes in the browser (the `url` flow).
The plugin panel and protocol v3: `plugins.md` §2.1.

Runs where: plugins of `plugins.md` are separate processes on a JSON pipe (any language; a
crash does not take the player down). Realtime audio never goes over the pipe: up/downmix is
the playback core's own, and audio effects are CLAP plugins hosted in process by the core
(`plugins.md` §"Audio effects (CLAP)"). The core owns the hosting (`player_core::clap`:
loading, the plugin host thread, the chain stage, failure); the app owns the user's chain
(`playback/effects.rs`: which effects, kept where, scanned how) and its page, Settings ›
Effects. A plugin's window is the plugin's own; the core gives one that only embeds a plain
window of its own (`clap/frame.rs`, Win32 / X11) above the player's. No service is involved, so no plugin of `plugins.md` can add or reach an effect.

## A Plugin controls

A plugin may add buttons to the player's face. The player owns where and how big (the slot),
what a control may read (the context keys) and what a press may do (the commands). The plugin
owns only the looks: a `.nui` fragment from its folder.

### A1 Declaring

```json
"contributes": { "controls": [{
  "id": "like", "slot": "lyrics-tools", "ui": "ui/controls.nui", "component": "Like",
  "when": "song.plugin == self && page == \"lyrics\""
}] }
```

`ui` is a path inside the plugin's folder. `when` is optional; without it the control is always
mounted. A fault in a control (bad slot, unreadable fragment, a command not allowed) is shown
on the plugin's row; its other capabilities stay.

### A2 The fragment

Read by the framework's `parse_fragment` with a fixed profile: elements `Row Column Stack Box
Text Path Button Spacer`; looks-only properties; no position, size, layer or z order; at most
64 KiB and 256 nodes. It cannot bind the player's own names, only the context keys below.

### A3 Context keys

What a fragment or its `when` may read:

| Key | Value |
|---|---|
| `page` | `""` (panel closed), `lyrics`, `queue`, `info` |
| `playing` | a song is playing |
| `song.plugin` | the playing song's plugin id (`""` for a local song) |
| `self` | the id of the plugin that owns the control |
| `song.layers.<id>` | the playing song's lyrics come with that layer |

### A4 Slots and commands

One slot today: `lyrics-tools`, the lyrics page's foot, after the player's own layer buttons
(§A5), where a control takes the player's `ctl` style. A press (`on-click`) names a command:

- `option.open`: open the flyout of the song's option values;
- `plugin.<name>`: the plugin's own `command` call (`plugins.md`); its answer may carry a toast
  and ask for the song's face again.

### A5 Lyrics layers

A plugin's `metadata` answer may carry `layers`: extra lines under the lyrics, as LRC text with
the same stamps as the lyrics. The player knows two, by id: `roma` (a romanisation) and `trans`
(a translation); others are ignored. Showing them is the player's, not the plugin's: any source
(a plugin today, a local file later) gives the same feature. The player lays each layer's line
under the lyric line of the same stamp (within 50 ms), romanisation first, and puts a button
for each layer the song carries in `lyrics-tools` (lit while on). A press folds the layer away
under its lines, or unfolds it from under them, in a ripple from the line at the focus
(`novaplayer-spec.md`); the choice is kept per layer
id, for every song (`lyrics-layers.txt`).

## C Lyric styles (contract v1)

How a lyric line looks is a style: a `.nui` fragment, not player code. The built-in look is
the first style (`ui/lyric-style.nui`), written against the same contract a community style
gets. The product ships that one look; others are installed (packages, below). A style is
declarative only: no scripts, now or later; what it can do is what novaui draws cheaply.

- **Components.** `LyricLine(line)`: a group's first line (the one with word timing), and
  `LyricReading(line)`: each reading or translation under it. One `Text` each.
- **Profile** (`STYLE_PROFILE`, `parse_fragment`): element `Text`; properties `text fg fg-alpha
  weight opacity translate scale sweep sweep-fg sweep-alpha`, and (v1) `units timing time
  u-translate u-scale u-opacity u-fg u-rotate u-glow text-rotate text-origin`; at most 16 KiB
  and 16 nodes. Pure calls (`min max clamp round lerp smoothstep sin cos spring mix keys …`)
  are allowed; `tween`, `window.*`, `let` are not. Size, wrap and the group's place stay the
  player's (the size rules); a style moves ink, never layout. What each property and `unit.*`
  value means: `novaui/docs/novaui-spec.md` (text units).
- **Keys** the player writes for every visible group each frame it moves (a group off screen
  keeps its last values):

| Key | Value |
|---|---|
| `light` | 0..1: how lit the group is (the sung group fades in, the one it leaves fades out) |
| `distance` | groups from the sung one (negative: already sung) |
| `edge` | 0..1: 1 inside the side panel, falling to 0 across the band at its top and bottom |
| `hover` | 1 while the pointer is over the group, else 0 |
| `sweep` | chars of the first line sung, fractional inside a word; -1 without word timing or when not the sung group |
| `time` (v1) | the song's position in seconds, held inside the group's window (0.5 s before its first part to 1 s after its last) |
| `timing` (v1) | set once: the first line's parts as `[first char, from, to, …]` in seconds; a line without word stamps is one part from its stamp to the next group's |
| `offset` (v1) | dips from the group's middle to the side panel's focus, as it scrolls |
| `middle` (v1) | dips from the group's middle to the window's middle (a one-line style holds its line there) |

- **Budget.** Only the sung group's `sweep` moves at the display rate (while playing); the
  rest change with the group's own motion. A style that reads `time` asks for frames only
  while playing, for the groups inside their window. The framework re-evaluates only the
  expressions that read a key that changed; a style that reads no v1 key (the built-in one)
  costs what v0 did. A style written for v0 keeps working.
- **Packages.** A folder holding `style.json` and the `.nui`:
  `{ "id": "one-line", "name": { "en": "One line", "zh-CN": "单行" }, "style": "style.nui" }`
  (`name` a text or one per language; `style` a path inside the folder, `style.nui` when not
  named). Installed from the folder (pick its `style.json`) or a zip / tar.gz into
  `<data dir>/lyric-styles/<id>/`, checked against the profile first; chosen, installed and
  deleted at 外观 › 歌词样式 (`app::lyric_styles`, setting `lyric-style`). A chosen style that is
  gone or no longer reads falls back to the built-in one. The example:
  `examples/lyric-styles/one-line`, the most motion the product's own examples go to.
