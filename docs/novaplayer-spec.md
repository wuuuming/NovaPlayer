# NovaPlayer (music) — product spec

What NovaPlayer does, as the user sees it. It is the acceptance target: a screenshot, a test or a
review checks the product against this file. Numbers are dips at the design size (`room` 1) and
scale by the layout's factors (§0.2); the owner of every number is `ui/player.nui` (principles D4),
and a difference between the two is a bug in one of them, fixed in the same commit.

Section headings name the code that implements each part.

## 0 Product

### 0.1 Scope and start

- Music only. Out of scope this round: video, SFTP remote, mini player, HDR.
- No home or picker page: first run creates an empty "播放列表", stopped. A kept list without
  playback memory restores only its queue; a kept played selection restores cover, metadata,
  controls and saved position **paused**, never with automatic sound.
- Empty state: the face shows the placeholder art (MusicNote), title "未在播放", transport disabled
  except the queue button; the side panel opens on the queue, whose empty body shows
  "拖入音乐文件或文件夹，或" and two buttons "添加歌曲…" / "添加文件夹…".
- Adding songs: file picker (multi-select), folder picker (recursive scan), drag-and-drop, m3u /
  m3u8 / cue. Added songs append to the open list; adding to an empty stopped list selects the
  first but does not autoplay.
- Platforms: Windows (player-core: FFmpeg + WASAPI / ASIO) and Linux (player-core: system
  `ffprobe` / `ffmpeg` decode, PulseAudio loaded at run time, clock-only when no sound server
  answers). A decode plugin's PCM reaches player-core through media-io's `PcmSource`
  (`plugins/pcm.rs`), one implementation for both platforms. Online sources, accounts and extra controls come from plugins
  (`plugins.md`); the app names no service.

### 0.2 Scale law and type

