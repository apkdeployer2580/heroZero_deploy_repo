//! A map file read the way the game reads it, down to the warps its step triggers run (Phase 7-6:
//! the world map follows the game's own quest guidance from map to map). The Rust twin of
//! `tools_re/hz_maps.py` — the format, the script tables and the operand sizes are documented
//! there and in HEROZERO_REMAKE_PLAN.md §5.6; that tool checks all 139 maps parse to their ends.

/// Operand bytes per condition (`eb.a(BLn;)Z`) and action (`eb.b(BLn;)Z`) opcode.
const COND_SIZE: [usize; 11] = [2, 2, 2, 3, 2, 3, 2, 3, 1, 1, 0];
const ACT_SIZE: [usize; 59] = [
    1, 2, 2, 2, 1, 1, 2, 1, 0, 1, 1, 2, 9, 2, 1, 2, 9, 6, 0, 1, 6, 6, 0, 4, 3, 2, 2, 2, 3, 2, 2, 4, 2, 2, 0, 2, 0, 3, 0, 0, 0, 0, 1, 3, 1, 1,
    2, 1, 2, 0, 2, 0, 1, 2, 2, 3, 3, 1, 4,
];
/// Action 23: warp to `[map, x, y, facing]`.
const WARP: u8 = 23;

/// One step-trigger warp: standing on any of `tiles` (trigger `trigger`) can take the player to
/// map `dest` (a trigger with story branches lists one warp per branch).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    pub trigger: u8,
    pub tiles: Vec<(u8, u8)>,
    pub dest: u8,
    pub at: (u8, u8),
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.p)?;
        self.p += 1;
        Some(v)
    }

    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }

    /// `n` entries of `size` bytes, each with a flag in its second byte (`& 0x80`) that adds one.
    fn flagged(&mut self, count: usize, size: usize) -> Option<()> {
        for _ in 0..count {
            let flag = *self.b.get(self.p + 1)? & 0x80 != 0;
            self.take(size + usize::from(flag))?;
        }
        Some(())
    }

    /// A script table (`cp.a([BII)I`): count, lengths (u8, or u16 LE when `wide`), the bytes.
    fn table(&mut self, wide: bool) -> Option<Vec<Vec<u8>>> {
        let count = usize::from(self.u8()?);
        let mut lens = Vec::with_capacity(count);
        for _ in 0..count {
            lens.push(if wide { usize::from(self.u8()?) | usize::from(self.u8()?) << 8 } else { usize::from(self.u8()?) });
        }
        lens.into_iter().map(|n| self.take(n).map(<[u8]>::to_vec)).collect()
    }
}

/// A map file's layout and step-trigger warps.
#[derive(Debug)]
pub struct MapFile {
    pub w: u8,
    pub h: u8,
    /// Whether the file places monsters — kinds or spots (`hz_maps`: the 72 maps with either, 20
    /// tiles wide and up). The game's target panel is drawn over a monster, so only over these.
    pub fights: bool,
    attrs: Vec<u8>,
    trees: Vec<Vec<u8>>,
    actions: Vec<Vec<u8>>,
}

impl MapFile {
    /// `g.a([B)V`'s order (see `tools_re/hz_maps.py`); `None` when the bytes end early.
    pub fn parse(b: &[u8]) -> Option<MapFile> {
        let mut r = Reader { b, p: 3 };
        for _ in 0..3 {
            let n = r.u8()? as i8;
            r.take(n.max(0) as usize)?;
        }
        let (w, h) = (r.u8()?, r.u8()?);
        let n = usize::from(w) * usize::from(h);
        r.take(n)?;
        let attrs = r.take(n)?.to_vec();
        for _ in 0..3 {
            let c = usize::from(r.u8()?);
            r.take(5 * c)?;
        }
        let kinds = r.u8()? as i8;
        r.take(kinds.max(0) as usize)?;
        let spots = r.u8()? as i8;
        r.flagged(spots.max(0) as usize, 3)?;
        let k = r.u8()? as i8;
        r.take(k.max(0) as usize)?;
        let c = r.u8()? as i8;
        r.flagged(c.max(0) as usize, 3)?;
        let trees = r.table(false)?;
        let actions = r.table(true)?;
        Some(MapFile { w, h, fights: kinds > 0 || spots > 0, attrs, trees, actions })
    }

