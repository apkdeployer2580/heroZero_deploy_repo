//! Just enough of the class file format to find a method's bytecode and rewrite it in place.
//!
//! In place = SAME LENGTH. This game's classes are CLDC-preverified (version 45.3) and carry
//! a `StackMap` attribute whose frames are keyed by bytecode offset; a rewrite that kept
//! every offset where it was cannot invalidate one. Growing or shrinking code would mean
//! re-encoding branches, the exception table and the stack map — not something to do
//! by hand, so this module does not offer it.
//!
//! The one thing that may GROW is the constant pool, and only at its end ([`methodref`]):
//! every existing index keeps its meaning, and nothing in a class file addresses bytes past
//! the pool absolutely (attributes carry their own lengths), so no instruction, attribute
//! or frame needs touching. That is what lets a 3-byte `invoke*`/`getfield` become a 3-byte
//! `invokestatic` of a host hook.

use std::ops::Range;

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.d.get(self.p)?;
        self.p += 1;
        Some(v)
    }

    fn u16(&mut self) -> Option<u16> {
        let b = self.d.get(self.p..self.p + 2)?;
        self.p += 2;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.d.get(self.p..self.p + 4)?;
        self.p += 4;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        (self.p + n <= self.d.len()).then(|| self.p += n)
    }
}

/// The byte range of the `Code` of method `name` + `descriptor` (the bytecode itself, not
/// the attribute header), or `None` if the class does not parse or has no such method.
pub fn method_code(class: &[u8], name: &str, descriptor: &str) -> Option<Range<usize>> {
    let mut r = Reader { d: class, p: 0 };
    (r.u32()? == 0xCAFE_BABE).then_some(())?;
    r.skip(4)?; // minor, major

    let count = r.u16()?;
    let mut utf8: Vec<Option<&[u8]>> = vec![None; count as usize];
    let mut i = 1;
    while i < count {
        match r.u8()? {
            1 => {
                let len = r.u16()? as usize;
                utf8[i as usize] = Some(class.get(r.p..r.p + len)?);
                r.skip(len)?;
            }
            3 | 4 => r.skip(4)?,
            5 | 6 => {
                r.skip(8)?;
                i += 1; // two slots
            }
            7 | 8 | 16 => r.skip(2)?,
            9 | 10 | 11 | 12 | 18 => r.skip(4)?,
            15 => r.skip(3)?,
            _ => return None,
        }
        i += 1;
    }
    let utf = |idx: u16| utf8.get(idx as usize).copied().flatten();

    r.skip(6)?; // access, this, super
    let interfaces = r.u16()? as usize;
    r.skip(interfaces * 2)?;

    // fields: skipped whole
    for _ in 0..r.u16()? {
        r.skip(6)?;
        for _ in 0..r.u16()? {
            r.skip(2)?;
            let len = r.u32()? as usize;
            r.skip(len)?;
        }
    }

    for _ in 0..r.u16()? {
        r.skip(2)?;
        let (n, d) = (r.u16()?, r.u16()?);
        let wanted = utf(n)? == name.as_bytes() && utf(d)? == descriptor.as_bytes();
        for _ in 0..r.u16()? {
            let attr = r.u16()?;
            let len = r.u32()? as usize;
            if wanted && utf(attr)? == b"Code" {
                let start = r.p + 8; // max_stack u2, max_locals u2, code_length u4
                let code_len = u32::from_be_bytes(class.get(r.p + 4..r.p + 8)?.try_into().ok()?) as usize;
                return (start + code_len <= class.len()).then_some(start..start + code_len);
            }
            r.skip(len)?;
        }
    }
    None
}

/// Replace `expect` with `with` at bytecode offset `at` of one method — only if the bytes
/// there are exactly `expect` (and the two are the same length). Anything else is `None`,
/// and the caller defines the original class.
pub fn rewrite_code(class: &[u8], name: &str, descriptor: &str, at: usize, expect: &[u8], with: &[u8]) -> Option<Vec<u8>> {
    (expect.len() == with.len()).then_some(())?;
    let code = method_code(class, name, descriptor)?;
    let start = code.start + at;
    (start + expect.len() <= code.end).then_some(())?;
    (&class[start..start + expect.len()] == expect).then_some(())?;
    let mut out = class.to_vec();
    out[start..start + with.len()].copy_from_slice(with);
    Some(out)
}

/// One constant-pool entry, as much of it as [`methodref`] needs to find an existing one.
enum Entry<'a> {
    Utf8(&'a [u8]),
    Class(u16),
    NameAndType(u16, u16),
    Methodref(u16, u16),
    Other,
}

