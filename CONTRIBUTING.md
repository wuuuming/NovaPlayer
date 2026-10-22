# Contributing

NovaPlayer is three repositories that change together:

| Repository | What it holds |
|---|---|
| NovaPlayer (this one) | the app, the plugin protocol and plugins; product docs |
| NovaUI | the UI framework (`novaui`) and the icon set (`nova-icons`) |
| Player-Core | the playback core (Windows: FFmpeg, WASAPI / ASIO; Linux: FFmpeg, PulseAudio) |

Building is `cargo build --release` (README). How things are checked:
[docs/verification.md](docs/verification.md).

To change NovaUI or Player-Core together with the app, clone the three repositories side by side
and point Cargo at them from a `.cargo/config.toml` in the folder holding them (nothing
machine-specific is committed):

```toml
[patch."https://github.com/wuuuming/NovaUI.git"]
novaui = { path = "novaui/crates/novaui" }
nova-icons = { path = "novaui/crates/nova-icons" }
novaui-shaders = { path = "novaui/crates/novaui-shaders" }

[patch."https://github.com/wuuuming/Player-Core.git"]
player-core = { path = "player-core/crates/player-core" }
media-io = { path = "player-core/crates/media-io" }
```

## Where a change belongs

Decide the layer first ([docs/extensions.md](docs/extensions.md) §1): what another product could
use goes into NovaUI; what another service could use goes into the app; only what a single
service needs is a plugin. The app names no service (no NetEase, no QQ Music in its code).

The design rules the code follows are in [docs/principles.md](docs/principles.md) (design
ownership, runtime and performance, robustness, names).

What already exists and where is in each repository's map: Player-Core's
[`docs/map.md`](https://github.com/wuuuming/Player-Core/blob/main/docs/map.md), NovaUI's
[`docs/map.md`](https://github.com/wuuuming/NovaUI/blob/main/docs/map.md) and NovaPlayer's
[`docs/map.md`](docs/map.md). Before writing a mechanism, find the one in the map
that can carry the change (D2). A change that adds, moves or removes a module or mechanism
updates the map in the same commit.

## Every change proves itself

- Build, `cargo fmt --check`, `cargo clippy --release --workspace --all-targets` and
  `cargo test --release` (the unit tests) in every repository you touched (`python tools/ci.py`).
- A bug fix brings a regression test that fails before the fix and passes after, at the lowest
  level that can see it, usually a unit test beside the code in the same commit.
- A UI change comes with a screenshot of the result.
- A performance change states its before/after numbers in the commit message.
- Tests never read or write the user's real data directory; they use a temporary one.

## One change, one repository order

NovaUI first: test, commit, push. Then `python tools/bump.py novaui` points NovaPlayer at that
commit, committed together with the NovaPlayer change that needs it. Player-Core the same way
(`python tools/bump.py player-core`). A pin bump is never a commit of its own, so every
repository builds from a fresh clone without a local `[patch]`.

## Specs

The documents in `docs/` describe the current code only. A change that alters behaviour updates
the spec in the same commit; larger work goes in one pull request per coherent step.

## Style

`cargo fmt` (each repository's `rustfmt.toml`); the workspace lints (`[workspace.lints]`) are
part of clippy's run. Every `unsafe` block states its contract in a `// SAFETY:` comment.

## Licence

Contributions are licensed as the repositories are: MIT OR Apache-2.0, at the user's option,
without additional terms.
