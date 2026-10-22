"""The portable package of this platform (Windows or Linux): one folder in a zip, run from
wherever it is extracted.

  python tools/portable.py            build, package, start-check; into target/dist-portable/
  python tools/portable.py --dirty    also from uncommitted changes (marked in BUILD-INFO.json)
  python tools/portable.py --sw       start-check on the software renderer (a machine without a
                                      GPU: CI; on Linux under an X server such as xvfb-run)

Builds `novaplayer` (release; on Windows with the default features: ASIO on, so the package is
GPL-3.0 as a whole, README "License"; on Linux without ASIO) from the committed state: the working tree must be clean, and a NovaUI or
Player-Core built from a sibling checkout through the parent folder's [patch] must be clean and at
the commit Cargo.toml pins, so BUILD-INFO.json names exactly what is inside. Then:

  NovaPlayer-Windows-x64-<commit>.zip       novaplayer.exe, FFmpeg (LGPL, replaceable DLLs), the
                                            TAK decoder, the VC++ runtime, the Noto fonts,
                                            licences, Launch.cmd (keeps all data in .\\Data),
                                            README.txt, BUILD-INFO.json
  NovaPlayer-Linux-x64-<commit>.zip         novaplayer, the Noto fonts, licences, NovaPlayer.sh
                                            (keeps all data in ./Data), README.txt,
                                            BUILD-INFO.json; FFmpeg is the system's
  *.sha256                                  beside it

Service plugins are packaged on their own: tools/plugins.py writes target/dist-plugins/.

The FFmpeg DLLs are the ones the build staged (the audio-only LGPL build Player-Core carries in
its source tree). The source the LGPL asks for is a separate companion,
NovaPlayer-FFmpeg-source-<rev>.zip, which the release workflow makes and publishes with the
package. The package is extracted and started once (temp data folder) before it is published.
"""
import ctypes, glob, hashlib, json, os, re, shutil, subprocess, sys, tempfile, time, zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import bump  # noqa: E402

WINDOWS = sys.platform == "win32"
# Cargo's target folder: CARGO_TARGET_DIR when the machine sets one.
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
DIST = os.path.join(TARGET, "dist-portable")
RELEASE = os.path.join(TARGET, "release")
# The VC++ runtime novaplayer.exe imports (the api-ms-win-crt-* rest is part of Windows 10+).
VCRUNTIME = ["vcruntime140.dll", "vcruntime140_1.dll"]

LAUNCH = r"""@echo off
setlocal
cd /d "%~dp0"
set "LOCALAPPDATA=%~dp0Data"
if not exist "%LOCALAPPDATA%" mkdir "%LOCALAPPDATA%"
start "" "%~dp0novaplayer.exe" %*
"""

MS_REDIST = """Distributable Code for Microsoft Visual Studio 2022 (Includes Utilities & BuildServer Files)

For the latest version of this Redist file, please visit https://aka.ms/vs/17/redist.txt.
"""

LAUNCH_SH = """#!/bin/sh
# Portable start: playlists, settings, plugin sign-ins and caches stay in ./Data
# (XDG_DATA_HOME) instead of ~/.local/share. Arguments go to novaplayer (files to open, or
# --backend vk|sw).
here=$(cd "$(dirname "$0")" && pwd)
export XDG_DATA_HOME="$here/Data"
mkdir -p "$XDG_DATA_HOME"
exec "$here/novaplayer" "$@"
"""

