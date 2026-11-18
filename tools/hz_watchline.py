"""The performance watchline: one fixed probe run, its bytes hashed and its time measured.

    python -I tools/hz_watchline.py [--exe target/release/hz_probe.exe] [--jar <herozero.jar>] [--out <dir>]

A new game from an empty RecordStore set (nothing per-PC, nothing from out/), so any PC with the jar
gets the same run: boot → sound question → 새로하기 → the opening scripts (OK and # in turn) → the
home map 136 field → a walk → the game's own warp to map 44 (실바나르_남부, eleven monsters) →
walks and attacks, the game's own death on one monster → menu 1 캐릭터 · 2 인벤토리 · 5 스킬 →
0 시스템 → 저장 (slot 1). 520×240, as the S25.

Prints two hashes and the speed:
  frames  md5 over every dumped frame's PNG bytes in frame order
  rms     md5 over the RecordStores at the end (store name, record file, bytes)
  the field ticks on map 44 (`--timing`), the run's wall time, and any guest error lines.

The probe runs on a virtual clock and its executor polls tasks in id order, so the same exe on the
same jar gives the same bytes (2026-10-27, wie_backend executor ★). A change to the JVM or the
platform that is meant to be invisible (performance work) must leave both hashes as they were; a
change to the game's patches or the hooks moves them, and then the new pair is the baseline —
CLAUDE.md 「성능 감시선」 records it with the commit it was taken at.
"""

import argparse
import hashlib
import os
import re
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

FRAMES = 11000
EVERY = 200
TIMING = 300


def keys(shift=0):
    """Every key `shift` probe frames later (negative = earlier): with the game's frame at 50 virtual ms
    and `--ms 20`, a shift of 3 moves every press and release one game frame."""
    k = ["300:OK", "400:OK", "500:OK"]
    # The opening: scripts that want OK and 「# 확인」 notices that want #, the field at ~7150.
    for i, f in enumerate(range(600, 7401, 60)):
        k.append(f"{f}:{'OK' if i % 2 == 0 else 'HASH'}")
    # The home map: a few steps each way.
    k += ["7500:DOWN:20", "7560:LEFT:15", "7620:RIGHT:15"]
    # Map 44 (the warp is at 7700): walk in, then fight where the monsters are.
    k += ["7800:DOWN:60", "7900:RIGHT:40"]
    for i, f in enumerate(range(8000, 8900, 20)):
        k.append(f"{f}:OK")
        if i % 9 == 4:
            k.append(f"{f + 10}:{['LEFT', 'DOWN', 'RIGHT', 'UP'][(i // 9) % 4]}:8")
    # The kill at 8500 levels the warrior up: its 「# 확인」 notice holds the field until #.
    k += ["8960:HASH", "9060:HASH"]
    # The menu and its windows, then 0 시스템 → 저장하시겠습니까? 예 → saved.
    k += ["9200:LSK", "9300:1", "9450:RSK", "9550:2", "9700:RSK", "9800:5", "9950:RSK",
          "10050:0", "10250:OK", "10450:OK", "10650:RSK", "10800:RSK"]
    if shift:
        k = [":".join([str(int(x.split(":")[0]) + shift)] + x.split(":")[1:]) for x in k]
    return ",".join(k)


def md5_files(paths, base):
    h = hashlib.md5()
    for p in paths:
        h.update(os.path.relpath(p, base).replace("\\", "/").encode())
        with open(p, "rb") as f:
            h.update(f.read())
    return h.hexdigest()[:8]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--exe", default=os.path.join(ROOT, "target", "release", "hz_probe.exe"))
    ap.add_argument("--jar", default=os.path.join(ROOT, "android_app", "app", "src", "main", "assets", "herozero.jar"))
    ap.add_argument("--out", default=os.path.join(os.environ.get("TEMP", ROOT), "hz-watchline"))
    ap.add_argument("--shift", type=int, default=0, help="every key this many probe frames later (3 = one game frame)")
    ap.add_argument("--patches", default="default", help="the probe's --patches (e.g. default,-HZ-028)")
    a = ap.parse_args()
    a.exe, a.jar, a.out = (os.path.abspath(p) for p in (a.exe, a.jar, a.out))
    for p in (a.exe, a.jar):
        if not os.path.exists(p):
            sys.exit(f"missing {p}")

    os.makedirs(a.out, exist_ok=True)
    for name in os.listdir(a.out):
        if name.startswith("frame_") and name.endswith(".png"):
            os.remove(os.path.join(a.out, name))
    rms = os.path.join(a.out, "rms")
    shutil.rmtree(rms, ignore_errors=True)  # stores left by an earlier run would join the hash
    cmd = [a.exe, "--jar", a.jar, "--w", "520", "--h", "240", "--ms", "20", "--frames", str(FRAMES),
           "--every", str(EVERY), "--timing", str(TIMING), "--keys", keys(a.shift), "--patches", a.patches,
           "--warp", "7700:44:13:1", "--kill", "8500", "--out", a.out, "--rms-out", rms]
    started = time.time()
    run = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    wall = time.time() - started
    log = run.stdout + run.stderr
    with open(os.path.join(a.out, "log.txt"), "w", encoding="utf-8") as f:
        f.write(log)
    if run.returncode != 0:
        sys.exit(f"probe exited {run.returncode} — {os.path.join(a.out, 'log.txt')}")

    frames = sorted(os.path.join(a.out, n) for n in os.listdir(a.out) if re.fullmatch(r"frame_\d+\.png", n))
    stores = sorted(os.path.join(d, n) for d, _, ns in os.walk(rms) for n in ns)
    print(f"frames {md5_files(frames, a.out)}  ({len(frames)} dumps)")
    print(f"rms    {md5_files(stores, rms)}  ({len(stores)} records)")

    # Map 44's field windows while it fights, after the warp's load and before the level-up notice.
    ticks = []
    for m in re.finditer(r"\[probe\] f=(\d+) timing: tick avg ([\d.]+) ms worst ([\d.]+) ms \(f=(\d+)\).*scene (\w+)", log):
        f, avg, worst, at, scene = int(m[1]), float(m[2]), float(m[3]), int(m[4]), m[5]
        if 7800 < f <= 8400 and scene == "Field":
            ticks.append((f, avg, worst, at))
    if ticks:
        avg = sum(t[1] for t in ticks) / len(ticks)
        worst = max(ticks, key=lambda t: t[2])
        print(f"map 44 field ticks: avg {avg:.2f} ms over {len(ticks) * TIMING} frames, worst {worst[2]:.2f} ms at f={worst[3]}")
    else:
        print("map 44 field ticks: none (the run did not reach the field on map 44 — see log.txt)")
    done = re.search(r"\[probe\] done: .*", log)
    print(done[0] if done else "no `[probe] done` line")
    latency = re.search(r"\[probe\] key->drawn: .*", log)
    if latency:
        print(latency[0])
    print(f"wall {wall:.1f} s")

    bad = [l for l in log.splitlines() if re.search(r"Uncaught|GUEST ERROR|panicked", l)]
    print(f"guest errors: {len(bad)}")
    for l in bad[:10]:
        print("  " + l[:200])
    saved = re.findall(r"\[rms\] (HOL\.\w+) (?:add|set) #\d+ \((\d+) B\)", log)
    print("saved: " + (", ".join(f"{s} {b} B" for s, b in saved) if saved else "nothing"))


if __name__ == "__main__":
    main()