    /// The trigger on tile `(x, y)` (`& 63`, 0 = none).
    pub fn trigger_at(&self, x: u8, y: u8) -> u8 {
        if x >= self.w || y >= self.h {
            return 0;
        }
        self.attrs[usize::from(y) * usize::from(self.w) + usize::from(x)] & 63
    }

    /// Every warp to ANOTHER map a step trigger can run — every branch of its condition tree and
    /// every choice of its action lists (`tools_re/hz_maps.py --exits`).
    pub fn exits(&self, own: u8) -> Vec<Exit> {
        let mut tiles: Vec<Vec<(u8, u8)>> = vec![Vec::new(); 64];
        for (i, &a) in self.attrs.iter().enumerate() {
            let t = a & 63;
            if t != 0 {
                tiles[usize::from(t)].push(((i % usize::from(self.w)) as u8, (i / usize::from(self.w)) as u8));
            }
        }
        let mut out = Vec::new();
        for (t, at) in tiles.iter().enumerate().filter(|(_, v)| !v.is_empty()) {
            let Some(tree) = self.trees.get(t) else { continue };
            for list in tree_lists(tree) {
                let Some(code) = self.actions.get(usize::from(list)) else { continue };
                for (op, arg) in list_actions(code) {
                    if op == WARP && arg.len() == 4 && arg[0] != own {
                        out.push(Exit { trigger: t as u8, tiles: at.clone(), dest: arg[0], at: (arg[1], arg[2]) });
                    }
                }
            }
        }
        out
    }

    /// The trigger numbers that can take the player to another map, ascending — the doors the
    /// field's minimap draws ([`crate::MapGrid::with_leaving`]); every other trigger is a shop
    /// counter, a bed, a shelf, an event spot.
    pub fn leaving(&self, own: u8) -> Vec<u8> {
        let mut t: Vec<u8> = self.exits(own).iter().map(|e| e.trigger).collect();
        t.dedup();
        t
    }

    /// The maps a guide tile points the player to: the warps of the step trigger under it, every
    /// story branch, in the file's order. A tile with no trigger is the game's 「!」 drawn just
    /// inside a door (실바나르 남부's (5,48) for quest 13 on, above the row that leads to
    /// 소버린구릉지): then the first neighbour, straight before diagonal, whose trigger leaves.
    pub fn guide_exits(&self, own: u8, (x, y): (u8, u8)) -> Vec<u8> {
        let exits = self.exits(own);
        let mut trigger = self.trigger_at(x, y);
        if trigger == 0 {
            const NEAR: [(i16, i16); 8] = [(0, 1), (0, -1), (-1, 0), (1, 0), (-1, -1), (1, -1), (-1, 1), (1, 1)];
            trigger = NEAR
                .iter()
                .filter_map(|&(dx, dy)| Some(self.trigger_at(u8::try_from(i16::from(x) + dx).ok()?, u8::try_from(i16::from(y) + dy).ok()?)))
                .find(|&t| t != 0 && exits.iter().any(|e| e.trigger == t))
                .unwrap_or(0);
        }
        let mut out: Vec<u8> = Vec::new();
        for e in exits.iter().filter(|e| trigger != 0 && e.trigger == trigger) {
            if !out.contains(&e.dest) {
                out.push(e.dest);
            }
        }
        out
    }
}

