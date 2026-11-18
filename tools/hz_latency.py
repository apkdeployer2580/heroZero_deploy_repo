"""Key → drawn: how long after a press the game draws a frame that holds it, in the probe's virtual time.

    python -I tools/hz_latency.py [--exe target/release/hz_probe.exe] [--jar <herozero.jar>] [--out <dir>]
                                  [--patches default]

The same new game as the watchline (tools/hz_watchline.py) up to the home map's field (~7,150 frames),
then 24 direction changes (RIGHT, DOWN, LEFT, UP in turn, each held 30 frames) 31 frames apart: at
`--ms 20` the game's frame is 60 virtual ms = 3 probe frames, so the presses walk through every phase of
it. The probe prints one line a press (`key->drawn`, from `hz_java::GuestState::key` — the canvas's
`f.c` as the blit hook reads it on each drawn frame) and a summary; this prints both.

Press → drawn is the game thread's own share: the pump, the loop's key read (`f.a()`), its sleep, the
update and the draw. What the phone adds (touch → worker, the UI thread's present) is the same before
and after any change here, and is measured on the device by the worker's `emu gauge` line instead.

`--patches default,-HZ-nnn` is the A/B of one patch. (HZ-029 — the loop's key read moved to just
before its frame — was tried and dropped here, 2026-10-28: on the phone the game's paint then left
BEFORE the key read, so the host's release landed in the same read as the press and a short tap
collapsed; HEROZERO_REMAKE_PLAN.md §10, 42nd session.)
"""

import argparse
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hz_watchline  # noqa: E402

ROOT = hz_watchline.ROOT

FIRST = 7500
SPACING = 31
HOLD = 30
PRESSES = 24


def keys():
    """The opening as the watchline plays it, then the phase-swept direction changes."""
    k = [x for x in hz_watchline.keys().split(",") if int(x.split(":")[0]) < FIRST]
    dirs = ["RIGHT", "DOWN", "LEFT", "UP"]
    for i in range(PRESSES):
        k.append(f"{FIRST + i * SPACING}:{dirs[i % 4]}:{HOLD}")
    return ",".join(k)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--exe", default=os.path.join(ROOT, "target", "release", "hz_probe.exe"))
    ap.add_argument("--jar", default=os.path.join(ROOT, "android_app", "app", "src", "main", "assets", "herozero.jar"))
    ap.add_argument("--out", default=os.path.join(os.environ.get("TEMP", ROOT), "hz-latency"))
    ap.add_argument("--patches", default="default")
    a = ap.parse_args()
    a.exe, a.jar, a.out = (os.path.abspath(p) for p in (a.exe, a.jar, a.out))
    for p in (a.exe, a.jar):
        if not os.path.exists(p):
            sys.exit(f"missing {p}")
    os.makedirs(a.out, exist_ok=True)

    frames = FIRST + PRESSES * SPACING + 100
    cmd = [a.exe, "--jar", a.jar, "--w", "520", "--h", "240", "--ms", "20", "--frames", str(frames),
           "--every", "0", "--keys", keys(), "--patches", a.patches, "--out", a.out]
    run = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    log = run.stdout + run.stderr
    with open(os.path.join(a.out, "log.txt"), "w", encoding="utf-8") as f:
        f.write(log)
    if run.returncode != 0:
        sys.exit(f"probe exited {run.returncode} — {os.path.join(a.out, 'log.txt')}")

    lines = [l for l in log.splitlines() if "key->drawn" in l or re.search(r"key \w+ pressed f=", l)]
    swept = [l for l in lines if re.search(r"pressed f=(\d+)", l) and int(re.search(r"pressed f=(\d+)", l)[1]) >= FIRST]
    for l in swept:
        print(l)
    ms = [int(m[1]) for l in swept for m in [re.search(r"\(\+(\d+) virtual ms", l)] if m]
    if ms:
        print(f"direction changes: {len(ms)} of {PRESSES} drawn, avg {sum(ms) / len(ms):.1f} virtual ms, "
              f"min {min(ms)} max {max(ms)}")
    else:
        print("no direction change was drawn — did the run reach the field? see log.txt")
    scene = re.findall(r"hz: scene GuestView \{ scene: (\w+)", log)
    print(f"scenes: {' '.join(dict.fromkeys(scene))}")
    bad = [l for l in log.splitlines() if re.search(r"Uncaught|GUEST ERROR|panicked", l)]
    print(f"guest errors: {len(bad)}")


if __name__ == "__main__":
    main()
