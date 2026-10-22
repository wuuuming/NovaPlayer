# NovaPlayer plugins (protocol v3)

A plugin adds a capability NovaPlayer does not have: a sound format, playlists from a service,
a connection to it (an account, or a server of your own), metadata for the songs it lists. It is a program in any language, run as a
resident child process; NovaPlayer talks to it in JSON lines over its stdin / stdout.

NovaPlayer ships no plugin for any service and no service credentials. A plugin that talks to a
service uses that service's published API under its terms, with credentials its author obtained
from the service. NovaPlayer plays only what a plugin hands it: PCM a decoder produces, or a
stream address a service gives out. It does not decrypt protected files, and a plugin that does
is not one NovaPlayer is built for.

What belongs in a plugin and what in the player: `extensions.md` §1. In short, everything that
only this service has (its API, its sign-in, its metadata, its streams) is the plugin's; the
plugin panel, the credential vault, queues and playback are the player's. A plugin never draws
anything of its own: it declares what it can do and answers calls with data.

## Installing

A plugin is a folder, or a `.zip` / `.tgz` of one:

```
<data>/NovaPlayer/plugins/<id>/
  plugin.json
  <program>            (and whatever else it needs)
```

`<data>` is `%LOCALAPPDATA%` on Windows and `$XDG_DATA_HOME` (`~/.local/share`) on Linux. The
title bar's menu button (☰) opens the app menu: 安装插件… installs a chosen package,
打开插件目录 shows the folder. Each plugin's panel opens beside the app menu, as below.

A plugin runs with the user's own rights; there is no sandbox. So an installed plugin is off.
Pressing it in the flyout opens its panel beside the flyout: the capabilities and hosts it
declares (shown while enabling), the 启用 row (启用？ on the first press, enabled on the second),
the connection, and 删除插件 at the bottom (stops it, removes its folder and stored credential). A plugin that cannot run
(other protocol version, bad manifest, missing program) shows a warning icon in the flyout; its
panel holds only name, version, why, and 删除插件. Esc closes the panel; a second Esc, the flyout. NovaPlayer records a hash of every file of the folder when it is
enabled, and a plugin whose files changed since is off again (已更改，需重新确认) until confirmed.

## plugin.json

```json
{
  "id": "my-plugin",
  "name": "My Plugin",
  "version": "1.0.0",
  "api": 3,
  "exec": "my-plugin.exe",
  "network": ["api.example.org"],
  "concurrent": true,
  "provides": {
    "decode":   { "ext": ["xyz"] },
    "source": {
      "schemes": ["myservice"],
      "links":   ["myservice", "example.org"],
      "connect": { "flows": ["qr", "form"],
                   "fields": [ { "id": "server", "label": "@server", "kind": "url" },
                               { "id": "password", "label": "@password", "kind": "password" } ] },
      "lists":   true,
      "option":  { "id": "quality", "name": "@quality" },
      "style":   { "res": "{res} · {option}" }
    },
    "metadata": true,
    "thumbs":   true
  },
  "contributes": { "controls": [] },
  "strings": {
    "zh-CN": { "quality": "音质", "server": "服务器", "password": "密码" },
    "en":    { "quality": "Quality", "server": "Server", "password": "Password" }
  }
}
```

- `id`: letters, digits, `-` `_` `.`; the folder name it installs under.
- `api`: the protocol version, `3`. A plugin asking for another is listed, not run.
- `exec`: the program, relative to the folder (no `..`, not absolute).
- `network`: the hosts it says it reaches, shown before it is enabled (not enforced).
- `concurrent`: it answers overlapping calls (see The pipe). Default `false`.
- `provides`: what it does; each part is optional. Only declared calls are made.
  - `decode.ext`: file extensions it decodes.
  - `source`: the plugin is a source of songs (a service, a server). Its parts:
    - `schemes`: URI schemes of the songs it resolves (`myservice:song:1`).
    - `links`: link schemes or hosts it turns into a playlist.
    - `connect`: see §connect. `flows` is required and not empty; `fields` only with `form`.
    - `lists`: the connected user's own playlists can be listed (`lists`); the player offers
      them for import (导入歌单…).
    - `option`: one choice its songs come in (quality, …); `style.res` is the template of the
      line under the artwork: `{res}` the stream's own resolution, `{option}` the chosen value's name.
  - `metadata`, `thumbs`: it answers those calls.