README_LINUX = """NovaPlayer {version} portable - Linux x86-64 (glibc {glibc} or newer)
Build {commit} ({date})

START
Extract the whole folder somewhere writable and run ./NovaPlayer.sh. Playlists, settings,
plugin sign-ins and caches then stay in Data/NovaPlayer beside it: keep Data when you update,
delete it to start over. Running ./novaplayer directly uses ~/.local/share/NovaPlayer instead.
Nothing is installed; no root is needed.

From the system it needs:
  - ffmpeg and ffprobe on the PATH (decoding; e.g. apt install ffmpeg)
  - PulseAudio or PipeWire (libpulse.so.0) for sound
  - Wayland or X11, and a Vulkan driver; without one: ./NovaPlayer.sh --backend sw (X11)
The Noto fonts are included. Service plugins come as separate packages.

REPORTING A BUG
Open an issue at https://github.com/wuuuming/NovaPlayer/issues with the build ({commit}, also in
BUILD-INFO.json), what you did, what you expected and what happened instead; your distribution,
desktop and sound server; Data/NovaPlayer/crash.log if NovaPlayer closed by itself. Do not
attach Data/NovaPlayer as a whole: it holds your plugin sign-ins.

LICENCE
NovaPlayer, NovaUI and Player-Core are Apache-2.0 or MIT (licenses/); the source of the commits
in BUILD-INFO.json is at https://github.com/wuuuming. The Noto fonts are under the SIL OFL 1.1
(fonts/OFL.txt). FFmpeg is not included: the system's own is used.
"""

README = """NovaPlayer {version} portable - Windows 10/11 x64
Build {commit} ({date})

START
Extract the whole folder somewhere writable (not Program Files), then double-click Launch.cmd.
Playlists, settings, plugin accounts, caches and logs then stay in Data\\NovaPlayer beside it:
keep Data when you update, delete it to start over. Starting novaplayer.exe directly uses your
normal %LOCALAPPDATA%\\NovaPlayer instead. Do not run it from inside the zip.

D3D11 draws by default; Vulkan needs the GPU vendor's driver; ASIO output needs the device's
ASIO driver. Fonts and the Visual C++ runtime are included; nothing is installed.
Service plugins come as separate zips: install one from the app's plugin menu if you want it.

REPORTING A BUG
Open an issue at https://github.com/wuuuming/NovaPlayer/issues with:
  - the build: {commit} (also in BUILD-INFO.json);
  - what you did, what you expected, what happened instead; a screenshot or a short recording
    if it is about how something looks or moves;
  - if NovaPlayer closed by itself: Data\\NovaPlayer\\crash.log (with Launch.cmd), or
    %LOCALAPPDATA%\\NovaPlayer\\crash.log (novaplayer.exe started directly);
  - your Windows version, display scaling, and the audio output (Settings) if it is about sound;
  - for a file that does not play: its format (or the file).
Do not attach Data\\NovaPlayer as a whole: it holds your plugin sign-ins.

LICENCE
NovaPlayer's source is Apache-2.0 or MIT. This build includes Steinberg's ASIO SDK (GPL-3.0),
so this package as a whole is distributed under the GPL-3.0 (licenses\\GPL-3.0.txt); the source
of the commits in BUILD-INFO.json is at https://github.com/wuuuming (NovaPlayer, NovaUI,
Player-Core).
FFmpeg is LGPL 2.1 or later, as separate replaceable DLLs; its matching source is the companion
NovaPlayer-FFmpeg-source-{ffmpeg}.zip. The Noto fonts are under the SIL OFL 1.1 (fonts\\OFL.txt).
The TAK decoder (tak_deco_lib.dll, unmodified) is free for non-commercial use only
(licenses\\TAK). The VC++ runtime DLLs are Microsoft's redistributable code.
"""


def run(cmd, **kw):
    print("==", " ".join(cmd), flush=True)
    r = subprocess.run(cmd, cwd=kw.pop("cwd", REPO), **kw)
    if r.returncode != 0:
        sys.exit(f"failed ({r.returncode}): {' '.join(cmd)}")
    return r


def git(cwd, *args):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8", check=True).stdout.strip()


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def pinned(name):
    """The commit the manifests pin for the repository `name` (`bump.pins`)."""
    rev = bump.pins().get(name)
    if not rev:
        sys.exit(f"the manifests pin no {name}")
    return rev


