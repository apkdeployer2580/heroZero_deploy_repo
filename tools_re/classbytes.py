"""Print a method's code bytes at given offsets and its StackMap frame offsets — what a
same-length `hz_patches` rewrite needs: the exact original bytes to verify, and proof that no
frame sits inside the span being replaced.

usage:  python -I tools_re/classbytes.py <herozero.jar> <class> <method> <descriptor> [off:len ...]
  e.g.  python -I tools_re/classbytes.py android_app/app/src/main/assets/herozero.jar ek j "(I)V" 0:21

A 3-byte get/put/invoke at an offset is resolved to its constant-pool target. Read-only.
(Written during the 2026-10-22 options RE; see HEROZERO_GAME_OPTIONS_RE.md.)
"""
import sys, struct, zipfile

def u1(b, p): return b[p], p + 1
def u2(b, p): return struct.unpack('>H', b[p:p+2])[0], p + 2
def u4(b, p): return struct.unpack('>I', b[p:p+4])[0], p + 4

def parse(data):
    p = 8
    n, p = u2(data, p)
    cp = [None] * n
    i = 1
    while i < n:
        t = data[p]; p += 1
        if t == 1:
            l, p = u2(data, p); cp[i] = ('utf8', data[p:p+l].decode('utf-8', 'replace')); p += l
        elif t in (3, 4):
            cp[i] = ('int' if t == 3 else 'float', struct.unpack('>i', data[p:p+4])[0]); p += 4
        elif t in (5, 6):
            cp[i] = ('long', data[p:p+8]); p += 8; i += 1
        elif t in (7, 8):
            v, p = u2(data, p); cp[i] = ('class' if t == 7 else 'string', v)
        elif t in (9, 10, 11, 12):
            a, p = u2(data, p); b2, p = u2(data, p); cp[i] = ({9: 'field', 10: 'meth', 11: 'imeth', 12: 'nat'}[t], a, b2)
        else:
            raise ValueError(t)
        i += 1
    p += 6
    ic, p = u2(data, p); p += 2 * ic
    def attrs(p):
        out = []
        c, p = u2(data, p)
        for _ in range(c):
            ni, p = u2(data, p); l, p = u4(data, p)
            out.append((cp[ni][1], p, l)); p += l
        return out, p
    fc, p = u2(data, p)
    for _ in range(fc):
        p += 6; _, p = attrs(p)
    mc, p = u2(data, p)
    methods = []
    for _ in range(mc):
        af, p = u2(data, p); ni, p = u2(data, p); di, p = u2(data, p)
        at, p = attrs(p)
        methods.append((cp[ni][1], cp[di][1], at))
    return cp, methods

def name_of(cp, idx):
    e = cp[idx]
    if e is None: return '?'
    if e[0] in ('field', 'meth', 'imeth'):
        c = cp[cp[e[1]][1]][1]; nat = cp[e[2]]
        return f"{c}.{cp[nat[1]][1]}:{cp[nat[2]][1]}"
    if e[0] == 'class': return cp[e[1]][1]
    return str(e)

def main():
    jar, cls, mname, mdesc = sys.argv[1:5]
    data = zipfile.ZipFile(jar).read(cls + '.class')
    print('version', struct.unpack('>HH', data[4:8]), 'cp_count', len(parse(data)[0]))
    cp, methods = parse(data)
    for nm, ds, at in methods:
        if nm == mname and ds == mdesc:
            for an, ap, al in at:
                if an == 'Code':
                    ms, ml = struct.unpack('>HH', data[ap:ap+4])
                    cl = struct.unpack('>I', data[ap+4:ap+8])[0]
                    code = data[ap+8:ap+8+cl]
                    print(f'{cls}.{nm}{ds}: max_stack={ms} max_locals={ml} code_len={cl}')
                    q = ap + 8 + cl
                    el = struct.unpack('>H', data[q:q+2])[0]; q += 2 + 8 * el
                    ac = struct.unpack('>H', data[q:q+2])[0]; q += 2
                    for _ in range(ac):
                        ni = struct.unpack('>H', data[q:q+2])[0]; l = struct.unpack('>I', data[q+2:q+6])[0]
                        an2 = cp[ni][1]
                        if an2 == 'StackMap':
                            cnt = struct.unpack('>H', data[q+6:q+8])[0]
                            r = q + 8; offs = []
                            for _ in range(cnt):
                                off = struct.unpack('>H', data[r:r+2])[0]; r += 2
                                for _k in range(2):
                                    nn = struct.unpack('>H', data[r:r+2])[0]; r += 2
                                    for _j in range(nn):
                                        t = data[r]; r += 1
                                        if t in (7, 8): r += 2
                                offs.append(off)
                            print('  StackMap frames at', offs)
                        else:
                            print('  attr', an2, l)
                        q += 6 + l
                    for spec in sys.argv[5:]:
                        o, ln = map(int, spec.split(':'))
                        bs = code[o:o+ln]
                        extra = ''
                        if ln == 3 and bs[0] in (0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8):
                            extra = '  -> #%d %s' % (struct.unpack('>H', bs[1:3])[0], name_of(cp, struct.unpack('>H', bs[1:3])[0]))
                        print(f'  @{o}: {bs.hex(" ")}{extra}')
    return

if __name__ == '__main__':
    main()
