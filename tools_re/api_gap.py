"""Which platform methods does the game call that the emulator does not implement?

usage:  python -I tools_re/api_gap.py <game.jar> [--all]

Reads every class file in the jar, collects each method/field reference whose owner
is NOT a class in the jar (the platform: java.*, javax.microedition.*, ...), and checks
it against the Java class protos the emulator defines in Rust — rustjava's
`java_runtime` and wie's `wie_midp` (+ `wie_jvm_support`), found by scanning
`JavaMethodProto::new*("name", "descriptor", ...)` and `JavaFieldProto::new(...)`
under each `name: "pkg/Class"`. Resolution walks `parent_class` and `interfaces`, as
the JVM would.

This is a static census, not proof: a method listed as present can still be a stub
that does the wrong thing, and a reference in dead code is listed as missing although
it never runs. It exists so that the boot loop does not discover gaps one
NoSuchMethodError per build. Exit 0 = no gaps, 1 = gaps listed.
"""

import re
import struct
import sys
import zipfile
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SOURCES = [
    REPO / "vendor/rustjava/java_runtime/src/classes",
    REPO / "vendor/wie/wie_midp/src/classes",
    REPO / "vendor/wie/wie_jvm_support/src",
]

CLASS_RE = re.compile(r'name:\s*"([\w/$]+)"\s*,\s*parent_class:\s*(?:Some\("([\w/$]+)"\)|None)', re.S)
IFACE_RE = re.compile(r'interfaces:\s*vec!\[([^\]]*)\]', re.S)
METHOD_RE = re.compile(r'JavaMethodProto::new(?:_abstract|_native)?\(\s*"([^"]+)"\s*,\s*"([^"]+)"', re.S)
FIELD_RE = re.compile(r'JavaFieldProto::new\(\s*"([^"]+)"\s*,\s*"([^"]+)"', re.S)


def scan_protos():
    classes = {}
    for root in SOURCES:
        for rs in root.rglob("*.rs"):
            text = rs.read_text(encoding="utf-8", errors="replace")
            m = CLASS_RE.search(text)
            if not m:
                continue
            name, parent = m.group(1), m.group(2)
            im = IFACE_RE.search(text)
            ifaces = re.findall(r'"([\w/$]+)"', im.group(1)) if im else []
            methods = {f"{n}{d}" for n, d in METHOD_RE.findall(text)}
            fields = {f"{n}:{d}" for n, d in FIELD_RE.findall(text)}
            classes[name] = {"parent": parent, "ifaces": ifaces, "methods": methods, "fields": fields, "src": rs}
    return classes


def parse_refs(data):
    n = struct.unpack(">H", data[8:10])[0]
    cp = [None] * n
    i, p = 1, 10
    while i < n:
        tag = data[p]
        if tag == 1:
            ln = struct.unpack(">H", data[p + 1:p + 3])[0]
            cp[i] = ("utf8", data[p + 3:p + 3 + ln].decode("utf-8", "replace"))
            p += 3 + ln
        elif tag in (3, 4):
            p += 5
        elif tag in (5, 6):
            p += 9
            i += 1
        elif tag in (7, 8):
            cp[i] = ("class" if tag == 7 else "string", struct.unpack(">H", data[p + 1:p + 3])[0])
            p += 3
        elif tag in (9, 10, 11, 12):
            a, b = struct.unpack(">HH", data[p + 1:p + 5])
            cp[i] = ({9: "field", 10: "method", 11: "imethod", 12: "nat"}[tag], a, b)
            p += 5
        else:
            raise ValueError(f"cp tag {tag}")
        i += 1
    this_name = cp[cp[struct.unpack(">H", data[p + 2:p + 4])[0]][1]][1]
    refs = []
    for e in cp:
        if e and e[0] in ("field", "method", "imethod"):
            owner = cp[cp[e[1]][1]][1]
            nat = cp[e[2]]
            refs.append((e[0], owner, cp[nat[1]][1], cp[nat[2]][1]))
    return this_name, refs


def resolves(classes, owner, kind, key, seen=None):
    seen = seen if seen is not None else set()
    if owner in seen or owner not in classes:
        return False
    seen.add(owner)
    c = classes[owner]
    if key in (c["fields"] if kind == "field" else c["methods"]):
        return True
    for nxt in ([c["parent"]] if c["parent"] else []) + c["ifaces"]:
        if resolves(classes, nxt, kind, key, seen):
            return True
    # every interface method call can land on java/lang/Object's methods
    return kind != "field" and owner != "java/lang/Object" and resolves(classes, "java/lang/Object", kind, key, seen)


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    show_all = "--all" in sys.argv
    jar = zipfile.ZipFile(sys.argv[1])
    classes = scan_protos()
    own, refs = set(), set()
    for name in jar.namelist():
        if name.endswith(".class"):
            this, r = parse_refs(jar.read(name))
            own.add(this)
            refs.update(r)
    missing_class, missing = set(), defaultdict(list)
    for kind, owner, name, desc in sorted(refs):
        if owner in own or owner.startswith("["):
            continue
        if owner not in classes:
            missing_class.add(owner)
            continue
        key = f"{name}:{desc}" if kind == "field" else f"{name}{desc}"
        ok = resolves(classes, owner, kind, key)
        if not ok or show_all:
            missing[owner].append(("ok " if ok else "-- ") + key)
    print(f"{len(classes)} platform classes implemented; {len(refs)} distinct references from {len(own)} game classes\n")
    if missing_class:
        print("== classes the game references that the emulator does not define ==")
        for c in sorted(missing_class):
            print(f"  {c}")
        print()
    if missing:
        print("== members not found (walking parents + interfaces) ==")
        for owner in sorted(missing):
            print(owner)
            for m in missing[owner]:
                print(f"    {m}")
    gaps = bool(missing_class) or any(m.startswith("--") for v in missing.values() for m in v)
    return 1 if gaps else 0


if __name__ == "__main__":
    sys.exit(main())