The window is measured against the design box of its shape (a landscape window: two columns'
1706.667×928; else one column's 744×928; 单列 only arranges, never changes the box): `room` = the
smaller ratio. `text_scale`, `chrome_scale`, `frame_scale`,
`cover_scale`, `caption_scale` and `control_squeeze` derive from it in `player.nui`; the cover,
the transport, the icons and the text scale by them. Text keeps its rank, not just its size:

- **Reading text** (body, captions, lyrics) is `base × text_scale`; floor 0.85.
- **A heading** is `14 × text_scale × (N/14)^e`, `e` from 1 at `room ≥ 0.9` to 0.5 at `room ≤ 0.5`
  (`heading_squeeze`; sizes `subtitle_size` / `heading_size` / `title_size`). On a GPD (`room`
  0.56) Title 28 stands at 17 over a 12 body.
- **Weight holds the ink.** `heading_lighten`: 0 → 50 at `text_scale` 1.28 (1440p, 4K at 150 %, an
  ultrawide) → 100 at 4K at 100 %. novaui keeps variable families at every 50.
- **Text in a box grows with the box's text**: a flyout, a text box, a toast are body text with
  room around them; one row height `flyout_row`.
- Devices to check a change against (dips, maximized): GPD 960×552 (`room` 0.56), a 2.5K 14-inch
  at 200 % or the default window (0.75), 1080p at 125 % (0.84), 1512×945 (0.89), 1080p at 100 %
  (1.08), 1440p / 4K at 150 % / 3440×1440 (1.47, height-bound), 4K at 100 % (2.24).

## 1 Now-playing face (`views/now_playing.rs`, `ui/player.nui`)

- Backdrop: 6×6 colour lattice read off the cover. Each cell = mean of its pixels weighted by `(vividness + 0.08)^2`, then 50 % own + 50 % mean of its 8 neighbours, saturation × 1.35 about luma, luma capped at 104/255 (white text ≥ 4.5:1). Drawn as a vertex-coloured mesh, 50 triangles, no texture, no blur.
  - Flow while playing: inner vertices wander ±0.34 cell, `x += cell·0.34·sin(drift·0.113 + row·1.3 + col·0.7)`, `y += cell_h·0.34·cos(drift·0.097 + row·0.9 + col·1.9)`; rim fixed. 10 fps (`FIELD_FRAME` 100 ms). Held = static, no frames.
  - With the side panel open: field darkened by 0.45·open right of panel.left, ramping in over one lattice cell.
  - Song change: crossfade of field + cover from old song to new. What does not change does not move: the same picture again (the next song of one album) does not crossfade, and the same song again (repeat one, the playing song opened anew) keeps its whole face as it is, with no stand-in while it is read again.
- Artwork: square, centred crop of non-square covers (`square_uv`), side = `520 × cover_scale` within the column (§0.2), hidden below 96 dip. Kept decoded ≤ 1040 px. Corner radius = WinUI overlay radius (8), Dialog shadow. Held (paused) scale 0.86, 0.25 s linear move. No cover: ControlFillDefault square + Fluent `MusicNote1` icon at 0.3 × side, TextTertiary. On an HDR monitor (Windows) a gain-map JPEG cover (Ultra HDR) shows its HDR rendition: its highlights brighter than SDR white, as far as the display allows; the field made from it, list thumbnails and the rest of the UI stay SDR (owner, 2026-10-21).
- Text under the art, centred, column width = `min(area, 520 × text_scale)`: title 28/36 bold (700), up to 2 lines; artist BodyLarge (18/24) TextSecondary; album Body (14/20) TextTertiary; resolution line "96 kHz / 24-bit ALAC" Caption (12/16) TextTertiary, spacing_s above. Line gap spacing_xs (4).
- Vertical block: art, gap spacing_xl (24), words, gap 16, controls; the block is centred vertically in the safe area. Controls as wide as the art, ≥ 320 dip.
- Safe area: frame minus margin 2·spacing_xxl (2×... see tokens) left/right, top = title bar inset + spacing_xl, bottom = spacing_xxl. Too small for controls → only the field.
- Scale: every metric scales by the layout's factors (§0.2).
- Side panel (lyrics or queue or song info):
  - Shape, in dips before any scale: two columns when the window is landscape (width ≥ 1.2 × height under the title bar, kept down to 1.15) and ≥ 840 wide (kept to 810), else one; 单列 always one. Two columns: art column 0.44 of width, gap 48, side panel = rest. One column: the side panel takes the whole band down to the controls, controls at the foot, art hidden.
  - Size: the window is measured against the design box of its shape: a landscape window (the shape rule above without 单列) two columns' 1706.667×928 (2560×1440 at 150 % under the title bar), else one column's 744×928 (4:5). A half-screen window is one column at its design size; the box eases over 0.3 s as the shape changes. 单列 only arranges: in a landscape window it keeps the wide box, so choosing it changes no size. In one column the safe area takes the band's whole height (no 884 cap), so a tall window gives the lyrics its height.
  - Open/close 0.30 s decelerating; side panel slides in 48 dip from the right; lyrics ↔ queue dissolve in place.
  - "Way in" button (32 dip square, 16 dip right of the art, vertically centred) appears on pointer over art, hides after the transport idle time; "back" button in the gap when a second-level page is up.
- Lyrics page :
  - Sung group at 35 % of side panel height; groups gap 24; primary line size = clamp(round(panel_w/17), 28, 40), line height 1.3×, bold; readings/translations 0.6× size (≥ 14), 1.35× lh, weight 600.
  - Ink: lit 1.0 / unlit 0.36; readings lit 0.78 / unlit 0.24; hover +0.25; lines fade over 96 dip at top/bottom of side panel.
  - Line change: scroll ease-out 0.30 s; next wake armed by `until_next` (no polling).
  - Wheel browses; after 3 s idle the side panel glides back to the sung line (the scroll motion, below). Click a group → seek to it.
  - Up to 3 wrapped rows per line.

## 2 Transport under the art (`views/transport.rs`, `playback/`)

- Rows (dip, × scale): optional top row 24 (ASIO switch at left, song-info button at right) + gap 8; scrub row 20; gap 8; button row 40. `HEIGHT` = 68 without the top row.
- Scrubber: elapsed time left, `-remaining` right, each 52 wide, Body style (14/20) TextSecondary. Track 8 tall, pill radius. Empty track = two translucent layers (white α 0x2B, then rgba(128,128,128,0x24)); fill white α 0xE6 (0x5C disabled), fill never narrower than its height. No thumb. Hit area = bar height extra above and below.
  - Drag previews only (no seek until release); click or release seeks. Opening: bar disabled, right text "正在打开"; opening/buffering → WinUI indeterminate bar animation.
  - Time format `m:ss`, `h:mm:ss` from 1 h.
- Buttons (Subtle style, radius = control radius 4): hover wash white α 0x0F, pressed 0x0A; lit toggle 0x2E (pressed 0x24). Ink TextPrimary (dark theme) or TextDisabled. Pointer = hand.
  - Centre: play/pause 40 square, glyph 24 (PlayFilled / PauseFilled); previous/next 32 square, glyph 22, 8 dip from the play button. Next disabled at the end of a non-wrapping list.
  - Left: volume button (SpeakerMute/0/1/2 Filled by level: 0, <0.34, <0.67, else) toggles mute (restores the last audible level, or 1.0); its secondary press opens the output flyout (acrylic, above the button, ≤ 0.618 of the artwork column wide): 驱动 (WASAPI（共享） / ASIO when the build has it), 设备 (系统默认（name）, each endpoint or driver, with channels · kHz · DSD detail), DSD 以 DoP 输出 under ASIO; kept in `output.txt` (`system`, `wasapi`, `asio`, `dop`). The device list follows Windows: a device that comes or goes (or a new default) re-lists the open flyout. The device played on going (unplugged, a loose cable) pauses the song where it was; play opens it again on the device there is then (the chosen one when back, else the default). With no device at all, play stays paused and toasts "没有可用的音频设备". Gain track 6 tall, 48–96 wide when there is room. The volume is kept in `volume.txt` (`volume 0.350`, `unmuted 0.700`), written 1 s after it changes and at exit, and sent to the engine before the first song opens.
  - Right, from the edge inward, 4 dip apart: queue (TextBulletListFilled, lit when shown; only when a queue exists), lyrics (bubble + quote mark 20, only when the song has lyrics), play order (RepeatOne → ArrowRepeat1Filled, RepeatAll → ArrowRepeatAllFilled, Off → RepeatAll glyph at 0.45 ink, Shuffle → ArrowShuffleFilled). Small glyph size 18.
  - Order cycle: one → all → off → shuffle → one; with one song: one ↔ off. Default off.
- Previous: first press directly selects the previous song, regardless of elapsed time. At the head, wrap to last if the order wraps (all/shuffle, >1 song); otherwise leave the current song unchanged. No restart-first/double-press behavior.
- Next: next song; after the last, first if the order wraps; else disabled.
- A song changed to by the user (next, previous, a row pressed) is never crossfaded: the song playing fades out within 0.1 s (as far as the sound decoded ahead reaches) and stops, the progress shows the new one loading, and it starts plain from 0:00 once it is open. Pause, resume and seek take an 8 ms declick only (Windows and Linux).
- End of a song while the list plays: the next one per the order (the same one on repeat one; none at the end of a list that stops) follows **gaplessly**, its first sample right after the last one's (codec delay and padding trimmed): it is handed to the engine while the song plays, a plugin's song resolved 30 s before the end. The face, lyrics, queue position and title switch when its first sample is heard. A song that cannot follow in the same stream (not resolved or opened in time, DSD, a DSD output, another rate on ASIO) switches the ordinary way at the end. Seeking back over a boundary the sound has not reached yet stays in the song before it.
- Crossfade (淡入淡出, §8): the next song is handed over to overlap the end of the one playing by the setting's length, each fading under equal-power gains (each at its own ReplayGain; the effects and the volume act on the sum). The face switches when the overlap starts (the next song begins to sound): title, list, progress (from 0:00) and lyrics are the next song's; the words dissolve as on any change, the cover and the backdrop dissolve over the whole overlap in step with the sound (their mix is the coming song's share of it, standing while paused). A seek in the overlap ends it, and the dissolve with it. A next song of the same album (one file's CUE tracks, the song itself on repeat one, or the same album tag and album artist, the artist where none is given) follows gaplessly instead. A seek into the overlap's length before the end hands the next song over gaplessly (the song plays out); a seek back out of it hands it over overlapping again. A next song not open when the overlap would start (a slow network) is not faded into: the song playing plays to its own end, and the next starts plain from 0:00 once it is ready.
- Keys (not while a text field has focus, no modifiers): Space play/pause; ←/→ seek ∓5 s (clamped); ↑/↓ volume ±0.05; M mute; L lyrics; Q side panel (and Tab, while 键盘导航 is off); F/F11 fullscreen; Esc leaves fullscreen. With 键盘导航 on, Tab / Shift+Tab move the keyboard focus through the controls (NovaUI's keyboard navigation; the caption's minimise, maximise and close are no Tab stops, as Windows' own are not: Alt+Space, Alt+F4); a focused control takes the keys it uses first (Enter / Space press it), the rest stay these.
- Title bar back button (leading slot, caption-button look): closes the media, not the player. In the new product (no home page): stop + keep queue.

## 3 Lyrics (`views/side_panel/lyrics.rs`)

- LRC: `[mm:ss]`, `[mm:ss.x|xx|xxx]`, `:` as fraction separator; several stamps per line; `[offset:±ms]` (positive = earlier); BOM; unordered lines; other `[key:…]` headers skipped; a non-stamp bracket starts the words.
- Lines with the same stamp form one group (original + readings/translations), stable order. Empty stamped line = empty group (words stop).
- Layers (a plugin's `layers`: `roma` romanisation, `trans` translation) sit under the line of the same stamp (±50 ms), romanisation first. On the lyrics page a button per layer the song carries stands at the foot (lit while on); a press folds that layer away under its lines, or unfolds it from under them. Per line, three motions apart: the space opens (the lines below give way), the layer line rides the box's foot so it slides down out from under the line above, and its ink comes in once 30 % of the room is there (folding, the ink goes first, then the space closes). A spring drives each line (about 0.5 s, no bounce; a press mid-way turns it from where it is), and the lines start in a ripple from the one at the side panel's focus outward, 30 ms a line, at most 240 ms. The sung line stays where it is. Kept per layer for every song.
- Source order: container tag first (embedded lyrics), then the sidecar `<name>.lrc` beside a local file.
- `current(pos)` = last group with at ≤ pos; `until_next(pos)` arms the next wake.

## 4 Queue page (`views/side_panel/queue.rs`)

- Heading "播放列表" (or the saved playlist's name; opens the playlist menu), style 20/28 bold. Under it, Body TextSecondary: "第 {n} 首，共 {m} 首" (+ " · {note}"); narrowed: "找到 {n} 首" (search) / "{artist}，{n} 首" / "{album}，{n} 首".
- Search box right of the heading, geometry declared by the size rules in `player.nui`, hint "搜索", IME-capable. **Not native WinUI TextBox chrome.** Verified against `b7b33f5` `sound_queue.rs::search_box`: no border/stroke/accent underline in any state; unfocused (including hover) draws two rounded fills in order—white alpha `0x2B`, then RGB `0x80` alpha `0x24` (`sound_controls.rs::bar_material`); focused draws no background plate. Keep text, caret, selection and IME composition underline. Generic input tokens or a focused dark fill must not replace this product-specific design.
- Query (`app/query`, the same rule as a dropdown's box): every whitespace word must be found in the title, artist or album, one of three ways, best first:
  - **as text**, both folded: case, marks (`beyonce` finds Beyoncé, `strasse` Straße), full-width forms, Cyrillic and Greek in Latin letters (`kino` finds Кино), katakana as hiragana;
  - **by reading**: the starts of the readings of consecutive pieces: an ideograph by its pinyin (every reading of a polyphone; `ü` as `v` or `u`), Cantonese jyutping or Japanese readings; kana by Hepburn or Kunrei romaji, long vowels optional; Hangul by Revised Romanization, the older k / t / p / ch spellings and common surname spellings, or its first consonant's jamo; a word of letters whole or by its initial. So `zjl`, `zhoujl`, `zhoujielun` find 周杰伦, `tokyo` finds 東京 and とうきょう, `ㅎㄱ` finds 한국, `tswift` finds Taylor Swift. Kana typed is read as romaji. Thai, Arabic, Hebrew and Devanagari are found as text only;
  - **with a typo**: words of 4–7 characters within one edit of the name or of its reading, longer ones within two (a swap of two neighbours is one edit); shorter words never.
- Found rows are ranked by the least well found word (text, then reading, then typo), each rank in play order. The search runs off the UI thread; the rows shown, and "找到 {n} 首", stay until its ranking lands. While a search shows, the rows have no grip (they are not in play order).
- Heading area 72; rows 64: cover thumb 48 (radius 4, or wash 0x0F + MusicNote 20), gap 16, title 18/24 (bold if playing), second line "artist · album" Body TextSecondary; each part clickable (hover: primary ink + 1 px underline) → narrows to that artist/album (second-level page, back via side panel back button).
- Right end: grip 32 (ReOrderDotsVertical 16, TextSecondary, hover 0x0F / down 0x0A) 8 from the edge; left of it the duration (Body), or on hover of a non-playing row a remove button 32 (Dismiss 12).
- Heading: the list name; the search box, then two face control buttons (32, glyph 16): 定位 (Target: eases to the playing song) and the WinUI List button, whose menu sorts the queue by 添加顺序 (default) / 歌曲名称 / 歌手名 / 专辑名 / 时长 (stable, case-insensitive; the order in force is checked; sorting ends a shuffle and keeps the current song).
- Routes: every way back to the whole queue (back button/key, the other first-level page, the info page, a cleared search, the side panel opening) shows it at the playing song, and a route that drops the search empties the box. A new search or narrowing snaps to its top and its rows rise 24 dips into place over the turn (0.3 s). Rows giving way to a carried row ease to their new place over 0.25 s (cubic-bezier 0.1, 0.9, 0.2, 1).
- Row fills: playing 0x1F, hover 0x0F; plate extends 8 left, inset 2 vertically, radius 4. Rule under each row from the text start: white α 0x14, 1 px. Lower edge fade 64.
- Click row → play it (not the current one). Right-click → menu (MenuFlyout): "置顶" (ArrowUpload; disabled for the first), "添加到" (Add) and "移动到" (ArrowExport; disabled for the playing song), each with a chevron, "删除" (Delete; disabled for the playing song), rule, "歌曲信息" (MusicInfoFilled). Menu rows 32, min width 96.
  - 添加到 / 移动到 open a picker beside the row (pressed again, it closes; Esc closes it first): "新建播放列表" (a new list named by default), rule, every kept list but the shown one ("{n} 首" at right). The song is appended to the picked list (its play order too); 移动到 also removes it from the shown list. Toast "已加入「name」" / "已移到「name」".
  - A list made from a link (a plugin's playlist URL) takes such edits locally only: when it is read again from its service (first open in a session, 重新扫描) the service's songs replace it — songs added here leave, songs removed here come back; only the playing song stays.
  - 歌曲信息 opens the info page (§6) for that row's song.
- Drag by grip: row lifts (Flyout shadow, fill 0x1F), others make room (exponential follow rate 12/s), autoscroll within 48 of the ends at 12 rows/s; drop → move.
- Scroll: wheel. Everything that scrolls (the queue, the lyrics, the info page, the settings page, the import list, the flyouts) moves one way, NovaUI's `Glide` on WinUI's offset change: 5 ms per dip travelled, 50 ms to 1 s, decelerating, a new end mid-way starting from where it stands; quick notches the same way go up to three notches' worth; a dragged thumb carries the content at once. The queue: scrollbar rail at window edge inset 6: thin 2, expanded 12 (thumb 6, min 24) after 0.4 s hover, collapses after 2 s.
- Follows the playing song on song change; brings it into view when the list opens. Reorder (shuffle toggles) dissolves old → new in place.
- Rows start as file name (or CUE / saved tags) and are filled from tags lazily for rows in view + read-ahead.

## 5 Playlists (`library/lists.rs`, `library/store.rs`)

- Location `playlists\` in the data folder (§9): `index.txt` (magic `NVPLAYLISTS 1`, `active\t<id>`, `list\t<id>\t<count>\t<kind -|folder|file>\t<path>\t<name>` per line) + `p<id>.txt` per list (magic `NVPLAYLIST 1`; `name`, `origin kind path`, `shuffled 0|1`, `current`, `position ms`, `order …`, `own …`, then `item\tpath\tspan_start_ns\tspan_end_ns\ttitle\t<tags: title artist album album_artist track genre date duration_ms>`). Control chars → space. Atomic write (temp + rename) on a writer thread, at most every 2 s while changing, on close and at exit.
- Origin: Made / Folder(path) / File(.m3u/.m3u8/.cue). Opening the same origin again refreshes instead of duplicating (case-insensitive, trailing-separator-insensitive). Name = folder name / file stem; default "播放列表".
- Kept per list: items (found order), `order` (play order, shuffle's), `own` (user arrangement), current, position, shuffled. On startup the active list is restored **paused at its position**.
- Heading menu (MenuFlyout 320 wide, rows 36): kept playlists (checkmark on the open one, "{n} 首" at right; hover shows a delete ✕ 28 → first press arms "删除？", second deletes), empty state "还没有保存的播放列表"; then "新建播放列表" (Add; opens rename box for it), "重命名" (Rename; name box hint "播放列表名称"), "添加歌曲…" / "添加文件夹…" (ArrowImport; like the empty queue's buttons they add to the open list and never make a playlist), "导入播放列表…" (ArrowImport), "导出为 M3U8…" (ArrowExport), "重新扫描" (ArrowClockwise; only for folder/file origins: new files join, missing leave). Heading shows name + ChevronDown 12 (gap 8), Subtle hover fill.
- M3U/M3U8 import: `#EXTINF:len attrs,Name`, relative paths resolved against the list's folder, max 64 MB, skipped count reported as the queue note. Export writes relative paths with `#EXTINF`.

## 6 Song info page (`views/side_panel/info.rs`)

- Title "歌曲信息". Three groups, every row always present, missing = "-":
  1. 标题, 艺术家, 专辑, 专辑艺术家, # (`3/12` → `3 / 12`), 年份 (first standalone 4-digit run of the date), 流派, 时长 (clock).
  2. 编解码器 (`pcm_*` → PCM, vorbis → Vorbis, opus → Opus, wavpack → WavPack, mp3/mp3float → MP3, else uppercase), 采样率 / 位深 (`44.1 kHz / 16 bit`; whole kHz without decimal), 比特率 (`round(bps/1000) kbps`).
  3. 类型 (`EXT（Container）` when they differ: mov→MPEG-4, matroska→Matroska, …), 大小 (1024-based, one decimal, "N 字节" below 1 KB), 文件位置.
- Opened by the info button over the scrubber's right end (MusicInfoFilled), describing the playing song (followed as it changes); or by a queue row's 歌曲信息, describing that song: the list's words at once, then a local file's own (format, size, …) once read off the UI thread; a link's song shows the stream rows as "-". The info button then brings the playing song's back. Page turns in over the side panel; back button returns.

## 7 Formats, scanning, CUE (`library/mod.rs`)

- Sound extensions (music scope): mp3, flac, m4a, aac, ogg, opus, wav (+ ape, wv, tta, tak via CUE). What a file is comes from its container, not its extension.
- Folder scan: recursive, sorted by name case-insensitively (then exact); skips `$…`, `@…`, `#…`, `.…`, "System Volume Information".
- CUE: `INDEX 01 mm:ss:ff` (1/75 s), per-track TITLE/PERFORMER, album's; pregap belongs to the previous track; multi-FILE sheets; legacy code pages; missing file → same stem with another audio extension, or the sheet's stem; embedded FLAC `CUESHEET` when a FLAC is opened directly. Sheet ≤ 1 MB. A CUE beside audio in a folder replaces the file's row by its tracks. Tracks play as spans of one decoder, gapless into the next span.
- Import notes: "{n} 首找不到，已跳过".

## 8 Window, chrome, tokens

- Window 1280×720 dip at start, title "NovaPlayer", dark only. Custom title bar 32 tall; caption buttons 46 wide (minimise, maximise/restore, close; glyph 10); hover white α 15, pressed α 10 (glyph α 197); close hover = WinUI red. Leading slot (46×32) holds the back button while media is open. Drag-and-drop of files/folders/lists opens them. Fullscreen: title bar hidden.
- Scale: §0.2. The face grows with the window by the scale law up to the design box; beyond it the room around the face grows.
- WinUI dark tokens used: TextPrimary #FFFFFFFF, TextSecondary #C5FFFFFF, TextTertiary #87FFFFFF, TextDisabled #5DFFFFFF; ControlFillDefault #0FFFFFFF; radius control 4, overlay 8; spacing xs 4, s 8, m 12, l 16, xl 20, xxl 24, xxxl 32; control height 32. Type ramp: Caption 12/16, Body 14/20, BodyStrong 14/20 600, BodyLarge 18/24, Subtitle 20/28 600, Title 28/36 600, TitleLarge 40/52 600. Font: the bundled Noto Sans (SC / JP / KR for CJK; Arabic, Hebrew, Devanagari, Thai for those scripts) unless the user picks another (§8.1).
- Song change: field + cover crossfade linear ~267 ms; at most 3 layers. Under a song crossfade (§2) they follow the sound's overlap instead.
- Menus (MenuFlyout): acrylic plate radius 8, 1 device-px stroke, Flyout shadow, padding 4, icon 16 at 11 from the edge, label at 39.

## 8.1 Settings page (`views/settings_page.rs`)

- **Entry:** the app menu's last row 设置 (☰, top-left). The page takes the face's place on the same backdrop (no background of its own); face and page crossfade 0.25 s. While it is up, ☰ is a back arrow.
- **Routes:** the back arrow and Esc return to playback from any depth (Esc in a text field first leaves the field). The mouse's back button goes up one level; from the first level, to playback. No history.
- **Title:** a breadcrumb `设置 › 插件 › <name>`; an earlier segment opens its level, the last is where the user is. When it does not fit, the earliest segments give way to `…`, which goes up one level. Every segment has the title's size and weight (an earlier one grey, the last white); the chevrons between them centre on the capitals' middle, as macOS centres a symbol in text (WinUI centres its breadcrumb chevron on the line, which in Segoe UI is nearly the same place). A new level fades in at its top.
- **Look:** WinUI's settings layout: section headings, one card per line (icon, title over description, control at the right). Cards are flat acrylic without outline; a white wash means hover / press / lit / current only. A card that opens a deeper level is a button with a chevron. A switch (WinUI's ToggleSwitch in the face's inks: off, the search box's plate without a border and a grey knob; on, the bars' white fill and a black knob) flips with its motion: the knob slides to the other end (367 ms) while the track and the knob cross-fade to the other look (83 ms); the knob is 12 at rest, 14 under the pointer, 17 × 14 pressed.
- **外观:**
  - 字体: a box that takes typing (Word's font box, WinUI's editable ComboBox). The list: 默认（Noto Sans）, then every installed family by its Chinese name, else its English one. Focus selects the name; typing narrows the list to the families whose names (any language) hold every typed word, any case, the first lit; Up / Down step, Enter picks, the chevron opens the whole list, Esc gives back the kept name. No match: 没有匹配项. The chosen family applies at once to every text.
  - 布局: 自动 (two columns once the window is wide enough) / 单列 (one column at every width: the side panel comes over the face, never beside it). In one column, for whatever reason (单列, or a narrow window), the lyrics and the song details are centred; beside the face they start at its edge.
  - 歌词样式: 默认 (the built-in look) or an installed style package (`extensions.md` §C), by name; 安装… takes a package (zip / tar.gz) or a folder's `style.json`, checks it, installs and chooses it; 删除 (shown while an installed style is chosen) deletes it and goes back to 默认. A style changes how lines are drawn, never where: size, wrap, scroll and the controls stay the player's.
  - 语言: 跟随系统 / 简体中文 / English (each named in itself). 跟随系统: Chinese when the system's display language is Chinese, else English. The switch is live: every text of the player and of its plugins speaks the new language at once; a toast already shown keeps its words.
  - 键盘导航: 关 (default) / 开, at once. On, Tab / Shift+Tab move the keyboard focus and a focused control takes Enter, Space and the arrows (Keys, above); off, the focus uses no key and Tab shows or hides the side panel.
  - 屏幕阅读器: 关 (default) / 开. On, screen readers (Narrator, NVDA; Orca) read and operate the window (NovaUI's accessibility, UI Automation / AT-SPI); it takes effect at the next start (the window is made with it). Off, no adapter is made. The controls drawn as icons are named (`a11y.*`), the seek and volume bars are sliders (seconds; percent; 5 a step), the queue a list of its rows, the song's title a live region announced when it changes.
- **播放:**
  - 回放增益: 关 (default) / 按单曲 / 按专辑. Songs play at the loudness their tags state (`REPLAYGAIN_TRACK_GAIN` / `REPLAYGAIN_ALBUM_GAIN`, else Opus' `R128_*`); 按单曲 takes the song's value, else its album's; 按专辑 the album's, else the song's; an untagged song plays as it is. A change applies to the song playing without a step (Windows: a ring's length later; Linux: the sound server's buffer later, about 0.1 s). Above 0 dB a peak limiter holds the sound under full scale. The kept value is sent to the engine before the first song opens.
  - 淡入淡出: 关 (default) / 1 秒 … 12 秒: how long one song overlaps the next (§2, Crossfade). It applies from the next song handed over: at once to the one playing.
  - 音效: a card (how many effects) opens `设置 › 音效`: the CLAP effects the sound plays through (`plugins.md` "Audio effects (CLAP)"), in order, after ReplayGain and before the volume. One card per effect (name, vendor and version; or why it was taken out): 打开 (its own window, floating above the player; one that only embeds opens in a window of the player's), 上移 / 下移, 移除, 参数 (for an effect with parameters), and 开 / 关 (its switch; 关: bypassed, still on time). 参数 opens `设置 › 音效 › <name>`: the effect's card (打开, 开 / 关), then under 参数 a card per parameter (grouped under its module's heading), its value as the effect prints it: a slider (the face's volume bar: a pill filled white up to the value, no thumb); a switch for one of two values, a dropdown for one of a few; none for one only the effect sets. A value set there is heard within a quarter second (what is already buffered) and shows in the effect's window; one turned in its window shows on the page. Below, 添加音效: the effects found (`CLAP_PATH`, then the system's CLAP folders), each with 添加, and 重新查找. A change to the chain is heard from the next seek or song. Opening the page builds the effects' windows in the background, so 打开 shows one at once; it never waits for the plugin. An effect that fails (an error, a NaN or infinity, a rate it cannot run at) is taken out for the session and its card says why; the song goes on without it.
- **插件:** a card `插件` (installed and enabled counts) opens `设置 › 插件`: one card per plugin (its state; the account when connected) opening its page, then 安装插件 (打开目录 / 安装…). A plugin's page: name, version (when its manifest gives one; the in-tree plugins give none until published), state and its switch (the first press shows what it asks for and 再点一次以启用; the second enables; off disables); 连接 (the form, a QR code while connecting, the account and 退出 when connected); 导入歌单…; a control that could not be read; 删除插件. The app menu's plugin rows and their panel stay as the quick way in; both show the same thing.
- **关于:** the version and the commit this build was made from ("development build" when built outside a checkout), Player-Core revision, licences: FFmpeg LGPL notice, TAK decoder terms (Windows), ASIO and the GPL-3.0 it puts the build under (Windows with `asio`), Noto Sans OFL.
- **Kept:** `settings.txt` in the data dir (`key value` lines; unknown keys kept): `font` (family, empty: default), `layout` (`auto` / `single`), `lyric-style` (an installed style's id; empty: the built-in one), `locale` (`zh-CN` / `en`; empty: the system's), `replay-gain` (`off` / `track` / `album`; absent: off), `crossfade` (`off` / `1` … `12` seconds; absent: off), `keyboard-navigation` (`off` / `on`; absent: off), `screen-reader` (`off` / `on`; absent: off: whether screen readers reach the window, read once at start).

## 9 Persistence (`main.rs` `DATA_DIR`, `library/store.rs`, `app/settings.rs`)

The data folder is `NovaPlayer` under `%LOCALAPPDATA%` (Windows), else `$XDG_DATA_HOME`,
else `~/.local/share`.

| Path | Holds | Written |
|---|---|---|
| `playlists\` | `index.txt` and `p<id>.txt` (§5) | writer thread, temp + rename |
| `row-cache-v2\` | rows' tags and cover thumbnails by source, in their folder's group, a thumbnail kept once by its content; a hit never touches the media path; local records expire after 7 days; rescan and cover edits invalidate | worker, temp + rename |
| `chosen-covers\` | covers the user picked for a song (never written beside the media) | temp + rename |
| `settings.txt` | `font`, `layout`, `lyric-style`, `locale`, `replay-gain` (§8.1); unknown keys kept | on change, writer thread |
| `lyrics-layers.txt` | which lyric layers are folded (§3) | on change, writer thread |
| `volume.txt` | `volume`, `unmuted` (§2) | 1 s after it changes, and at exit; writer thread |
| `output.txt` | driver, device, DoP (§2) | on change, writer thread |
| `options.txt` | plugin control values | on change, writer thread |
| `effects.txt`, `effects\<n>.bin` | the effect chain (`on`/`off`, plugin id, file, a line each in order) and each effect's saved state (§8.1) | 2 s after it changes, and at exit; writer thread |
| `clap-scan.txt` | what each `.clap` file holds, by path, size and time of change | after a scan |
| `plugins\`, `plugins\enabled.txt` | installed plugins; enabled ids with their fingerprint (`plugins.md`) | on install / enable |
| `lyric-styles\` | installed lyric style packages (§8.1) | on install |

Credentials are not in the folder: the vault keeps them in the system's credential store, opaque
to the player (`plugins/accounts.rs`). Tests use a temporary data root and an in-memory vault.
