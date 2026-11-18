"""Name and sum the stacks `hz_probe --sample <file>` wrote (hz_probe/src/sampler.rs).

    python -I tools/hz_profile.py <samples.txt> [--exe target_prof/release/hz_probe.exe] [--top 40]

Every address goes through `llvm-symbolizer --inlining` (the NDK's), so an inlined function
counts as itself, not as the function it was folded into. Prints two tables:
  self       — the innermost function a sample was in (where the time is spent)
  inclusive  — every function on the sample's stack, once per sample (what the time is spent for)

Build the probe for it with line tables (the stacks come from the system's unwind tables, so no
frame pointers are needed), into its own target dir:

    $env:CARGO_PROFILE_RELEASE_DEBUG = "line-tables-only"
    cargo build --release -p hz_probe --target-dir target_prof      # (with build_probe.ps1's dlltool shim on PATH)
    target_prof\\release\\hz_probe.exe ... --sample out.txt --sample-from 3000
    python -I tools/hz_profile.py out.txt

First measured 2026-10-27 (map 44 field): resolution by name and its allocations were most of the
game's frame — HEROZERO_REMAKE_PLAN.md §10, thirty-eighth session.
"""

import argparse
import collections
import os
import re
import subprocess
import sys


def symbolizer():
    sdk = os.environ.get("ANDROID_HOME") or os.path.join(os.environ.get("LOCALAPPDATA", ""), "Android", "Sdk")
    ndk = os.path.join(sdk, "ndk")
    for v in sorted(os.listdir(ndk), reverse=True) if os.path.isdir(ndk) else []:
        p = os.path.join(ndk, v, "toolchains", "llvm", "prebuilt", "windows-x86_64", "bin", "llvm-symbolizer.exe")
        if os.path.exists(p):
            return p
    sys.exit("no llvm-symbolizer under the NDK")


# Generic noise that says nothing about which function it is: closures and monomorphisations.
TRIM = [
    (re.compile(r"::\{\{closure\}\}"), ""),
    (re.compile(r"::h[0-9a-f]{16}$"), ""),
    (re.compile(r"<impl [^>]*>"), "<impl>"),
]


def short(name):
    for pat, rep in TRIM:
        name = pat.sub(rep, name)
    return name if len(name) <= 140 else name[:137] + "..."


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("samples")
    ap.add_argument("--exe", default=os.path.join("target_prof", "release", "hz_probe.exe"))
    ap.add_argument("--top", type=int, default=40)
    a = ap.parse_args()

    stacks = []
    base = None
    with open(a.samples) as f:
        for line in f:
            if line.startswith("# base"):
                base = int(line.split()[2], 16)
                continue
            parts = line.split()
            if parts:
                stacks.append([int(x, 16) for x in parts])
    if base is None or not stacks:
        sys.exit("no samples")
    # The image's preferred base: llvm-symbolizer wants the address as linked.
    image_base = 0x140000000
    addrs = sorted({x for s in stacks for x in s})
    out = subprocess.run(
        [symbolizer(), "--obj=" + a.exe, "--inlining", "--demangle", "--functions=linkage", "--no-untag-addresses"],
        input="\n".join(hex(image_base + x) for x in addrs) + "\n",
        capture_output=True,
        text=True,
    ).stdout
    # Blocks separated by blank lines: (function, file:line) pairs, innermost first.
    blocks = out.strip("\n").split("\n\n")
    names = {}
    for addr, block in zip(addrs, blocks):
        lines = block.split("\n")
        chain = [short(lines[i]) for i in range(0, len(lines), 2)]
        names[addr] = [c if c != "??" else f"?{addr:#x}" for c in chain]

    selfc = collections.Counter()
    incl = collections.Counter()
    for s in stacks:
        selfc[names[s[0]][0]] += 1
        seen = set()
        for x in s:
            for n in names[x]:
                if n not in seen:
                    seen.add(n)
                    incl[n] += 1
    total = len(stacks)
    print(f"{total} samples")
    print("\n== self ==")
    for n, c in selfc.most_common(a.top):
        print(f"{100.0 * c / total:6.2f}%  {n}")
    print("\n== inclusive ==")
    for n, c in incl.most_common(a.top * 2):
        print(f"{100.0 * c / total:6.2f}%  {n}")


if __name__ == "__main__":
    main()
