"""What CI runs on every push, from a fresh clone (Windows or Linux), one command:

  python tools/ci.py

Nothing to set up first (the build scripts fetch what they need): fmt, clippy (warnings are errors)
and the workspace's unit tests (the plugin protocol's included), which build every crate; the
program a release ships is built by tools/portable.py. Cargo.lock is used as committed
(--locked): tools/bump.py is what changes it. The L3 scenarios, the screenshots against their
baseline and the perf targets are maintained separately.
"""
import os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)

WINDOWS = sys.platform == "win32"
# Linux has no ASIO: the default features are Windows'.
FEATURES = [] if WINDOWS else ["--no-default-features"]
# The `ci` profile (Cargo.toml): optimized like a release, built in a fraction of its time (no
# LTO); the program a release ships is built by tools/portable.py with `release`.
PROFILE = ["--profile", "ci"]
# novaui's contracts (phase rules, damage audit) in the tests.
AUDIT = {"NOVAUI_AUDIT": "1"}


def step(name, cmd, env=None):
    print(f"== {name}: {' '.join(cmd)}", flush=True)
    t0 = time.time()
    r = subprocess.run(cmd, cwd=REPO, env=dict(os.environ, **(env or {})))
    print(f"== {name}: {'ok' if r.returncode == 0 else 'FAILED'} in {time.time() - t0:.0f}s", flush=True)
    if r.returncode != 0:
        sys.exit(r.returncode)


def main():
    cargo = ["cargo"]
    step("fmt", cargo + ["fmt", "--all", "--", "--check"])
    step("clippy", cargo + ["clippy", *PROFILE, "--workspace", "--all-targets", "--locked", *FEATURES, "--", "-D", "warnings"])
    step("unit tests", cargo + ["test", *PROFILE, "--workspace", "--locked", *FEATURES], AUDIT)
    print("ci: all passed")


if __name__ == "__main__":
    main()