def dependencies(dirty):
    """{name: (commit, package dir)} for NovaUI and Player-Core as this build compiles them."""
    with bump.lock_kept():
        meta = json.loads(run(["cargo", "metadata", "--format-version", "1"], capture_output=True, text=True, encoding="utf-8").stdout)
    out = {}
    for name in ("novaui", "player-core"):
        pkg = next(p for p in meta["packages"] if p["name"] == name)
        pkg_dir = os.path.dirname(pkg["manifest_path"])
        source = pkg["source"] or ""
        if source.startswith("git+"):
            out[name] = (source.rsplit("#", 1)[1], pkg_dir)
            continue
        # Built from a sibling checkout through the parent folder's [patch].
        head = git(pkg_dir, "rev-parse", "HEAD")
        if head != pinned(name) or git(pkg_dir, "status", "--porcelain", "--untracked-files=no"):
            message = f"{name} is built from {git(pkg_dir, 'rev-parse', '--show-toplevel')} at {head[:10]}" \
                      f" (pinned {pinned(name)[:10]}), or it has uncommitted changes"
            if not dirty:
                sys.exit(message + ": commit, push and pin it (tools/bump.py), or pass --dirty")
            print("note:", message)
        out[name] = (head, pkg_dir)
    return out


def vc_redist():
    vswhere = os.path.join(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)"),
                           "Microsoft Visual Studio", "Installer", "vswhere.exe")
    vs = subprocess.run([vswhere, "-latest", "-products", "*", "-property", "installationPath"],
                        capture_output=True, text=True, encoding="utf-8").stdout.strip()
    found = sorted(glob.glob(os.path.join(vs, "VC", "Redist", "MSVC", "*", "x64", "Microsoft.VC14*.CRT")))
    if not found:
        sys.exit("no Visual C++ redistributable folder (VC\\Redist\\MSVC\\*\\x64\\Microsoft.VC14*.CRT)")
    return found[-1]


def ffmpeg_info(bin_dir):
    """What the FFmpeg DLLs say about themselves: version, licence, configuration."""
    os.add_dll_directory(bin_dir)
    avutil = ctypes.CDLL(os.path.join(bin_dir, "avutil-60.dll"))
    lines = []
    for fn in ("av_version_info", "avutil_license", "avutil_configuration"):
        f = getattr(avutil, fn)
        f.restype = ctypes.c_char_p
        lines.append(f"{fn}: {f().decode()}")
    version = lines[0].split(": ", 1)[1]
    m = re.search(r"-g([0-9a-f]+)", version)
    return "\n".join(lines) + "\n", (m.group(1) if m else version)


def start_check(folder, sw):
    """Starts the packaged program as its launcher would, with a throwaway data folder: it must
    still be running after a few seconds (missing libraries, fonts or a crash at start end it).
    `sw`: on the software renderer."""
    with tempfile.TemporaryDirectory() as data:
        if WINDOWS:
            env = dict(os.environ, LOCALAPPDATA=data)
            # Only the package's own folder and Windows: no DLL may come from PATH.
            env["PATH"] = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32")
            exe = os.path.join(folder, "novaplayer.exe")
        else:
            env = dict(os.environ, XDG_DATA_HOME=data)
            exe = os.path.join(folder, "novaplayer")
        p = subprocess.Popen([exe] + (["--backend", "sw"] if sw else []), cwd=folder, env=env)
        time.sleep(8)
        alive = p.poll() is None
        if alive:
            p.kill()
        p.wait()
        time.sleep(1)
        crash = os.path.join(data, "NovaPlayer", "crash.log")
        if not alive or os.path.exists(crash):
            log = open(crash, encoding="utf-8", errors="replace").read() if os.path.exists(crash) else ""
            sys.exit(f"start check: exited with {p.returncode} within 8 s\n{log}")
    print("start check: running after 8 s", flush=True)


def write_zip(src_dir, top, path):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for root, _, files in os.walk(src_dir):
            for name in sorted(files):
                full = os.path.join(root, name)
                z.write(full, os.path.join(top, os.path.relpath(full, src_dir)).replace(os.sep, "/"))
    with open(path + ".sha256", "w", newline="\n", encoding="utf-8") as f:
        f.write(f"{sha256(path)}  {os.path.basename(path)}\n")


