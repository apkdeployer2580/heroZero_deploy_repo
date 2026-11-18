"""Every map of the jar, parsed the way the game parses it, and the exits between them — the data
the world map (Phase 7) is drawn from.

usage:  python -I tools_re/hz_maps.py <herozero.jar> [--exits] [--check] [--map N]
  (no flag)  one line per map: number, name, size, step triggers on its floor
  --exits    every step-trigger warp: map → map (x, y), the road graph
  --check    the script tables parse to their exact ends in every map (exit 1 if not)
  --map N    one map in full: header, triggers with their tiles, scripts, warps

Read-only. Written 2026-10-23 (fifteenth session) from javap of rpg/e, g, eb, dw, cp, n — see
HEROZERO_REMAKE_PLAN.md §5.6 for the format and how it was established.

Map file (`/m/m/NN.m`, maps >= 100 `/m/m2/<'a'+(n-100)/10><n%10>.m`; loader rpg/e.g(), parser
g.a([B)V):
  u8 kind · u8 ac[2] · 2 strings (u8 length + EUC-KR: the map's name, a second name) ·
  s8 tile kinds + that many tile ids · u8 width, u8 height · tile layer w·h · attribute layer w·h
  (`& 192` blocked, `& 63` trigger) · three object lists (u8 count, 5 B each) · monster kinds
  (s8 count + ids) · monster spots (s8 count; 3 B, +1 when byte 1 & 0x80) · NPC kinds (s8 count +
  ids) · NPCs (s8 count; 3 B, +1 when byte 1 & 0x80) · script tables (dw.a([BI)I):
    table A: u8 count, count × u8 length, bytes   — per trigger number: a condition tree
    table B: u8 count, count × u16le length, bytes — action lists the trees point at
    u8 count, count × u8 length, then count × (bytes, 1 pad byte)
Condition tree (dw.a(n)): s8 b; b >= 0 → queue action list B[b]; b == -1 → u8 k, k lengths,
then k sub-trees of those lengths (first that holds wins); b <= -2 → condition -2-b with its
operands (COND_SIZE); a false condition ends the tree.
Action list (dw.b()): s8 op; op == -1 → u8 x, u8 k, k pairs (u8, u8 length), then k bodies of
those lengths (a choice — each body an action list); else op's operands (ACT_SIZE).
Action 23 = warp: map, x, y, facing (eb.b(BLn;)Z case 23 → eb.ac; same map = a step only).
"""
import sys, zipfile

# Operand bytes per condition (eb.a(BLn;)Z) and action (eb.b(BLn;)Z) opcode. Read off the
# interpreter: n.a()B = 1, n.a()S / n.b()I = 2, n.a([BI)V = I; 17 and 20 read 2 B three times.
COND_SIZE = [2, 2, 2, 3, 2, 3, 2, 3, 1, 1, 0]
ACT_SIZE = [1, 2, 2, 2, 1, 1, 2, 1, 0, 1, 1, 2, 9, 2, 1, 2, 9, 6, 0, 1,
            6, 6, 0, 4, 3, 2, 2, 2, 3, 2, 2, 4, 2, 2, 0, 2, 0, 3, 0, 0,
            0, 0, 1, 3, 1, 1, 2, 1, 2, 0, 2, 0, 1, 2, 2, 3, 3, 1, 4]
WARP = 23


def map_path(n):
    if n < 100:
        return f'm/m/{n:02d}.m'
    return f'm/m2/{chr(ord("a") + (n - 100) // 10)}{n % 10}.m'


def s8(v):
    return v - 256 if v >= 128 else v