- `contributes.controls`: buttons it adds to the player's face (`extensions.md` §A).
- `strings`: the plugin's own texts, a `key → text` table per locale (`zh-CN`, `en`, …; a key
  missing from the player's language is taken from `en`). `name`, `option.name`,
  `connect.fields[].label` and the texts of its control fragments may be `@key` instead of a
  literal: the player shows that key's text in its language, and again in the new one after a
  switch. A plugin's keys are its own (`@quality` reads its `quality`, never the player's).

## The pipe

Each request is one line, a JSON object with `id` (a number, unique per process run) and `call`
(the call's name), and the call's parameters as further fields. Each answer is one line with the
same `id`:

```
→ {"id":7,"call":"open","path":"D:\\Music\\a.xyz"}
← {"id":7,"ok":true,"rate":44100,"channels":2,"frames":9261000}
← {"id":8,"ok":false,"error":"unsupported variant"}
```

- An answer may carry raw bytes: `"bytes":K` in its line, then exactly K bytes after the `\n`.
- A line without an `id` is ignored. Write nothing else to stdout; diagnostics go to stderr
  (NovaPlayer logs them as `[plugin id]`).
- **Pipelining.** Answers are matched to calls by `id`, never by order. A plugin with
  `"concurrent": true` gets requests as they come, several at once, and may answer each as soon
  as it is done and in any order; a slow `playlist` does not hold up `resolve`. It must write
  each line (and its raw bytes) whole, never mixed with another's. Any other plugin gets one
  request at a time: the next comes after the answer, and the player's playback calls (`open`,
  `read`, `resolve`, `stream`) go ahead of the others waiting.
- A call has a time limit: `read`, `play_event` and `connect.cancel` 5 s, `open` 10 s, `thumbs`
  60 s, `playlist` 180 s, the rest 20 s (`novaplayer_plugin_protocol::timeout`; a plugin built on
  the kit gives itself `budget`, the limit less a tenth, at most 5 s, to write its answer). An
  answer after it is dropped. A plugin with no other call in flight is
  hung: it is killed and started again on the next call. One that exits is started again, at most
  three times a minute.
- Lines are at most 16 MiB, raw bytes at most 64 MiB per answer.

