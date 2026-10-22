# Verification

How every change is checked: one command per level, every check in git, no machine paths.

## Levels

| Level | What | Where it runs | Where it lives |
|---|---|---|---|
| L0 | unit tests: pure logic, crate internals, no window, no device | every OS, CI | beside the code in each repository; `cargo test --release --workspace` |
| L1 | novaui rendering: the software backend's frames against a from-scratch repaint; GPU backends against the software reference where a device exists | every OS, CI (software) | maintained separately |
| L2 | plugin protocol: manifests, the pipeline, calls, controls (the demo plugin, `examples/demo_plugin.rs`) | every OS, CI | unit tests here (part of L0's command) |
| L3 | scenarios: the real app on the software backend, scripted input, null audio, generated media, a temp profile | Windows, CI | maintained separately |
| L4 | desktop: screenshots against the baseline, perf targets on real windows | a Windows desktop | maintained separately |
| L5 | manual: a person's devices, accounts or recordings | where they are | each `#[ignore = "L5 manual: …"]` says what it needs |

Integration tests, end-to-end scenarios and screenshot baselines are maintained separately. They
reach the app through `novaplayer`'s `test-support` feature (the modules they drive made public,
and the app's test seams: an in-memory account store, fixed About revisions); Player-Core's
`test-support` adds its null output and the hidden `internals` its pump and CLAP tests use.

CI (`.github/workflows/ci.yml` in each repository) runs fmt, clippy and the unit tests on
every push, on a Windows and a Linux runner. A change is done when CI passes and, for a visible
change, its screenshots have been looked at.

## From a fresh clone

```sh
git clone https://github.com/wuuuming/NovaPlayer.git && cd NovaPlayer
python tools/ci.py          # fmt, clippy (-D warnings), unit tests
```

Nothing to set up first beyond Rust (and on Windows LLVM, for the ASIO bindings). FFmpeg on
Windows comes with Player-Core (its audio-only build, in the source tree); the build scripts
fetch the Noto fonts (this crate's, from Google Fonts' and Noto's repositories at pinned
commits, each checked by sha256) into `target/`.

- **Dependencies**: NovaUI and Player-Core are git dependencies pinned by `rev`; a fresh clone and
  CI use the pinned commits (CONTRIBUTING.md shows the `[patch]` for working on sibling
  checkouts). After pushing a NovaUI or Player-Core commit, `python tools/bump.py novaui` (or
  `player-core`) pins it and refreshes `Cargo.lock` as a fresh clone resolves it; commit both
  with the change that needs them. CI builds `--locked`.
- **Contracts**: novaui's phase rules and damage audit (`novaui/docs/runtime.md` "What each
  phase may do") are on in debug builds; `NOVAUI_AUDIT=1` turns them on in release, and CI's
  test steps set it. A broken contract panics with what broke.
- **Data folders**: no test reads or writes the user's; each uses a temp one.

## The portable packages and releases

`python tools/portable.py` builds the package of the platform it runs on, from the committed
state (a dirty tree, or a sibling NovaUI / Player-Core checkout that is not the pinned commit,
stops it; `--dirty` overrides and marks the package), and writes it with its `.sha256` into
`target/dist-portable/`:

- **Windows**, `NovaPlayer-Windows-x64-<commit>.zip`: `novaplayer` with ASIO, the FFmpeg DLLs, the
  TAK decoder, the VC++ runtime, the fonts and the licences, `Launch.cmd` (data in `.\Data`),
  `README.txt` (start, reporting a bug, licence) and `BUILD-INFO.json` (the three commits,
  FFmpeg's build, every file's sha256). Give the FFmpeg source companion
  (`NovaPlayer-FFmpeg-source-<rev>.zip`: FFmpeg's source at the commit and Player-Core's
  tools/ffmpeg-audio scripts that build it, made by the release workflow) out with it.
- **Linux**, `NovaPlayer-Linux-x64-<commit>.zip`: `novaplayer` without ASIO, the fonts and the
  licences, `NovaPlayer.sh` (data in `./Data`), `README.txt` and `BUILD-INFO.json`; FFmpeg is the
  system's. Built on the oldest glibc it should run on.

Either starts the packaged program once with a throwaway data folder and only its own libraries
(`--sw`: on the software renderer, for a machine without a GPU; on Linux under an X server). The
plugins are packaged on their own: `python tools/plugins.py` writes
`target/dist-plugins/NovaPlayer-plugin-<id>-<Windows|Linux>-x64.zip`.

`.github/workflows/release.yml` builds both packages (Windows on `windows-2025`, Linux on
`ubuntu-22.04` under `xvfb-run`), the plugins for both and the FFmpeg source companion. It runs
by itself when CI passes on a push to `main` and the version in `crates/novaplayer/Cargo.toml`
has no release yet: it publishes them as release `v<version>` (the tag made on that commit), the
latest. A release is a version bump pushed to `main`. Started by hand it only keeps them as the
run's artifacts, to be tried.

## CI runners

The three repositories run CI on GitHub Actions' hosted runners, named exactly (`windows-2025`,
`ubuntu-24.04`, `ubuntu-22.04` for the Linux package) so an image change is a deliberate edit;
Rust is pinned by each repository's `rust-toolchain.toml`. While NovaUI and Player-Core are
private, NovaPlayer's workflows reach them with the secret `NOVA_READ_TOKEN` (Contents: read).