def publish(stage, files):
    """Moves each of `files` (and its .sha256) from the stage into DIST, replacing an earlier
    package of the same commit; returns their new paths."""
    out = []
    for f in files:
        for p in (f, f + ".sha256"):
            dest = os.path.join(DIST, os.path.basename(p))
            if os.path.exists(dest):
                os.remove(dest)
            shutil.move(p, dest)
        out.append(os.path.join(DIST, os.path.basename(f)))
    return out


def build_info(pkg, version, date, short, commit, deps, features, extra):
    info = {
        "version": version,
        "built": date,
        "dirty": short.endswith("-dirty"),
        "features": features,
        "commits": {"novaplayer": commit, **{n: c for n, (c, _) in deps.items()}},
        **extra,
        "files": {os.path.relpath(f, pkg).replace(os.sep, "/"): sha256(f)
                  for f in sorted(glob.glob(os.path.join(pkg, "**", "*"), recursive=True)) if os.path.isfile(f)},
    }
    open(os.path.join(pkg, "BUILD-INFO.json"), "w", newline="\n", encoding="utf-8").write(json.dumps(info, indent=2) + "\n")


def package_windows(stage, version, date, short, commit, deps, sw):
    # The build staged FFmpeg's DLLs and licence (Player-Core's build script) and the fonts
    # (this crate's) next to the program.
    ffmpeg_dlls = sorted(glob.glob(os.path.join(RELEASE, "av*-*.dll")) + glob.glob(os.path.join(RELEASE, "sw*-*.dll")))
    ffmpeg_build, ffmpeg_rev = ffmpeg_info(RELEASE)
    fonts = os.path.join(RELEASE, "fonts")
    tak = os.path.join(deps["player-core"][1], "third_party", "tak")
    crt = vc_redist()
    name = f"NovaPlayer-Windows-x64-{short}"
    pkg = os.path.join(stage, name)
    lic = os.path.join(pkg, "licenses")
    os.makedirs(os.path.join(lic, "TAK"))
    os.makedirs(os.path.join(pkg, "fonts"))
    shutil.copy2(os.path.join(RELEASE, "novaplayer.exe"), pkg)
    for dll in ffmpeg_dlls:
        shutil.copy2(dll, pkg)
    shutil.copy2(os.path.join(tak, "tak_deco_lib.dll"), pkg)
    for dll in VCRUNTIME:
        shutil.copy2(os.path.join(crt, dll), pkg)
    for f in glob.glob(os.path.join(fonts, "*.ttf")) + [os.path.join(fonts, "OFL.txt")]:
        shutil.copy2(f, os.path.join(pkg, "fonts"))
    shutil.copy2(os.path.join(RELEASE, "FFmpeg-LICENSE.txt"), os.path.join(lic, "FFmpeg-LGPL-2.1.txt"))
    open(os.path.join(lic, "FFmpeg-build.txt"), "w", newline="\n", encoding="utf-8").write(ffmpeg_build)
    open(os.path.join(lic, "Microsoft-Redistributable-List.txt"), "w", newline="\n", encoding="utf-8").write(MS_REDIST)
    shutil.copy2(os.path.join(HERE, "portable", "GPL-3.0.txt"), os.path.join(lic, "GPL-3.0.txt"))
    for f in ("LICENSE-APACHE", "LICENSE-MIT"):
        shutil.copy2(os.path.join(REPO, f), os.path.join(lic, f"novaplayer-{f}.txt"))
    for f in ("Readme.html", "ref_Main.css"):
        shutil.copy2(os.path.join(tak, f), os.path.join(lic, "TAK"))
    # Windows text files: CRLF.
    crlf = lambda s: s.replace("\n", "\r\n")
    open(os.path.join(pkg, "Launch.cmd"), "w", newline="", encoding="utf-8").write(crlf(LAUNCH))
    readme = README.format(version=version, commit=short, date=date, ffmpeg=ffmpeg_rev)
    open(os.path.join(pkg, "README.txt"), "w", encoding="utf-8", newline="").write(crlf(readme))
    ffmpeg = dict(l.split(": ", 1) for l in ffmpeg_build.splitlines())
    build_info(pkg, version, date, short, commit, deps, ["asio"], {"ffmpeg": ffmpeg})
    start_check(pkg, sw)
    zip_path = os.path.join(stage, name + ".zip")
    write_zip(pkg, name, zip_path)
    return [zip_path], f"NovaPlayer-FFmpeg-source-{ffmpeg_rev}.zip"