/// The pool's entries (index 0 and the second slot of a long/double are `Other`) and the
/// byte offset just past the last one.
fn constant_pool(class: &[u8]) -> Option<(Vec<Entry<'_>>, usize)> {
    let mut r = Reader { d: class, p: 0 };
    (r.u32()? == 0xCAFE_BABE).then_some(())?;
    r.skip(4)?;
    let count = r.u16()?;
    let mut entries = Vec::with_capacity(count as usize);
    entries.push(Entry::Other);
    while entries.len() < count as usize {
        let e = match r.u8()? {
            1 => {
                let len = r.u16()? as usize;
                let s = class.get(r.p..r.p + len)?;
                r.skip(len)?;
                Entry::Utf8(s)
            }
            7 => Entry::Class(r.u16()?),
            12 => Entry::NameAndType(r.u16()?, r.u16()?),
            10 => Entry::Methodref(r.u16()?, r.u16()?),
            3 | 4 | 9 | 11 | 18 => r.skip(4).map(|_| Entry::Other)?,
            8 | 16 => r.skip(2).map(|_| Entry::Other)?,
            15 => r.skip(3).map(|_| Entry::Other)?,
            5 | 6 => {
                r.skip(8)?;
                entries.push(Entry::Other);
                Entry::Other
            }
            _ => return None,
        };
        entries.push(e);
    }
    (entries.len() == count as usize).then_some((entries, r.p))
}

/// Entries appended to a pool: the ORIGINAL entries to search, the bytes to add and the next
/// free index. A freshly appended entry can only be referenced by another fresh one, so
/// searching the original pool alone is right at every step.
struct Appender<'a> {
    entries: Vec<Entry<'a>>,
    added: Vec<u8>,
    next: usize,
}

impl Appender<'_> {
    fn find(&self, want: impl Fn(&Entry) -> bool) -> Option<u16> {
        self.entries.iter().position(want).map(|i| i as u16)
    }

    fn push(&mut self, bytes: &[u8]) -> Option<u16> {
        // 0xFFFF is not a valid index (the count field is u16 and counts index 0).
        let i = u16::try_from(self.next).ok().filter(|&i| i < u16::MAX)?;
        self.added.extend_from_slice(bytes);
        self.next += 1;
        Some(i)
    }

    fn utf8(&mut self, s: &str) -> Option<u16> {
        // Modified UTF-8 = plain UTF-8 for the ASCII names hooks use; refuse anything else
        // rather than encode it subtly wrong.
        s.is_ascii().then_some(())?;
        if let Some(i) = self.find(|e| matches!(e, Entry::Utf8(b) if *b == s.as_bytes())) {
            return Some(i);
        }
        let len = u16::try_from(s.len()).ok()?;
        self.push(&[&[1u8][..], &len.to_be_bytes(), s.as_bytes()].concat())
    }

    fn class(&mut self, name: u16) -> Option<u16> {
        match self.find(|e| matches!(e, Entry::Class(n) if *n == name)) {
            Some(i) => Some(i),
            None => self.push(&[&[7u8][..], &name.to_be_bytes()].concat()),
        }
    }

    fn name_and_type(&mut self, name: u16, desc: u16) -> Option<u16> {
        match self.find(|e| matches!(e, Entry::NameAndType(n, d) if *n == name && *d == desc)) {
            Some(i) => Some(i),
            None => self.push(&[&[12u8][..], &name.to_be_bytes(), &desc.to_be_bytes()].concat()),
        }
    }

    fn methodref(&mut self, class: u16, nat: u16) -> Option<u16> {
        match self.find(|e| matches!(e, Entry::Methodref(c, n) if *c == class && *n == nat)) {
            Some(i) => Some(i),
            None => self.push(&[&[10u8][..], &class.to_be_bytes(), &nat.to_be_bytes()].concat()),
        }
    }
}

/// The constant-pool index of `Methodref owner.name:descriptor`, reusing whatever entries the
/// class already has and APPENDING the missing ones (Utf8, Class, NameAndType, Methodref) at
/// the end of the pool. Returns the new class bytes (unchanged if nothing was missing) and
/// the index. `None` if the class does not parse or the pool would overflow.
pub fn methodref(class: &[u8], owner: &str, name: &str, descriptor: &str) -> Option<(Vec<u8>, u16)> {
    let (entries, end) = constant_pool(class)?;
    let next = entries.len();
    let mut a = Appender { entries, added: Vec::new(), next };

    let owner = a.utf8(owner)?;
    let name = a.utf8(name)?;
    let descriptor = a.utf8(descriptor)?;
    let class_i = a.class(owner)?;
    let nat = a.name_and_type(name, descriptor)?;
    let index = a.methodref(class_i, nat)?;

    let mut out = Vec::with_capacity(class.len() + a.added.len());
    out.extend_from_slice(&class[..8]);
    out.extend((a.next as u16).to_be_bytes());
    out.extend_from_slice(&class[10..end]);
    out.extend_from_slice(&a.added);
    out.extend_from_slice(&class[end..]);
    Some((out, index))
}

