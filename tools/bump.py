"""Points NovaPlayer at a pushed commit of NovaUI or Player-Core:

  python tools/bump.py novaui [rev]
  python tools/bump.py player-core [rev]

`rev` defaults to the HEAD of the sibling checkout (../novaui, ../player-core). The commit must
be on the server (on one of its branches): a rev only this machine has would build here, through
the local [patch], and nowhere else. Every manifest line naming that repository gets the full
hash (Player-Core's also pins cpal in the workspace Cargo.toml); Cargo.lock is refreshed the way
a fresh clone resolves it, without the parent folder's [patch]; both are staged, to be committed
with the change that needs them.
"""
import contextlib, glob, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
DEPS = {
    "novaui": ("NovaUI.git", ["novaui", "nova-icons", "novaui-shaders"]),
    "player-core": ("Player-Core.git", ["player-core", "cpal"]),
}


def git(cwd, *args):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8")
    if r.returncode != 0:
        sys.exit(f"git {' '.join(args)} in {cwd}: {r.stderr.strip()}")
    return r.stdout.strip()


def _working_tree(rel):
    return open(os.path.join(REPO, rel), encoding="utf-8").read()


def pins(read=_working_tree):
    """The commit the manifests pin for each repository (`DEPS`' key: `novaui`, `player-core`).
    `read(path relative to the repo)` gives a file's text: the working tree's by default."""
    manifests = ["Cargo.toml"] + [os.path.relpath(p, REPO).replace(os.sep, "/")
                                   for p in glob.glob(os.path.join(REPO, "crates", "**", "Cargo.toml"), recursive=True)]
    repo_of = {tail: repo for repo, (tail, _) in DEPS.items()}
    found = {}
    for rel in manifests:
        for tail, rev in re.findall(r'git = "[^"]*/([A-Za-z-]+\.git)", rev = "([0-9a-f]+)"', read(rel)):
            if tail in repo_of:
                found[repo_of[tail]] = rev
    return found


def lock_problems(read=_working_tree):
    """What in Cargo.lock does not match the manifests' pins: each package of NovaUI and
    Player-Core must come from the pinned commit. A lock written by a local build (through the
    parent folder's [patch]) names them with no source, and a clone without the [patch] cannot
    build with it (`--locked`, tools/ci.py). `read(path relative to the repo)` gives a file's text:
    the working tree's by default."""
    pinned = pins(read)
    lock = read("Cargo.lock")
    wrong = []
    for repo, (tail, packages) in DEPS.items():
        for name in packages + (["media-io"] if repo == "player-core" else []):
            m = re.search(rf'\[\[package\]\]\nname = "{re.escape(name)}"\nversion = "[^"]+"\n(?:source = "([^"]+)")?', lock)
            source = (m and m.group(1)) or ""
            if f"/{tail}?rev={pinned.get(repo, '?')}#" not in source:
                wrong.append(f"{name}: {source.rsplit('#', 1)[-1][:10] if source else 'no source'}")
    return wrong


@contextlib.contextmanager
def lock_kept():
    """Puts Cargo.lock back after a local build: through the parent folder's [patch] Cargo rewrites
    it to name the sibling checkouts, which must not be committed (`lock_problems`)."""
    path = os.path.join(REPO, "Cargo.lock")
    kept = open(path, "rb").read()
    try:
        yield
    finally:
        if open(path, "rb").read() != kept:
            open(path, "wb").write(kept)


def main():
    if len(sys.argv) not in (2, 3) or sys.argv[1] not in DEPS:
        sys.exit(__doc__)
    name = sys.argv[1]
    url_tail, packages = DEPS[name]
    sibling = os.path.join(os.path.dirname(REPO), name)
    if not os.path.isdir(os.path.join(sibling, ".git")):
        sys.exit(f"no checkout at {sibling}")
    rev = git(sibling, "rev-parse", sys.argv[2] if len(sys.argv) == 3 else "HEAD")
    git(sibling, "fetch", "--quiet", "origin")
    branches = git(sibling, "branch", "--remotes", "--contains", rev)
    if not branches:
        sys.exit(f"{name} {rev[:10]} is not on the server: push it first")
    if git(sibling, "status", "--porcelain", "--untracked-files=no") and len(sys.argv) == 2:
        print(f"note: {sibling} has uncommitted changes; they are not in {rev[:10]}")

    manifests = [os.path.join(REPO, "Cargo.toml")] + glob.glob(os.path.join(REPO, "crates", "**", "Cargo.toml"), recursive=True)
    changed = []
    for path in manifests:
        text = open(path, encoding="utf-8").read()
        new = "\n".join(re.sub(r'rev = "[0-9a-f]+"', f'rev = "{rev}"', line) if url_tail in line else line
                        for line in text.split("\n"))
        if new != text:
            open(path, "w", encoding="utf-8", newline="\n").write(new)
            changed.append(os.path.relpath(path, REPO))
    if not changed:
        print(f"no manifest names {url_tail}")
    # Resolve from outside this folder tree: Cargo reads `.cargo/config.toml` from the working
    # directory up, so the parent's [patch] (local checkouts) does not apply here.
    with tempfile.TemporaryDirectory() as outside:
        cmd = ["cargo", "update", "--manifest-path", os.path.join(REPO, "Cargo.toml"),
               "--config", "net.git-fetch-with-cli=true"] + [a for p in packages for a in ("-p", p)]
        r = subprocess.run(cmd, cwd=outside)
        if r.returncode != 0:
            sys.exit("cargo update failed")
    git(REPO, "add", "Cargo.lock", *changed)
    print(f"{name} -> {rev} ({branches.splitlines()[0].strip()}); staged: Cargo.lock {' '.join(changed)}")


if __name__ == "__main__":
    main()
