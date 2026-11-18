// Overlay button icons, embedded from `ui_icons.bin` — hero2's module and its file, the bin byte
// for byte (`D:\tmp\hero2NativeAPP\wie_android\src\ui_icons.{bin,rs}`, md5 `ae14a71a…`, A8 2026-10-26;
// hero2 builds it with `android_app/tools/gen_ui_icons.py` from the PNGs in
// `android_app/tools/ui_icons/`). This port draws only the minimap's folded map.
//
// ★ These are COVERAGE MASKS, not colour images: one byte of alpha per pixel, no RGB.
// The source art is a black silhouette, and the overlay draws every control with a
// single light foreground colour — so baking the source RGB in would give a black
// glyph on a translucent black plate, i.e. invisible. Keeping only the shape lets one
// asset serve the normal / pressed / dimmed states and match the hand-drawn glyphs in
// `overlay.rs` (`draw_glyph`, `draw_arrow`) instead of clashing with them.
//
// The packer crops each source to its inked bounding box and centres it in a fixed
// square cell, so art authored with different padding still draws at the same visual
// weight. Scaling and cropping happen at build time: the sources are up to 4000x4000
// and this is drawn on the UI thread every frame.
//
// Binary format (see gen_ui_icons.py):
//
//   magic "H3UI" (4)
//   count u8
//   per icon:  name_len u8, name[name_len] (ascii), w u8, h u8, a[w*h] u8
//
// Structured as a parser over a byte slice with the embedding kept separate, so the
// format can be exercised with synthetic blobs (see the tests below) rather than only
// against whatever happens to be checked in.

use std::sync::OnceLock;

static UI_ICONS_BIN: &[u8] = include_bytes!("ui_icons.bin");

/// A single-channel coverage mask. `a[y * w + x]` = 0 (nothing) .. 255 (solid).
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub a: Vec<u8>,
}

impl Mask {
    /// Coverage at `(x, y)`, or 0 outside. Callers blit with this as the blend weight
    /// against their chosen colour.
    pub fn coverage(&self, x: u32, y: u32) -> u8 {
        if x >= self.w || y >= self.h {
            return 0;
        }
        self.a[(y * self.w + x) as usize]
    }
}

/// Which icon to draw. The names are the strings the packer wrote, and the mapping is
/// checked by a test so a renamed asset fails the build's tests rather than silently
/// drawing nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    /// Page-switch arrows, for the quickbar page button.
    QuickPage,
    /// Folded map with a pin, for the minimap RESTORE button — drawn in the panel's corner
    /// while the panel is hidden (`overlay::minimap_restore_button`; 2026-10-21, back after the
    /// `MAP` tab became the world map's switch).
    MinimapToggle,
}

impl Icon {
    pub fn name(self) -> &'static str {
        match self {
            Icon::QuickPage => "quick_page",
            Icon::MinimapToggle => "minimap_toggle",
        }
    }
}

pub struct UiIcons {
    icons: Vec<(String, Mask)>,
}

impl UiIcons {
    pub fn get(&self, icon: Icon) -> Option<&Mask> {
        let want = icon.name();
        self.icons.iter().find(|(n, _)| n == want).map(|(_, m)| m)
    }
}

/// The embedded set, parsed once. A malformed blob yields an EMPTY set rather than a
/// panic: a missing icon costs one button's face, and the alternative is taking the
/// whole app down on the UI thread over an asset problem.
pub fn icons() -> &'static UiIcons {
    static CACHE: OnceLock<UiIcons> = OnceLock::new();
    CACHE.get_or_init(|| {
        let icons = parse(UI_ICONS_BIN).unwrap_or_else(|| {
            log::error!("ui_icons: ui_icons.bin is malformed; button icons disabled");
            Vec::new()
        });
        UiIcons { icons }
    })
}

/// Parse the packer's output. `None` on any inconsistency — a truncated blob is a
/// build problem, and half-reading it would draw garbage.
fn parse(d: &[u8]) -> Option<Vec<(String, Mask)>> {
    if d.len() < 5 || &d[0..4] != b"H3UI" {
        return None;
    }
    let count = d[4] as usize;
    let mut off = 5usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let name_len = *d.get(off)? as usize;
        off += 1;
        let name = std::str::from_utf8(d.get(off..off.checked_add(name_len)?)?).ok()?.to_owned();
        off += name_len;
        let w = u32::from(*d.get(off)?);
        let h = u32::from(*d.get(off + 1)?);
        off += 2;
        if w == 0 || h == 0 {
            return None;
        }
        let n = (w as usize).checked_mul(h as usize)?;
        let a = d.get(off..off.checked_add(n)?)?.to_vec();
        off += n;
        out.push((name, Mask { w, h, a }));
    }
    // Trailing bytes mean the writer and this reader disagree about the format.
    (off == d.len()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(entries: &[(&str, u8, u8)]) -> Vec<u8> {
        let mut v = b"H3UI".to_vec();
        v.push(entries.len() as u8);
        for (name, w, h) in entries {
            v.push(name.len() as u8);
            v.extend_from_slice(name.as_bytes());
            v.push(*w);
            v.push(*h);
            v.extend(std::iter::repeat_n(0u8, usize::from(*w) * usize::from(*h)));
        }
        v
    }

    #[test]
    fn parses_a_well_formed_blob() {
        let got = parse(&blob(&[("a", 2, 3), ("bb", 1, 1)])).expect("parses");
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].0.as_str(), got[0].1.w, got[0].1.h), ("a", 2, 3));
        assert_eq!(got[1].0.as_str(), "bb");
    }

    #[test]
    fn rejects_bad_magic_and_truncation() {
        assert!(parse(b"NOPE\x00").is_none());
        let mut short = blob(&[("a", 4, 4)]);
        short.truncate(short.len() - 1);
        assert!(parse(&short).is_none(), "a truncated mask must not half-parse");
    }

    /// Trailing bytes mean the packer and this reader disagree; that has to fail loudly
    /// in tests rather than quietly dropping an icon at runtime.
    #[test]
    fn rejects_trailing_bytes() {
        let mut extra = blob(&[("a", 1, 1)]);
        extra.push(0);
        assert!(parse(&extra).is_none());
    }

    #[test]
    fn rejects_a_zero_sized_mask() {
        assert!(parse(&blob(&[("a", 0, 4)])).is_none());
    }

    #[test]
    fn coverage_is_zero_outside_the_mask() {
        let m = Mask { w: 2, h: 2, a: vec![1, 2, 3, 4] };
        assert_eq!(m.coverage(1, 1), 4);
        assert_eq!(m.coverage(2, 0), 0);
        assert_eq!(m.coverage(0, 2), 0);
    }

    /// ★ The real asset must contain exactly the icons the `Icon` enum names. This is
    /// what catches a renamed or dropped PNG — the failure mode otherwise is a button
    /// that draws an empty face on a device.
    #[test]
    fn the_embedded_asset_has_every_icon_the_enum_names() {
        let set = icons();
        for icon in [Icon::QuickPage, Icon::MinimapToggle] {
            let m = set.get(icon).unwrap_or_else(|| panic!("{} missing from ui_icons.bin", icon.name()));
            assert!(m.w > 0 && m.h > 0);
            assert!(m.a.iter().any(|&v| v > 128), "{} has no inked pixels", icon.name());
        }
    }
}
