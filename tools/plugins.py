"""Packages the service plugins (crates/plugins/<id>) for this platform, one zip each, the way
NovaPlayer installs them (☰ › Install plugin…):

  python tools/plugins.py      into <target>/dist-plugins/NovaPlayer-plugin-<id>-<Windows|Linux>-x64.zip

Each zip holds plugin.json, the program it names and the controls' .nui files it declares. The
manifest in the repository names the Windows program (`<name>.exe`); a Linux package's manifest
names the Linux one (`<name>`), the file it carries, executable. A package is checked complete
before any is replaced.
"""
import json, os, shutil, subprocess, sys, tempfile, zipfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
DIST = os.path.join(TARGET, "dist-plugins")
WINDOWS = sys.platform == "win32"
PLATFORM = "Windows" if WINDOWS else "Linux"
PLUGINS = ["ncm", "qqmusic"]


def safe(relative):
    """A path inside the package: relative, no drive, no `.` or `..`."""
    parts = relative.replace("\\", "/").split("/")
    return relative and not os.path.isabs(relative) and ":" not in relative and not any(p in ("", ".", "..") for p in parts)


def package(stage, plugin):
    source = os.path.join(REPO, "crates", "plugins", plugin)
    manifest = json.load(open(os.path.join(source, "plugin.json"), encoding="utf-8"))
    if manifest.get("id") != plugin or manifest.get("api") != 3:
        sys.exit(f"crates/plugins/{plugin}/plugin.json: id must be {plugin!r} and api 3")
    exe = manifest["exec"]
    if not safe(exe) or not exe.endswith(".exe"):
        sys.exit(f"crates/plugins/{plugin}/plugin.json: exec {exe!r} must be a relative <name>.exe")
    program = exe[: -len(".exe")] + (".exe" if WINDOWS else "")
    manifest["exec"] = program
    controls = [c["ui"] for c in manifest.get("contributes", {}).get("controls", [])]
    for ui in controls:
        if not safe(ui):
            sys.exit(f"crates/plugins/{plugin}/plugin.json: control {ui!r} is not a path inside the package")

    zip_path = os.path.join(stage, f"NovaPlayer-plugin-{plugin}-{PLATFORM}-x64.zip")
    built = os.path.join(TARGET, "release", os.path.basename(program))
    if not os.path.isfile(built):
        sys.exit(f"missing {built}")
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("plugin.json", json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
        info = zipfile.ZipInfo.from_file(built, program)
        info.external_attr = (0o100755 << 16)  # a regular file, executable
        info.compress_type = zipfile.ZIP_DEFLATED
        with open(built, "rb") as f:
            z.writestr(info, f.read())
        for ui in controls:
            path = os.path.join(source, ui)
            if not os.path.isfile(path):
                sys.exit(f"missing {path}")
            z.write(path, ui.replace("\\", "/"))
    return zip_path


def main():
    packages = [a for p in PLUGINS for a in ("-p", f"novaplayer-plugin-{p}")]
    r = subprocess.run(["cargo", "build", "--release", "--locked", *packages], cwd=REPO)
    if r.returncode != 0:
        sys.exit(f"cargo build failed ({r.returncode}); nothing packaged")
    os.makedirs(DIST, exist_ok=True)
    stage = tempfile.mkdtemp(prefix=".stage-", dir=DIST)
    try:
        made = [package(stage, p) for p in PLUGINS]
        for path in made:
            dest = os.path.join(DIST, os.path.basename(path))
            if os.path.exists(dest):
                os.remove(dest)
            shutil.move(path, dest)
            print(f"{os.path.getsize(dest) / 2**20:6.1f} MiB  {dest}")
    finally:
        shutil.rmtree(stage, ignore_errors=True)


if __name__ == "__main__":
    main()