/// Where quest `q`'s guidance ends, followed from map `start`: from each map's guide tile through
/// the door it points at ([`MapFile::guide_exits`]) to the first map with no guide tile for `q` —
/// the NPC or spot itself. Depth first, never revisiting a map; at a door with story branches the
/// ones whose map guides on are tried before the ones that stop (남부관문's door to the event
/// version of 습지대 and to 습지대 itself), so a loop (실바나르 ↔ 실바나르 남부 on quest 2) or an
/// event map does not end the trail early. Over this jar's 139 maps every quest ends in ONE map
/// from every map that guides it (2026-10-24). `None` when `start` has no guide tile for `q` or
/// every branch loops. Data comes from the caller's cache: `Err(map)` = load that map's `/mmi/`
/// table and file, then ask again.
pub fn guide_target(
    start: u8,
    q: u8,
    mmi: impl Fn(u8) -> Result<Option<std::sync::Arc<[u8]>>, u8>,
    file: impl Fn(u8) -> Result<Option<std::sync::Arc<MapFile>>, u8>,
) -> Result<Option<u8>, u8> {
    let tile = |m: u8| -> Result<Option<(u8, u8)>, u8> { Ok(mmi(m)?.and_then(|t| guide_tile(&t, q))) };
    let doors = |m: u8| -> Result<Vec<u8>, u8> {
        let Some(t) = tile(m)? else { return Ok(Vec::new()) };
        let Some(f) = file(m)? else { return Ok(Vec::new()) };
        let mut on = Vec::new();
        let mut stop = Vec::new();
        for d in f.guide_exits(m, t) {
            if tile(d)?.is_some() { on.push(d) } else { stop.push(d) }
        }
        on.extend(stop);
        Ok(on)
    };
    if tile(start)?.is_none() {
        return Ok(None);
    }
    let mut seen = vec![start];
    let mut stack = vec![(doors(start)?, 0usize)];
    while let Some((next, i)) = stack.last_mut() {
        let Some(&d) = next.get(*i) else {
            stack.pop();
            continue;
        };
        *i += 1;
        if seen.contains(&d) {
            continue;
        }
        seen.push(d);
        if tile(d)?.is_none() {
            return Ok(Some(d));
        }
        stack.push((doors(d)?, 0));
    }
    Ok(None)
}

/// The action lists a condition tree can queue (`dw.a(n)`): b ≥ 0 a list, −1 a branch block, ≤ −2
/// a condition with its operands.
fn tree_lists(code: &[u8]) -> Vec<u8> {
    let (mut out, mut p) = (Vec::new(), 0);
    while p < code.len() {
        let b = code[p] as i8;
        p += 1;
        if b >= 0 {
            out.push(b as u8);
        } else if b == -1 {
            let Some(&k) = code.get(p) else { break };
            p += 1;
            let lens: Vec<usize> = code.get(p..p + usize::from(k)).map(|s| s.iter().map(|&l| usize::from(l)).collect()).unwrap_or_default();
            p += usize::from(k);
            for n in lens {
                if let Some(sub) = code.get(p..p + n) {
                    out.extend(tree_lists(sub));
                }
                p += n;
            }
        } else {
            let op = (-2 - i32::from(b)) as usize;
            let Some(&size) = COND_SIZE.get(op) else { break };
            p += size;
        }
    }
    out
}

/// Every action of a list (`dw.b()`), choice bodies included: `(op, operands)`.
fn list_actions(code: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let (mut out, mut p) = (Vec::new(), 0);
    while p < code.len() {
        let op = code[p] as i8;
        p += 1;
        if op == -1 {
            p += 1;
            let Some(&k) = code.get(p) else { break };
            p += 1;
            let lens: Vec<usize> = (0..usize::from(k)).filter_map(|i| code.get(p + 2 * i + 1).map(|&l| usize::from(l))).collect();
            p += 2 * usize::from(k);
            for n in lens {
                if let Some(sub) = code.get(p..p + n) {
                    out.extend(list_actions(sub));
                }
                p += n;
            }
            continue;
        }
        let Some(&size) = usize::try_from(op).ok().and_then(|o| ACT_SIZE.get(o)) else { break };
        if let Some(arg) = code.get(p..p + size) {
            out.push((op as u8, arg.to_vec()));
        }
        p += size;
    }
    out
}