## Linux

On a Wayland desktop NovaUI uses Wayland (`platform/wayland.rs`); X11 windows
(`platform/x11.rs`) are shown through XWayland where asked for.

Network audio (L5: system `ffmpeg`/`ffprobe` and a PulseAudio / PipeWire server). It serves a
generated WAV over loopback HTTP and checks URL queries, duration, play/pause/seek/EOF, and that
HTTP 404 becomes an error rather than queue advancement. An isolated null sink keeps the sound
off the speakers:

```sh
profile=$(mktemp -d)
sink=np_http_test_$$
module=$(pactl load-module module-null-sink sink_name="$sink")
trap 'pactl unload-module "$module"; rm -rf "$profile"' EXIT
LOCALAPPDATA="$profile" XDG_DATA_HOME="$profile" PULSE_SINK="$sink" \
  cargo test --release -p novaplayer --lib \
  playback::engine::http_tests::http_playback_pause_seek_and_failure \
  -- --ignored --exact --nocapture --test-threads=1
```

Real-display end-to-end via XWayland (L5; inherit the KDE session's display and
authentication file — `DISPLAY` can change after login, and assuming `:0` may reject the cookie):

```sh
desktop_pid=$(pgrep -u "$(id -u)" -x plasmashell | head -1)
while IFS='=' read -r key value; do
  export "$key=$value"
done < <(tr '\000' '\n' < "/proc/$desktop_pid/environ" | grep -E '^(DISPLAY|XAUTHORITY|XDG_RUNTIME_DIR)=')
cd novaui
cargo build --release -p novaui --example gallery --features backend
python3 crates/novaui/examples/gallery_e2e.py target/release/examples/gallery /tmp/e2e vk   # and `sw`
```

The gallery settles, writes the software reference of its exact tree state, and the script
captures the real window from the X server and compares (2/255, at most 16 px at 3). Phase 1
repeats after a synthetic click sent to the gallery's own window (the pointer is not moved).
Keep screen blanking (DPMS) off while it runs: with the monitor off the server delivers no
frames.

Vulkan validation (L5, after barrier or layout changes; no sudo needed). The layer reports
nothing visible without a settings file (novaui installs no debug messenger):

```sh
apt-get download vulkan-validationlayers && dpkg -x vulkan-validationlayers_*.deb vvl
export VK_LAYER_PATH=$PWD/vvl/usr/share/vulkan/explicit_layer.d
export LD_LIBRARY_PATH=$(dirname "$(find "$PWD/vvl" -name libVkLayer_khronos_validation.so)")
export VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation VK_LAYER_SETTINGS_PATH=$PWD/vvlcfg
mkdir -p vvlcfg && cat > vvlcfg/vk_layer_settings.txt <<'END'
khronos_validation.report_flags = error,warn
khronos_validation.debug_action = VK_DBG_LAYER_ACTION_LOG_MSG
khronos_validation.log_filename = /tmp/vvl.log
khronos_validation.enables = VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT
END
NOVAUI_AUDIT=1 cargo test --release -p novaui   # then: /tmp/vvl.log must be empty
```

Native Wayland: NovaUI opens a Wayland window when `WAYLAND_DISPLAY` is set (`NOVAUI_X11=1`
forces XWayland, which `gallery_e2e.py` needs for its X capture). A shot of the compositor's
view over ssh: `QT_QPA_PLATFORM=wayland spectacle -b -n -f -o shot.png` (with the session's
`WAYLAND_DISPLAY` and `DBUS_SESSION_BUS_ADDRESS` too; without the platform it writes nothing).

GPU backends: `cargo test --release -p novaui` runs `vulkan_matches_reference`, `image_sharing`
and `layer_storage` against every GPU backend the machine has (D3D11 and Vulkan on Windows,
Vulkan on Linux). OBS's implicit Vulkan capture layer crashes test processes on exit on the dev
machine; the tests set `DISABLE_VULKAN_OBS_CAPTURE=1`.
