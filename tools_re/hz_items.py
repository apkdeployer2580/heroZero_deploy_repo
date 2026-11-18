"""Dump 영웅서기 Zero's item tables (`/itm/00`..`/itm/20`) from the jar.

usage:  python -I tools_re/hz_items.py <game.jar> [category ...]

Layout (javap 2026-10-25, `ba.a(BB)Lba;` + `rpg/e.a(String,I)[B`): a file is records of a u16
big-endian length + the record; a record is `ba.a([BI)I`'s name (u8 length + EUC-KR), price
(int BE — the shop's buying price, it buys back at a tenth), text (u8 length + EUC-KR), then for
equipment (categories 0..15, `bx.a([BI)I`) level `d:B`, grade `e:B`, `f:B`, sprite `g:B`, four
shorts `a b c d` (`a` the palette; `b c d` the stats the item window shows), slots `h:B` and six
option bytes `c:[B` (100 = rolled 0..15, -3 none), and last a check byte = the byte sum before it.
Categories: `t/it.tdf` [0..20] (도끼 … 특수).
"""
import struct
import sys
import zipfile

CATEGORIES = ["도끼", "양손검", "도검", "둔기", "단검", "총", "총검", "방패", "경장갑", "중장갑", "투구", "장신구", "경갑옷", "중갑옷",
              "경신발", "중신발", "조합재료", "마법석", "조합서", "퀘스트", "특수"]
GRADES = ["일반", "레어", "에픽", "세트"]


def records(data):
    p, out = 0, []
    while p + 2 <= len(data):
        n = struct.unpack(">H", data[p:p + 2])[0]
        out.append(data[p + 2:p + 2 + n])
        p += 2 + n
    return out, p == len(data)


def parse(cat, r):
    p = 0
    n = r[p]; name = r[p + 1:p + 1 + n].decode("cp949", "replace"); p += 1 + n
    price = struct.unpack(">i", r[p:p + 4])[0]; p += 4
    n = r[p]; text = r[p + 1:p + 1 + n].decode("cp949", "replace").replace("\r\n", "|"); p += 1 + n
    item = {"name": name, "price": price, "text": text}
    if cat <= 15:
        level, grade, f, sprite = struct.unpack("bbbb", r[p:p + 4]); p += 4
        a, b, c, d = struct.unpack(">hhhh", r[p:p + 8]); p += 8
        slots = struct.unpack("b", r[p:p + 1])[0]; p += 1
        options = list(struct.unpack("6b", r[p:p + 6])); p += 6
        item.update(level=level, grade=grade, f=f, sprite=sprite, palette=a, stats=(b, c, d), slots=slots, options=options)
    item["rest"] = r[p:-1].hex()
    item["check_ok"] = (sum(r[:-1]) & 0xff) == r[-1]
    return item


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return 2
    z = zipfile.ZipFile(args[0])
    cats = [int(c) for c in args[1:]] or list(range(21))
    for cat in cats:
        rs, whole = records(z.read(f"itm/{cat:02d}"))
        print(f"== /itm/{cat:02d} {CATEGORIES[cat]}: {len(rs)} items{'' if whole else ' (trailing bytes!)'}")
        for i, r in enumerate(rs):
            it = parse(cat, r)
            if cat <= 15:
                g = GRADES[it["grade"]] if 0 <= it["grade"] < 4 else it["grade"]
                print(f"  [{i:3}] {it['name']:<14} {it['price']:>8}  LV{it['level']:>3} {g} f{it['f']} spr{it['sprite']} pal{it['palette']}"
                      f" stats{it['stats']} slots{it['slots']} opt{it['options']}{'' if it['check_ok'] else ' BAD CHECK'}"
                      f"{' +' + it['rest'] if it['rest'] else ''}  {it['text']}")
            else:
                print(f"  [{i:3}] {it['name']:<14} {it['price']:>8}  {it['text']}{'' if it['check_ok'] else ' BAD CHECK'}{' +' + it['rest'] if it['rest'] else ''}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