class Map:
    def __init__(self, number, b):
        self.number = number
        p = 0
        self.kind = b[p]; p += 1
        self.ac = (b[p], b[p + 1]); p += 2
        names = []
        for _ in range(2):
            n = s8(b[p]); p += 1
            names.append(b[p:p + max(n, 0)].decode('euc-kr', 'replace') if n > 0 else '')
            p += max(n, 0)
        self.name, self.name2 = names
        k = s8(b[p]); p += 1
        self.tile_ids = list(b[p:p + k]); p += k
        self.w, self.h = b[p], b[p + 1]; p += 2
        n = self.w * self.h
        self.tiles = b[p:p + n]; p += n
        self.attrs = b[p:p + n]; p += n
        self.objects = []
        for _ in range(3):
            c = b[p]; p += 1
            self.objects.append([tuple(b[p + 5 * i:p + 5 * i + 5]) for i in range(c)]); p += 5 * c
        k = s8(b[p]); p += 1
        self.monster_kinds = list(b[p:p + k]); p += k
        c = s8(b[p]); p += 1
        self.monsters = []
        for _ in range(c):
            flag = b[p + 1] & 0x80
            self.monsters.append((b[p], b[p + 1] & 0x7f, b[p + 2])); p += 3 + (1 if flag else 0)
        k = s8(b[p]); p += 1
        self.npc_kinds = list(b[p:p + k]); p += k
        c = s8(b[p]); p += 1
        self.npcs = []
        for _ in range(c):
            flag = b[p + 1] & 0x80
            self.npcs.append((b[p], b[p + 1] & 0x7f, b[p + 2])); p += 3 + (1 if flag else 0)
        self.scripts_at = p
        self.trees, p = self._table(b, p, wide=False)
        self.actions, p = self._table(b, p, wide=True)
        c = b[p]; p += 1
        lens = list(b[p:p + c]); p += c
        self.extra = []
        for n in lens:
            self.extra.append(b[p:p + n]); p += n + 1
        self.end = p
        self.size = len(b)

    @staticmethod
    def _table(b, p, wide):
        count = b[p]; p += 1
        lens = []
        for _ in range(count):
            if wide:
                lens.append(b[p] | (b[p + 1] << 8)); p += 2
            else:
                lens.append(b[p]); p += 1
        out = []
        for n in lens:
            out.append(b[p:p + n]); p += n
        return out, p

    def triggers(self):
        """Trigger number -> the floor tiles that carry it."""
        out = {}
        for i, a in enumerate(self.attrs):
            t = a & 63
            if t:
                out.setdefault(t, []).append((i % self.w, i // self.w))
        return out


def walk_tree(code, problems, where):
    """Action-list indices a condition tree can queue, and whether it parsed to its end."""
    lists, p = [], 0
    while p < len(code):
        b = s8(code[p]); p += 1
        if b >= 0:
            lists.append(b)
        elif b == -1:
            k = code[p]; p += 1
            lens = list(code[p:p + k]); p += k
            for n in lens:
                lists += walk_tree(code[p:p + n], problems, where + '/branch')
                p += n
        else:
            op = -2 - b
            if op >= len(COND_SIZE):
                problems.append(f'{where}: condition {op} unknown at {p - 1}')
                return lists
            p += COND_SIZE[op]
    if p != len(code):
        problems.append(f'{where}: tree overran by {p - len(code)}')
    return lists


def walk_actions(code, problems, where):
    """Every action (op, operand bytes) in a list, choice bodies included."""
    out, p = [], 0
    while p < len(code):
        op = s8(code[p]); p += 1
        if op == -1:
            p += 1
            k = code[p]; p += 1
            pairs = [(code[p + 2 * i], code[p + 2 * i + 1]) for i in range(k)]; p += 2 * k
            for _, n in pairs:
                out += walk_actions(code[p:p + n], problems, where + '/choice')
                p += n
            continue
        if op < 0 or op >= len(ACT_SIZE):
            problems.append(f'{where}: action {op} unknown at {p - 1}')
            return out
        out.append((op, bytes(code[p:p + ACT_SIZE[op]])))
        p += ACT_SIZE[op]
    if p != len(code):
        problems.append(f'{where}: list overran by {p - len(code)}')
    return out


def load(jar):
    z = zipfile.ZipFile(jar)
    names = set(z.namelist())
    maps = {}
    for n in range(256):
        path = map_path(n)
        if path in names:
            maps[n] = Map(n, z.read(path))
    return maps


def exits(m, problems):
    """(trigger, tiles, dest map, x, y) for every warp a step trigger can run."""
    out = []
    for t, tiles in sorted(m.triggers().items()):
        if t >= len(m.trees):
            problems.append(f'map {m.number}: trigger {t} has no tree ({len(m.trees)})')
            continue
        for li in walk_tree(m.trees[t], problems, f'map {m.number} tree {t}'):
            if li >= len(m.actions):
                continue
            for op, arg in walk_actions(m.actions[li], problems, f'map {m.number} list {li}'):
                if op == WARP and arg[0] != m.number:
                    out.append((t, tiles, arg[0], arg[1], arg[2]))
    return out


def main(argv):
    # `python -I` ignores PYTHONIOENCODING, and a Windows console's code page mangles the names.
    sys.stdout.reconfigure(encoding='utf-8')
    if len(argv) < 2:
        print(__doc__)
        return 2
    maps = load(argv[1])
    flags = argv[2:]
    problems = []
    for m in maps.values():
        if m.end != m.size:
            problems.append(f'map {m.number}: parsed {m.end} of {m.size} bytes')
        for i, code in enumerate(m.trees):
            walk_tree(code, problems, f'map {m.number} tree {i}')
        for i, code in enumerate(m.actions):
            walk_actions(code, problems, f'map {m.number} list {i}')
    if '--check' in flags:
        for p in problems:
            print(p)
        print(f'{len(maps)} maps, {len(problems)} problems')
        return 1 if problems else 0
    if '--map' in flags:
        m = maps[int(flags[flags.index('--map') + 1])]
        print(f'map {m.number} {m.name!r} {m.name2!r} kind {m.kind} ac {m.ac} {m.w}x{m.h} file {m.size} B')
        print(f'  objects {[len(o) for o in m.objects]} monsters {len(m.monsters)} kinds {m.monster_kinds} npcs {len(m.npcs)}')
        for t, tiles in sorted(m.triggers().items()):
            print(f'  trigger {t}: {tiles[:6]}{" …" if len(tiles) > 6 else ""} tree {m.trees[t].hex(" ") if t < len(m.trees) else None}')
        for i, code in enumerate(m.actions):
            acts = walk_actions(code, [], '')
            print(f'  list {i}: {" ".join(f"{op}:{a.hex()}" for op, a in acts)}')
        return 0
    if '--exits' in flags:
        for n, m in sorted(maps.items()):
            for t, tiles, dest, x, y in exits(m, problems):
                d = maps.get(dest)
                print(f'{n:3d} {m.name:<16s} t{t:<2d} {tiles[0]} -> {dest:3d} {d.name if d else "?":<16s} ({x},{y})')
        return 0
    for n, m in sorted(maps.items()):
        print(f'{n:3d} {m.name:<18s} {m.w:3d}x{m.h:<3d} triggers {sorted(m.triggers())}')
    print(f'{len(maps)} maps, {len(problems)} parse problems (--check lists them)')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