def package_linux(stage, version, date, short, commit, deps, sw):
    fonts = os.path.join(RELEASE, "fonts")
    glibc = os.confstr("CS_GNU_LIBC_VERSION").split()[-1]
    name = f"NovaPlayer-Linux-x64-{short}"
    pkg = os.path.join(stage, name)
    lic = os.path.join(pkg, "licenses")
    os.makedirs(lic)
    os.makedirs(os.path.join(pkg, "fonts"))
    shutil.copy2(os.path.join(RELEASE, "novaplayer"), pkg)
    for f in glob.glob(os.path.join(fonts, "*.ttf")) + [os.path.join(fonts, "OFL.txt")]:
        shutil.copy2(f, os.path.join(pkg, "fonts"))
    for f in ("LICENSE-APACHE", "LICENSE-MIT"):
        shutil.copy2(os.path.join(REPO, f), os.path.join(lic, f"novaplayer-{f}.txt"))
    launch = os.path.join(pkg, "NovaPlayer.sh")
    open(launch, "w", newline="\n", encoding="utf-8").write(LAUNCH_SH)
    os.chmod(launch, 0o755)
    readme = README_LINUX.format(version=version, commit=short, date=date, glibc=glibc)
    open(os.path.join(pkg, "README.txt"), "w", encoding="utf-8", newline="\n").write(readme)
    build_info(pkg, version, date, short, commit, deps, [], {"glibc": glibc})
    start_check(pkg, sw)
    zip_path = os.path.join(stage, name + ".zip")
    write_zip(pkg, name, zip_path)
    return [zip_path], None


def main():
    args = sys.argv[1:]
    dirty, sw = "--dirty" in args, "--sw" in args
    if git(REPO, "status", "--porcelain", "--untracked-files=no") and not dirty:
        sys.exit("uncommitted changes: commit them, or pass --dirty")
    commit = git(REPO, "rev-parse", "HEAD")
    short = commit[:7] + ("-dirty" if dirty and git(REPO, "status", "--porcelain", "--untracked-files=no") else "")
    version = re.search(r'^version = "([^"]+)"', open(os.path.join(REPO, "crates", "novaplayer", "Cargo.toml"),
                                                      encoding="utf-8").read(), re.M).group(1)

    wrong = bump.lock_problems()
    if wrong and not dirty:
        sys.exit("Cargo.lock does not pin NovaUI / Player-Core as the manifests (" + "; ".join(wrong) + "): tools/bump.py")
    # Through the parent folder's [patch] when there is one (`dependencies` checks it is the pinned
    # commit); Cargo.lock is put back after. Linux has no ASIO: no default features.
    with bump.lock_kept():
        run(["cargo", "build", "--release", "-p", "novaplayer"] + ([] if WINDOWS else ["--no-default-features"]))
    deps = dependencies(dirty)
    date = time.strftime("%Y-%m-%d")

    os.makedirs(DIST, exist_ok=True)
    stage = tempfile.mkdtemp(prefix=".stage-", dir=DIST)
    try:
        package = package_windows if WINDOWS else package_linux
        made, source = package(stage, version, date, short, commit, deps, sw)
        published = publish(stage, made)
    finally:
        shutil.rmtree(stage, ignore_errors=True)

    for f in published:
        print(f"{os.path.getsize(f) / 2**20:7.1f} MiB  {f}")
    if source:
        source = os.path.join(DIST, source)
        if os.path.exists(source):
            print(f"{os.path.getsize(source) / 2**20:7.1f} MiB  {source}")
        else:
            print(f"missing: {source}: the LGPL source companion for FFmpeg; give it out with the package")
        print("give out: the Windows zip and the FFmpeg source zip; plugins: python tools/plugins.py")


if __name__ == "__main__":
    main()
