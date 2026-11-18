"""Dump every monster's drop table from `/en/edf` (u16-BE-length records, EUC-KR names).

usage:  python -I tools_re/edf_drops.py <herozero.jar>

Per monster: name, level and the four drop slots `[j] cat_lo,cat_hi <threshold`. The game rolls
R = abs(Random.nextInt()) % 65535 once per kill (`cq.o()V`) and the first slot with R < threshold
drops (HEROZERO_GAME_OPTIONS_RE.md §3). Field layout before the slots is only partly understood —
the names of the skipped fields are guesses. Read-only.
(Written during the 2026-10-22 drop RE.)
"""
import struct
import sys
import zipfile


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    sys.stdout.reconfigure(encoding="utf-8")
    d = zipfile.ZipFile(sys.argv[1]).read("en/edf")
    recs, off = [], 0
    while off + 2 <= len(d):
        n = struct.unpack(">H", d[off:off + 2])[0]
        recs.append(d[off + 2:off + 2 + n])
        off += 2 + n
    print(f"edf {len(d)} B, {len(recs)} records")
    for i, r in enumerate(recs):
        if len(r) < 2:
            print(i, "short", r.hex())
            continue
        nl = r[0]
        name = r[1:1 + nl]
        try:
            nm = name.decode("euc-kr")
        except UnicodeDecodeError:
            nm = name.hex()
        p = 1 + nl + 3 + 4 + 6 + 2 + 6  # k,a,l · c4 · d6 · bB,cB · aS,bS,cS
        lvl = r[p]
        p += 1
        hp = struct.unpack(">h", r[p:p + 2])[0]
        p += 6 + 3 + 1  # dS,eS,fS · za,zb,zc · eB
        drop = r[p:p + 24]
        slots = []
        for j in range(4):
            e = drop[j * 6:j * 6 + 6]
            a0, a1 = struct.unpack("bb", e[0:2])
            thr = struct.unpack(">i", e[2:6])[0]
            slots.append(f"[{j}] {a0},{a1} <{thr}")
        print(f"{i:3d} {nm:<12} lvl={lvl:3d} hp?={hp} " + " | ".join(slots))


if __name__ == "__main__":
    main()