/// Make the 3-byte instruction-or-operand run at `at` a call of a host hook: check `expect`
/// on the ORIGINAL bytes first (so a mismatch costs nothing and changes nothing), then add
/// `Methodref owner.name:descriptor` to the pool and write `with(index)` — which must be
/// `expect.len()` bytes and is where the caller places `invokestatic` (0xb8) and the index.
#[allow(clippy::too_many_arguments)]
pub fn rewrite_with_methodref(
    class: &[u8],
    name: &str,
    descriptor: &str,
    at: usize,
    expect: &[u8],
    hook: (&str, &str, &str),
    with: impl Fn(u16) -> Vec<u8>,
) -> Option<Vec<u8>> {
    let code = method_code(class, name, descriptor)?;
    (class.get(code.start + at..code.start + at + expect.len())? == expect).then_some(())?;
    let (class, index) = methodref(class, hook.0, hook.1, hook.2)?;
    rewrite_code(&class, name, descriptor, at, expect, &with(index))
}

#[cfg(test)]
pub(crate) mod tests {
    /// A minimal, valid class file `name` extending Object, with one method `m` + `desc`
    /// whose code is `code`. Enough to exercise the walker without the game's own classes
    /// (which never enter this repository).
    pub fn synth_class(name: &str, m: &str, desc: &str, code: &[u8]) -> Vec<u8> {
        synth_class_n(name, &[(m, desc, code)])
    }