Every request carries `locale`, the player's language (`zh-CN`, `en`); texts the plugin makes
(errors, layer and option names, a playlist's group) should be in it, else in English. It may
change between two requests (the user switched).

Every plugin answers `hello` (its name and `api`) once started.

## decode

For files whose extension is in `provides.decode.ext`.

| call   | parameters               | answer                                          |
|--------|--------------------------|-------------------------------------------------|
| `open` | `path`                   | `rate`, `channels` (1–8), `frames` (total, or omit if unknown); optional tags: `title`, `artist`, `album`, `album_artist`, `track`, `date`, `genre` |
| `read` | `path`, `at`, `frames`   | `frames` (how many follow, 0 at the end), then `bytes` of PCM: signed 16-bit little-endian, interleaved |

`read` may be asked for any `at` (a seek is a read elsewhere); NovaPlayer reads ahead in blocks of
about 100 ms. The PCM is presented to the playback core as an ordinary PCM stream, so resampling,
the output device, gapless playback and seeking work as for any file.

## Songs of a service

A song of a service is a URI with one of the plugin's `schemes` (`myservice:song:123`). It is
the plugin's wherever it is found: in a list the plugin made, or in any `.m3u8` on disk.

| call       | parameters              | answer                                                          |
|------------|-------------------------|-----------------------------------------------------------------|
| `playlist` | `url`, `credential`?    | `m3u8`: the list as an extended M3U8 text (`#PLAYLIST:` the name, then `#EXTINF` / `#EXTALB` and the song's URI); `unavailable`? (songs it left out, a count) |
| `resolve`  | `uri`, `option`?, `credential`? | `url`: a playable address (http(s) or a local file); `option`?, `option_name`? (the value it was given at) |
| `options`  | `uri`, `credential`?    | `values`: `[{id, name, label}]`, in the plugin's order           |
| `thumbs`   | `uris`, `credential`?   | `thumbs`: `{uri: cover URL}` for as many as it can (small covers) |
| `command`  | `name`, `uri`, `credential`? | `toast`? (text to show), `refresh`? (the song's face is asked again) |

- `playlist` is asked for links whose scheme (or host, for `https`) is in `provides.links`: the
  playlist menu's 从链接导入. A song the service withholds is left out of `m3u8` and counted in
  `unavailable`.
- `resolve` is called right before a song plays. The address is used once and never stored:
  services forbid caching their streams. `option` is the user's chosen value of `provides.option`.
  A song that does not come in it is given at another value, named in the answer's `option`:
  the player says so in a toast once, until the user picks a value again. A song the service
  offers only as a preview clip fails, with the reason as its `error`.
- `command` is a control's `plugin.<name>` press (`extensions.md` §A).

## connect

For sources that need the user connected: signed in to a service, or logged in to a server of
the user's own. The player owns the panel (the 连接 section of the plugin's panel), the poll
timing and the vault; the plugin owns only the service's flow.

`provides.source.connect`: `flows` (`qr`, `url`, `code`, `form`; the first is used), and for
`form`, `fields`: `[{id, label, kind}]` (`kind`: `text` (default), `url`, `password`; at most 6,
ids unique). `provides.source.lists: true` when the connected user's own playlists can be listed
(the player then offers them for import after connecting).

| call             | parameters             | answer                                                       |
|------------------|------------------------|--------------------------------------------------------------|
| `connect.begin`  | `flow`, `fields`? (`form`: `{id: value}`) | `session` (an id of the plugin's), `show` (not for `form`), `poll_ms`?, `expires_s`? |
| `connect.poll`   | `session`              | `state`: `waiting`, `done`, `expired`, `failed`. `waiting`: `show`? (a renewed one). `done`: `credential`, `display_name`?. `failed`: `error`? |
| `connect.cancel` | `session`              | (the user gave up or the player timed out: drop the session) |
| `connect.end`    | `credential`           | (the user disconnected: end the account on the service if it has such a thing) |
| `lists`          | `credential`           | `lists`: `[{url, name, group?, count?}]`; every `url` is one `playlist` accepts |

- `show` is exactly one of `{"qr": "<text>"}` (the player draws it as a QR code),
  `{"url": "<https://…>"}` (opened in the browser) or `{"code": "<text>"}` (shown to be typed in
  at the service).
- `form`: the player draws the fields (a `password` is masked and never reaches the clipboard)
  and sends the values once, with `connect.begin`; the first `connect.poll` follows at once. The
  values are kept nowhere: only the `credential` the plugin returns is.
- `poll_ms`: how often `connect.poll` is asked (500–10000, default 2000). `expires_s`: how long
  the connection may wait for the user (at most 900, default 300). The player calls
  `connect.cancel` on any way out but the plugin's own verdict.
- `credential` is a string the player does not read: whatever the plugin needs to act for the
  account later (cookies, a JSON of keys). The player keeps it in the system's credential store
  (Windows Credential Manager, the Secret Service on Linux) under `NovaPlayer/<plugin id>`, and
  passes it as `credential`, only to that plugin's calls (`playlist`, `resolve`, `options`,
  `thumbs`, `command`, `lists`, `play_event`). It is absent when nobody is connected. A
  plugin may refresh it and keep the newer one in memory, but it never gets a new one back to the
  vault: a credential must outlive a refresh by itself.
- The user's password never passes through NovaPlayer for `qr` / `url` flows; in a `form` it
  passes once, to the plugin only.

## metadata

For the songs of a service (never local files). A song's face is the same for every listener and
is asked for wherever a row comes into view, kept on disk and shared by every account, so the
call carries no `credential`.

| call       | parameters                                       | answer                         |
|------------|--------------------------------------------------|--------------------------------|
| `metadata` | `uri`, `title`, `artist`, `album`?, `duration`? | any of `title`, `artist`, `album`, `album_artist`, `track`, `date`, `genre`, `cover` (a URL or local path), `lyrics` (LRC text), `layers` |

`layers` gives extra lines under the lyrics: `{"trans": {"name": "翻译", "lrc": "<LRC with the
lyrics' own stamps>"}, "roma": {"name": "罗马音", "lrc": "…"}}`. The player knows `trans` (a
translation) and `roma` (a romanisation) and shows them with its own buttons
(`extensions.md` §A5); other ids are ignored.

`lyrics` may time each word (enhanced LRC): a stamp before a word inside the line,
`[00:12.00]<00:12.00>word <00:12.40>by <00:12.80>word <00:13.20>`; a last stamp after the last
word says when the line is fully sung. A service's own word format (NetEase `yrc`, QQ `qrc`) is
turned into this by its plugin. Lines without word stamps are lit whole, as before.

## play_event

Sent for songs of the plugin, for services that ask for accurate play reports.

| call         | parameters                                  |
|--------------|---------------------------------------------|
| `play_event` | `uri`, `event` (`start`, `pause`, `resume`, `seek`, `end`, `skip`), `position` (s), `credential`? |

Events reach the plugin in the order they happened and are never waited for by playback.

## Example

`crates/novaplayer/examples/demo_plugin.rs` implements every call above with no network and no
account (a `.tone` format, `demo:<n>` playlists, a QR connection that succeeds after two polls and a
form one that succeeds at once, a
`slow` call to show pipelining). It is `concurrent`. The plugin tests run it as a child process.
`crates/plugins/kit` is the shared plumbing for service plugins (the pipe loop, JSON, HTTPS).

## Audio effects (CLAP)

Audio effects are not plugins of this protocol: they are [CLAP](https://cleveraudio.org)
plugins, loaded into the player and run on its audio. Settings › Effects lists the chain
and the effects that can be added.

- **Found** in `CLAP_PATH`, then `%COMMONPROGRAMFILES%\CLAP` and
  `%LOCALAPPDATA%\Programs\Common\CLAP` (Windows), `~/.clap` and `/usr/lib/clap` (Linux).
  Each `.clap` file is opened by a child process, `novaplayer --scan-clap <file>`, so a plugin
  that crashes as it loads fails only its own scan. What a scan found is kept in
  `<data>/clap-scan.txt` by path, size and time of change; "Look again" scans again.
- **Hosted:** only effects (feature `audio-effect`) with a stereo main input and output. The
  chain runs after ReplayGain and before the volume, on the thread that already runs the chain
  (Windows: the decode worker; Linux: the Pulse writer), never the device's callback, in stereo
  32-bit float at the session's rate; mono is doubled in and mixed out. Each effect's
  `clap.latency` is compensated, so the sound stays on time, also when bypassed. A seek resets
  every effect; a change to the chain takes effect at the next seek or song.
- **Main thread:** one plugin host thread per process, not the UI's: a Win32 message loop on
  Windows, a `poll` loop on Linux over the descriptors and timers plugins register
  (`clap.timer-support`, `clap.posix-fd-support`).
- **Its window:** above the player's (`clap.gui`): the effect's own floating window when it
  offers one, otherwise (most effects; JUCE offers none) its view embedded in a window of the
  host's (`player_core::clap` `frame.rs`), sized to what the effect asks; on a Wayland desktop
  through XWayland. Closing it hides the view; removing the effect destroys it. Building the
  view is the slow part (Airwindows Consolidated: 0.7 s), so it is built hidden in the
  background when Settings › Effects opens and when an effect is added; Open then only shows
  it, and never makes the player wait.
- **Parameters** (`clap.params`): Settings › Effects › an effect lists them, hidden ones and the
  effect's own bypass left out, each under its module's heading with its value as the effect
  prints it: a slider; a switch for a stepped one of two values, a dropdown for one of up to 32;
  a read-only one only shows its value. A value set there is handed to the effect with its next
  block while the sound plays (within 400 ms of its last block), through `params.flush` on the
  plugin host thread otherwise; either way it is in the effect's window and in its state. What
  the effect changes itself (its window, its output events, `params.rescan`) moves the chain's
  change count: the page reads the values again and the chain is written two seconds later.
- **Failure:** an effect that reports an error, answers with a NaN or an infinity, or cannot run
  at the session's rate is taken out of the chain for the rest of the session and its card says
  why; the sound goes on without it.
- **Kept:** the chain in `<data>/effects.txt` (a line per effect: `on`/`off`, plugin id, file),
  each effect's `clap.state` in `<data>/effects/<n>.bin`, written when the chain changes, two
  seconds after a parameter or an effect's state changed, and at exit.

