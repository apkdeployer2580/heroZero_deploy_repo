"""Dump 영웅서기 Zero's text tables (`t/*.tdf`) from the jar.

usage:  python -I tools_re/tdf.py <game.jar> [table ...] [--ids 14,29,30]

Format (all 31 tables decode exactly to their last byte): u8 count, then `count` entries
of u16 big-endian length + EUC-KR (CP949) bytes. CR LF inside an entry is a line break in
the game's text box; it is printed as `|`. With no table names, every table is listed.
"""
import struct
import sys
import zipfile


def decode(data: bytes):
    n, p, out = data[0], 1, []
    for _ in range(n):
        ln = struct.unpack(">H", data[p:p + 2])[0]
        out.append(data[p + 2:p + 2 + ln].decode("cp949", "replace").replace("\r\n", "|"))
        p += 2 + ln
    return out, p == len(data)


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    ids = None
    if "--ids" in sys.argv:
        ids = {int(x) for x in sys.argv[sys.argv.index("--ids") + 1].split(",")}
        args = [a for a in args if a != sys.argv[sys.argv.index("--ids") + 1]]
    if not args:
        print(__doc__)
        return 2
    jar = zipfile.ZipFile(args[0])
    names = [f"t/{t}" if not t.startswith("t/") else t for t in args[1:]] or sorted(
        n for n in jar.namelist() if n.startswith("t/") and n.endswith(".tdf"))
    for name in names:
        if not name.endswith(".tdf"):
            name += ".tdf"
        rows, exact = decode(jar.read(name))
        print(f"== {name}: {len(rows)} entries{'' if exact else ' (TRAILING BYTES — not this format?)'}")
        for i, r in enumerate(rows):
            if ids is None or i in ids:
                print(f"  [{i:3}] {r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