    /// [`synth_class`] with several methods. Pool: #1 `name`, #2 its Class, #3 Object, #4 its
    /// Class, then each method's name and descriptor (#5, #6, #7, ...), then "Code".
    pub fn synth_class_n(name: &str, methods: &[(&str, &str, &[u8])]) -> Vec<u8> {
        let mut c = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 3, 0, 45];
        let utf = |c: &mut Vec<u8>, s: &str| {
            c.push(1);
            c.extend((s.len() as u16).to_be_bytes());
            c.extend(s.as_bytes());
        };
        let code_utf = 5 + 2 * methods.len() as u16;
        c.extend((code_utf + 1).to_be_bytes());
        utf(&mut c, name); // #1
        c.extend([7, 0, 1]); // #2 Class #1
        utf(&mut c, "java/lang/Object"); // #3
        c.extend([7, 0, 3]); // #4 Class #3
        for (m, desc, _) in methods {
            utf(&mut c, m);
            utf(&mut c, desc);
        }
        utf(&mut c, "Code");
        c.extend([0, 0x30, 0, 2, 0, 4, 0, 0, 0, 0]); // access, this, super, no interfaces, no fields
        c.extend((methods.len() as u16).to_be_bytes());
        for (i, (_, _, code)) in methods.iter().enumerate() {
            let n = 5 + 2 * i as u16;
            c.extend([0, 0x10]);
            c.extend(n.to_be_bytes());
            c.extend((n + 1).to_be_bytes());
            c.extend([0, 1]); // one attribute: Code
            c.extend(code_utf.to_be_bytes());
            c.extend((12 + code.len() as u32).to_be_bytes());
            c.extend([0, 4, 0, 3]);
            c.extend((code.len() as u32).to_be_bytes());
            c.extend(*code);
            c.extend([0, 0, 0, 0]); // no exception table, no code attributes
        }
        c.extend([0, 0]); // no class attributes
        c
    }

    #[test]
    fn finds_code_and_rewrites_only_what_it_expects() {
        let code = [0x2a, 0x2b, 0x60, 0xac];
        let class = synth_class("t", "f", "(II)I", &code);
        let r = super::method_code(&class, "f", "(II)I").unwrap();
        assert_eq!(&class[r], &code);
        assert_eq!(super::method_code(&class, "f", "()V"), None);
        let out = super::rewrite_code(&class, "f", "(II)I", 2, &[0x60], &[0x64]).unwrap();
        assert_eq!(out.len(), class.len());
        assert_eq!(&out[super::method_code(&out, "f", "(II)I").unwrap()], &[0x2a, 0x2b, 0x64, 0xac]);
        assert_eq!(super::rewrite_code(&class, "f", "(II)I", 2, &[0x61], &[0x64]), None, "unexpected bytes");
        assert_eq!(super::rewrite_code(&class, "f", "(II)I", 3, &[0xac, 0], &[0, 0]), None, "past the end");
        assert_eq!(super::rewrite_code(&class, "f", "(II)I", 0, &[0x2a], &[0, 0]), None, "length change");
    }

    /// Resolve a Methodref the way a JVM would: index -> (class name, name, descriptor).
    pub fn resolve(class: &[u8], index: u16) -> (String, String, String) {
        let (entries, _) = super::constant_pool(class).unwrap();
        let utf = |i: u16| match entries[i as usize] {
            super::Entry::Utf8(b) => String::from_utf8(b.to_vec()).unwrap(),
            _ => panic!("#{i} is not Utf8"),
        };
        let super::Entry::Methodref(c, nat) = entries[index as usize] else { panic!("#{index} is not a Methodref") };
        let super::Entry::Class(cn) = entries[c as usize] else { panic!("#{c} is not a Class") };
        let super::Entry::NameAndType(n, d) = entries[nat as usize] else { panic!("#{nat} is not a NameAndType") };
        (utf(cn), utf(n), utf(d))
    }

    #[test]
    fn methodref_appends_and_keeps_every_existing_index() {
        let code = [0x2a, 0xb6, 0x00, 0x05, 0xac];
        let class = synth_class("t", "f", "(II)I", &code);
        let (out, i) = super::methodref(&class, "net/wie/hz/Hooks", "exp", "(I)I").unwrap();
        assert_eq!(i, 13, "8 original slots, then Utf8 x3 + Class + NameAndType -> Methodref is #13");
        assert_eq!(resolve(&out, i), ("net/wie/hz/Hooks".into(), "exp".into(), "(I)I".into()));
        // The method is still found (its name/descriptor indices are unchanged) with the same code.
        assert_eq!(&out[super::method_code(&out, "f", "(II)I").unwrap()], &code);
        assert_eq!(out.len(), class.len() + (3 + 16) + (3 + 3) + (3 + 4) + 3 + 5 + 5);

        // Asking again finds everything and adds nothing.
        let (again, j) = super::methodref(&out, "net/wie/hz/Hooks", "exp", "(I)I").unwrap();
        assert_eq!((again.as_slice(), j), (out.as_slice(), i));
        // A second hook on the same owner reuses its Utf8 + Class.
        let (two, k) = super::methodref(&out, "net/wie/hz/Hooks", "drop", "(I)I").unwrap();
        assert_eq!(k, 16, "only Utf8 'drop' + NameAndType + Methodref are new; '(I)I' is reused");
        assert_eq!(resolve(&two, k).1, "drop");
        assert_eq!(resolve(&two, i).1, "exp", "the first Methodref still means what it did");
        // Existing Utf8 and Class entries are reused too ("t" #1 / Class #2, "f" #5, "(II)I" #6):
        // only a NameAndType and the Methodref are new.
        let (own, l) = super::methodref(&class, "t", "f", "(II)I").unwrap();
        assert_eq!(resolve(&own, l), ("t".into(), "f".into(), "(II)I".into()));
        assert_eq!(own.len(), class.len() + 5 + 5);
    }

    #[test]
    fn rewrite_with_methodref_checks_the_original_first() {
        let code = [0x2a, 0x1b, 0xb6, 0x00, 0x05, 0xac];
        let class = synth_class("t", "f", "(I)I", &code);
        let hook = ("net/wie/hz/Hooks", "h", "(Ljava/lang/Object;I)I");
        let out = super::rewrite_with_methodref(&class, "f", "(I)I", 2, &[0xb6, 0x00, 0x05], hook, |i| {
            let [hi, lo] = i.to_be_bytes();
            vec![0xb8, hi, lo]
        })
        .unwrap();
        let new = &out[super::method_code(&out, "f", "(I)I").unwrap()];
        assert_eq!(new[2], 0xb8);
        assert_eq!(resolve(&out, u16::from_be_bytes([new[3], new[4]])), (hook.0.into(), hook.1.into(), hook.2.into()));
        // Wrong bytes: nothing at all is added, not even to the pool.
        assert_eq!(super::rewrite_with_methodref(&class, "f", "(I)I", 1, &[0xb6, 0x00, 0x05], hook, |_| vec![0; 3]), None);
        // A `with` of the wrong length is refused by the same-length rule.
        assert_eq!(super::rewrite_with_methodref(&class, "f", "(I)I", 2, &[0xb6, 0x00, 0x05], hook, |_| vec![0; 2]), None);
    }
}