/// The quest the game guides toward (`g.a()V`, the `/mmi/` reader): from quest 35 down, the first
/// whose state (`ed.a(I)B`, two bits each) is 1 = under way; a 2 = done met first means nothing is
/// under way (the quests run in order). `state(q)` is the game's.
pub fn guided_quest(state: impl Fn(u8) -> u8) -> Option<u8> {
    for q in (1..=35u8).rev() {
        match state(q) {
            1 => return Some(q),
            2 => return None,
            _ => {}
        }
    }
    None
}

/// The guide tile `/mmi/<map>` holds for quest `q` (70 B = 35 × (x, y), `ff ff` = none).
pub fn guide_tile(mmi: &[u8], q: u8) -> Option<(u8, u8)> {
    let i = usize::from(q.checked_sub(1)?) * 2;
    match (mmi.get(i), mmi.get(i + 1)) {
        (Some(&0xff), Some(&0xff)) | (None, _) | (_, None) => None,
        (Some(&x), Some(&y)) => Some((x, y)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic map: 3 × 2, a trigger 1 on tile (2, 0), its tree queueing list 0 behind one
    /// condition, list 0 = a choice whose second body warps to map 7 (and one that steps in place).
    fn file() -> Vec<u8> {
        let mut f = vec![9, 0, 0x1a, 0, 0, 1, 0, 3, 2];
        f.extend([0u8; 6]); // tiles
        f.extend([0, 0, 1, 0, 0x80, 0]); // attributes
        f.extend([0, 0, 0]); // three object lists
        f.extend([0, 0, 0, 0]); // monster kinds, spots, NPC kinds, NPCs
        // Table A: tree 0 empty, tree 1 = condition 8 (1 operand), then list 0.
        f.extend([2, 0, 3, 0xf6, 5, 0]);
        // Table B: list 0 = choice (-1, x, 2 bodies: lens 5 and 5): warp(own 44) / warp(7).
        let list = [0xff, 0, 2, 0, 5, 0, 5, WARP, 44, 1, 1, 0, WARP, 7, 3, 4, 2];
        f.extend([1, list.len() as u8, 0]);
        f.extend(list);
        f.extend([0]); // the short-strings table
        f
    }

    #[test]
    fn a_step_trigger_warp_is_found_through_its_tree_and_choice() {
        let m = MapFile::parse(&file()).unwrap();
        assert_eq!((m.w, m.h), (3, 2));
        assert_eq!(m.trigger_at(2, 0), 1);
        assert_eq!(m.trigger_at(1, 1), 0, "a blocked tile is no trigger");
        assert_eq!(m.exits(44), vec![Exit { trigger: 1, tiles: vec![(2, 0)], dest: 7, at: (3, 4) }], "the warp to its own map is a step, not an exit");
        assert_eq!(m.leaving(44), vec![1]);
        assert!(!m.fights, "no monster kinds, no spots");
        assert!(MapFile::parse(&file()[..20]).is_none());
    }

    /// A map that places monsters — by kind (a boss room may list its kind and spawn it by
    /// script) or by spot — is one the target panel can be drawn over.
    #[test]
    fn monster_kinds_or_spots_make_a_fight_map() {
        let at = 9 + 6 + 6 + 3; // header, tiles, attributes, object lists
        let with = |monsters: &[u8]| {
            let mut f = file();
            f.splice(at..at + 2, monsters.iter().copied());
            MapFile::parse(&f).unwrap()
        };
        assert!(with(&[1, 5, 0]).fights, "one kind, no spot");
        assert!(with(&[0, 1, 5, 2, 3]).fights, "no kind, one spot");
        let both = with(&[1, 5, 2, 5, 0x81, 3, 9, 5, 4, 6]);
        assert!(both.fights && both.leaving(44) == vec![1], "a flagged spot's extra byte read past: {both:?}");
        assert!(!with(&[0, 0]).fights);
    }

    /// Opt-in parity with `tools_re/hz_maps.py --exits` over the real maps:
    /// `HZ_MAPS_DIR=<dir of NNN.m files> cargo test -p hz_java maps_parity -- --ignored` writes
    /// `<dir>/exits_rust.txt` (`map trigger dest x y` per warp) for a diff against the tool.
    #[test]
    #[ignore]
    fn maps_parity() {
        let Ok(dir) = std::env::var("HZ_MAPS_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        let mut out = String::new();
        let mut parsed = 0;
        for n in 0..=255u8 {
            let Ok(b) = std::fs::read(dir.join(format!("{n:03}.m"))) else { continue };
            let m = MapFile::parse(&b).unwrap_or_else(|| panic!("map {n} does not parse"));
            parsed += 1;
            for e in m.exits(n) {
                out += &format!("{n} {} {} {} {}\n", e.trigger, e.dest, e.at.0, e.at.1);
            }
        }
        std::fs::write(dir.join("exits_rust.txt"), out).unwrap();
        eprintln!("parsed {parsed} maps");
        // With the jar's `/mmi/N` tables there as `mmi_N`: every quest's trail from every map that
        // guides it, `quest start target` per line (`guides_rust.txt`).
        let mmi: std::collections::HashMap<u8, std::sync::Arc<[u8]>> =
            (0..=255u8).filter_map(|n| std::fs::read(dir.join(format!("mmi_{n}"))).ok().map(|b| (n, b.into()))).collect();
        if mmi.is_empty() {
            return;
        }
        let files: std::collections::HashMap<u8, std::sync::Arc<MapFile>> = (0..=255u8)
            .filter_map(|n| std::fs::read(dir.join(format!("{n:03}.m"))).ok().and_then(|b| MapFile::parse(&b)).map(|f| (n, f.into())))
            .collect();
        let mut out = String::new();
        for q in 1..=35u8 {
            for (&s, t) in mmi.iter().collect::<std::collections::BTreeMap<_, _>>() {
                if guide_tile(t, q).is_some() {
                    let target = guide_target(s, q, |m| Ok(mmi.get(&m).cloned()), |m| Ok(files.get(&m).cloned())).unwrap();
                    out += &format!("{q} {s} {target:?}\n");
                }
            }
        }
        std::fs::write(dir.join("guides_rust.txt"), out).unwrap();
    }

    /// A parseable `w` × `h` map: `triggers` put on tiles, and each trigger's list warps to `dests`
    /// (one warp per story branch).
    fn map(w: u8, h: u8, triggers: &[((u8, u8), u8)], warps: &[(u8, &[u8])]) -> MapFile {
        let n = usize::from(w) * usize::from(h);
        let mut attrs = vec![0u8; n];
        for &((x, y), t) in triggers {
            attrs[usize::from(y) * usize::from(w) + usize::from(x)] = t;
        }
        let top = warps.iter().map(|w| w.0).max().unwrap_or(0);
        let mut f = vec![9, 0, 0x1a, 0, 0, 1, 0, w, h];
        f.extend(vec![0u8; n]);
        f.extend(attrs);
        f.extend([0, 0, 0, 0, 0, 0, 0]);
        let lists: Vec<Vec<u8>> = (0..=top)
            .map(|t| warps.iter().filter(|w| w.0 == t).flat_map(|w| w.1.iter().flat_map(|&d| [WARP, d, 1, 1, 0])).collect())
            .collect();
        f.push(top + 1);
        f.extend(std::iter::repeat_n(1u8, usize::from(top) + 1));
        f.extend(0..=top);
        f.push(top + 1);
        for l in &lists {
            f.extend((l.len() as u16).to_le_bytes());
        }
        f.extend(lists.concat());
        f.push(0);
        MapFile::parse(&f).unwrap()
    }

    fn mmi(q: u8, tile: Option<(u8, u8)>) -> std::sync::Arc<[u8]> {
        let mut b = vec![0xffu8; 70];
        if let Some((x, y)) = tile {
            b[usize::from(q - 1) * 2..usize::from(q) * 2].copy_from_slice(&[x, y]);
        }
        b.into()
    }

    #[test]
    fn a_guide_tile_inside_a_door_points_through_it() {
        // Row 4 is a door (trigger 4) to map 56, with branches; (5, 2) is a bed (trigger 9, no warp).
        let m = map(8, 5, &[((4, 4), 4), ((5, 4), 4), ((5, 2), 9)], &[(4, &[56, 57, 56])]);
        assert_eq!(m.guide_exits(44, (4, 4)), vec![56, 57], "on the door: every branch, once");
        assert_eq!(m.guide_exits(44, (5, 3)), vec![56, 57], "one tile inside: the door below");
        assert_eq!(m.guide_exits(44, (5, 2)), Vec::<u8>::new(), "a trigger that goes nowhere is not looked past");
        assert_eq!(m.guide_exits(44, (1, 1)), Vec::<u8>::new());
    }

    /// 0 → [1, 2]; 1 → 0 (a loop back); 2 → [3 (event map, no guide), 4]; 4's tile is one step
    /// inside its door to 5; 5 has no guide: the quest is there, from 0 and from 1 alike.
    #[test]
    fn the_trail_backs_out_of_loops_and_prefers_branches_that_guide_on() {
        use std::{collections::HashMap, sync::Arc};
        const Q: u8 = 13;
        let files: HashMap<u8, Arc<MapFile>> = [
            (0, map(4, 4, &[((1, 3), 2)], &[(2, &[1, 2])])),
            (1, map(4, 4, &[((1, 0), 2)], &[(2, &[0])])),
            (2, map(4, 4, &[((3, 1), 5)], &[(5, &[3, 4])])),
            (3, map(4, 4, &[], &[])),
            (4, map(4, 4, &[((0, 2), 3)], &[(3, &[5])])),
            (5, map(4, 4, &[], &[])),
        ]
        .into_iter()
        .map(|(n, f)| (n, Arc::new(f)))
        .collect();
        let tables: HashMap<u8, Arc<[u8]>> =
            [(0, Some((1, 3))), (1, Some((1, 0))), (2, Some((3, 1))), (3, None), (4, Some((1, 2))), (5, None)].into_iter().map(|(n, t)| (n, mmi(Q, t))).collect();
        // The hook's protocol: start with nothing cached, load what is asked for, ask again.
        let target = |start: u8| {
            let (mut have_mmi, mut have_file) = (HashMap::new(), HashMap::new());
            let mut loads = 0;
            loop {
                match guide_target(start, Q, |m| have_mmi.get(&m).cloned().ok_or(m), |m| have_file.get(&m).cloned().ok_or(m)) {
                    Ok(t) => return (t, loads),
                    Err(m) => {
                        have_mmi.insert(m, tables.get(&m).cloned());
                        have_file.insert(m, files.get(&m).cloned());
                        loads += 1;
                        assert!(loads <= 6, "each map is asked for once");
                    }
                }
            }
        };
        assert_eq!(target(0).0, Some(5));
        assert_eq!(target(1).0, Some(5), "the loop back to 0 is not the answer");
        assert_eq!(target(4).0, Some(5));
        assert_eq!(target(3), (None, 1), "a map that guides nowhere starts no trail");
        assert_eq!(guide_target(0, Q, |m| Ok(tables.get(&m).cloned()), |_| Ok(None)), Ok(None), "no file, no doors");
    }

    #[test]
    fn the_guided_quest_is_the_latest_under_way() {
        let states = |under_way: u8, done_after: u8| move |q: u8| if q == under_way { 1 } else if q < done_after { 2 } else { 0 };
        assert_eq!(guided_quest(states(4, 4)), Some(4));
        assert_eq!(guided_quest(|q| if q <= 3 { 2 } else { 0 }), None, "the latest started is done");
        assert_eq!(guided_quest(|_| 0), None);
        let mut mmi = vec![0xffu8; 70];
        mmi[6..8].copy_from_slice(&[4, 10]);
        assert_eq!(guide_tile(&mmi, 4), Some((4, 10)));
        assert_eq!(guide_tile(&mmi, 3), None);
        assert_eq!(guide_tile(&mmi, 0), None);
    }
}
