"""Stage the 영웅서기 Zero jar where the build and the probe read it.

usage:  python -I tools_re/make_hz_asset.py "dist/영웅서기제로(최종버전).zip" [--out <path>]

The input is the download package exactly as the user has it: a zip holding ONE
MIDlet jar (its member name is CP949 without the UTF-8 flag, so it reads as mojibake
in most tools — it is matched by extension, not by name). The jar is copied out
byte-for-byte: no repack, no edits. Every Hero Zero patch lives in the host
(`hz_patches`), keyed by the md5 of the class it applies to, so the asset stays the
untouched original and a patch can never silently land on a different build.

A bare .jar is accepted as input too.

Exit status: 0 staged, 1 refused (unknown build or unreadable input), 2 usage.
"""

import argparse
import hashlib
import io
import sys
import zipfile
from pathlib import Path

# The builds this repository knows. A patch table, an anchor or a probe baseline is
# only meaningful for a jar listed here; anything else is refused rather than staged,
# because "boots, but every patch silently misses" is the failure this prevents.
KNOWN_JARS = {
    "850ad70748b9e1e48b30f6fa60164229": "영웅서기 Zero 진홍의 사도 — EA Mobile CN port, Korean text, MIDlet-Version 1.0.0, 27,334,365 B (최종버전)",
}

REPO = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO / "android_app" / "app" / "src" / "main" / "assets" / "herozero.jar"


def extract_jar(src: Path) -> bytes:
    data = src.read_bytes()
    if not zipfile.is_zipfile(io.BytesIO(data)):
        raise ValueError(f"{src}: not a zip/jar")
    z = zipfile.ZipFile(io.BytesIO(data))
    names = z.namelist()
    if any(n.endswith(".class") for n in names) or "META-INF/MANIFEST.MF" in names:
        return data  # already a bare jar
    jars = [n for n in names if n.lower().endswith(".jar")]
    if len(jars) != 1:
        raise ValueError(f"{src}: expected exactly one .jar member, found {len(jars)}: {jars!r}")
    return z.read(jars[0])


def main() -> int:
    sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("source", type=Path, help="the original download package (.zip) or a bare .jar")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    a = ap.parse_args()

    try:
        jar = extract_jar(a.source)
    except (OSError, ValueError, zipfile.BadZipFile) as e:
        print(f"refused: {e}", file=sys.stderr)
        return 1

    md5 = hashlib.md5(jar).hexdigest()
    build = KNOWN_JARS.get(md5)
    if build is None:
        print(f"refused: jar md5 {md5} ({len(jar):,} B) is not a known build.", file=sys.stderr)
        print("Known builds:", file=sys.stderr)
        for k, v in KNOWN_JARS.items():
            print(f"  {k}  {v}", file=sys.stderr)
        return 1

    manifest = zipfile.ZipFile(io.BytesIO(jar)).read("META-INF/MANIFEST.MF").decode("utf-8", "replace")
    midlet = next((l.split(":", 1)[1].strip() for l in manifest.splitlines() if l.startswith("MIDlet-1:")), "?")

    a.out.parent.mkdir(parents=True, exist_ok=True)
    if a.out.exists() and hashlib.md5(a.out.read_bytes()).hexdigest() == md5:
        print(f"already staged: {a.out} (md5 {md5})")
        return 0
    a.out.write_bytes(jar)
    print(f"staged {a.out}")
    print(f"  md5     {md5}")
    print(f"  build   {build}")
    print(f"  MIDlet  {midlet}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
