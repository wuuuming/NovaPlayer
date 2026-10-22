<h1 align="center">NovaPlayer</h1>

<p align="center">一个用 Rust 写的 Windows / Linux 音乐播放器。</p>

<p align="center">
  <a href="https://github.com/wuuuming/NovaPlayer/releases/latest"><img src="https://img.shields.io/github/v/release/wuuuming/NovaPlayer?include_prereleases&label=release" alt="最新版本"></a>
  <a href="#许可证"><img src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue" alt="许可证"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux-lightgrey" alt="平台">
</p>

<p align="center"><a href="README.md">English</a> | 简体中文</p>

<p align="center"><img src="docs/images/screenshot.png" alt="NovaPlayer 正在播放，歌词在封面旁滚动" width="800"></p>

NovaPlayer 播放你手里已有的音乐文件，在封面旁边同步显示歌词。主要就是这些。不用导入媒体库，不用
注册账号，也没有常驻后台的服务。

想做的是这样一个播放器：点开就有，画面不动的时候不占 CPU，接 ASIO 声卡也能放得对，在 Windows 11
上看着不突兀，而且底下没有塞一个浏览器内核。窗口由 [NovaUI](https://github.com/wuuuming/NovaUI)
绘制，声音走 [Player-Core](https://github.com/wuuuming/Player-Core)，这两个都是为它写的。

这是第一个公开版本（0.0.1），难免有不顺手的地方，碰到了欢迎
[提 issue](https://github.com/wuuuming/NovaPlayer/issues)。

## 能做什么

- 播放 FLAC、MP3、AAC / M4A、Ogg Vorbis、Opus 和 WAV；APE、WavPack、TTA、TAK 通过 CUE 播放（整轨
  也行）。把文件或文件夹拖进窗口，或者打开 M3U、CUE。
- 专辑无缝播放。想要淡入淡出（1 到 12 秒）可以在设置里打开；手动切到的歌不会淡入淡出。
- 歌词来自文件标签或同目录的 `.lrc`，跟着歌曲滚动。点一行就跳到那里。
- Windows 上用 WASAPI 或 ASIO 输出（ASIO 下 DSD 可原生或 DoP），Linux 上用 PulseAudio 或 PipeWire。
  支持回放增益（按单曲或按专辑）和 CLAP 音效插件。
- 记住你的播放列表和队列，下次打开停在上次的位置（暂停着）。
- 在线音源交给插件。每个插件是单独的进程，可以提供登录、导入歌单、歌词翻译和额外的按钮；播放器本身
  不认识任何具体的服务。怎么写插件见 [docs/plugins.md](docs/plugins.md)（英文）。
- 界面有简体中文和英文。键盘导航和屏幕阅读器（讲述人、NVDA、Orca）可以在设置里打开。

还没有的：按专辑、艺术家浏览媒体库（目前只有播放列表和队列）、编辑标签、媒体键和系统的媒体控制、
迷你播放器，以及 macOS 版。想要哪一个，欢迎提 issue 或拉取请求。

## 下载

在 [Releases](https://github.com/wuuuming/NovaPlayer/releases/latest) 下载对应系统的 zip，解压到
哪里都行，直接运行，不需要安装。

- **Windows 10 / 11**（x64）：运行 `Launch.cmd`。FFmpeg 和字体都已带上。
- **Linux**（x86-64，glibc 2.35 以上，比如 Ubuntu 22.04、Debian 12）：运行 `./NovaPlayer.sh`。
  需要系统里有 `ffmpeg` 和 `ffprobe`、PulseAudio 或 PipeWire，以及 Vulkan 驱动（没有的话加
  `--backend sw` 启动）。

这样启动时，播放列表、设置和插件的登录信息都放在程序旁边的 `Data` 文件夹里，整个文件夹拷到 U 盘也能
用。直接运行 `novaplayer` 的话，则使用 `%LOCALAPPDATA%\NovaPlayer` 或 `~/.local/share/NovaPlayer`。

## 怎么用

用空队列里的按钮、直接拖进窗口，或者播放列表名字旁边的菜单添加音乐。设置在 ☰ 菜单最下面。右键歌曲
打开它的菜单，右键音量按钮选择输出设备。

| 按键 | |
|---|---|
| 空格 | 播放 / 暂停 |
| ← → | 快退 / 快进 5 秒 |
| ↑ ↓ | 音量 |
| M | 静音 |
| L | 歌词 |
| Q、Tab | 显示 / 隐藏侧栏 |
| F、F11 | 全屏 |

## 自己构建

需要 Rust（Windows 上还要 rustup 要求的 MSVC 构建工具，以及 ASIO 绑定要用的 LLVM）。然后：

```sh
git clone https://github.com/wuuuming/NovaPlayer.git
cd NovaPlayer
cargo build --release
```

第一次构建会自动下载仓库里没有的东西，每样都按校验和固定：FFmpeg（Windows）和 Noto 字体。程序在
`target/release/novaplayer`。Linux 上运行时需要 `ffmpeg`、`ffprobe`，以及 PulseAudio 或 PipeWire。

## 参与

现在最有用的是报告问题。想提交代码的话，请先看 [CONTRIBUTING.md](CONTRIBUTING.md)（英文），里面说明了
一个改动该放进三个仓库中的哪一个。应用的行为都写在 [规格](docs/novaplayer-spec.md) 里，
[地图](docs/map.md) 说明各部分在哪。

## 致谢

解码用的是 [FFmpeg](https://ffmpeg.org/)，ASIO 靠 [cpal](https://github.com/RustAudio/cpal)，布局靠
[Taffy](https://github.com/DioxusLabs/taffy)，文字排版靠 [HarfRust](https://github.com/harfbuzz/harfrust)，
屏幕阅读器靠 [AccessKit](https://github.com/AccessKit/accesskit)。图标是微软的
[Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons)，字体是
[Noto](https://fonts.google.com/noto)，控件的样子参照 [WinUI](https://github.com/microsoft/microsoft-ui-xaml)。

## 免责声明

NovaPlayer 与任何音乐服务都没有关系。插件带来的歌曲、歌词和封面归各自的权利人所有。请只在服务条款
允许的范围内使用插件，并通过正规渠道支持音乐人。

## 许可证

代码可在 [Apache-2.0](LICENSE-APACHE) 和 [MIT](LICENSE-MIT) 中任选其一使用。Windows 下载包里还有
FFmpeg（LGPL）、Noto 字体（OFL）、TAK 解码器（非商业用途免费）和 Steinberg 的 ASIO SDK（GPL-3.0），
所以这个 zip 整体按 GPL-3.0 分发。不带 ASIO 的构建（`--no-default-features`）保持上面的条款。ASIO 是
Steinberg Media Technologies GmbH 的商标。

中英文版本如有出入，以英文版为准。
