//! Note-card color palette (§3.1.2: "8-12 warna pastel ala Keep") and a
//! small deterministic tag-color assignment (§3.1.3). Kept separate from
//! `notes::` so that module stays free of `egui`. Callers: `app.rs`.

use egui::Color32;

/// `(frontmatter value, display color)` pairs offered by the card color
/// picker. An unrecognized or absent value falls back to the theme's
/// default card background (`color_for` returns `None`).
pub const PALETTE: &[(&str, Color32)] = &[
    ("yellow", Color32::from_rgb(253, 224, 71)),
    ("green", Color32::from_rgb(187, 247, 208)),
    ("blue", Color32::from_rgb(191, 219, 254)),
    ("purple", Color32::from_rgb(221, 214, 254)),
    ("pink", Color32::from_rgb(251, 207, 232)),
    ("red", Color32::from_rgb(254, 202, 202)),
    ("orange", Color32::from_rgb(254, 215, 170)),
    ("teal", Color32::from_rgb(153, 246, 228)),
    ("gray", Color32::from_rgb(229, 231, 235)),
];

/// The display color for a note's `frontmatter.color` value, or `None` if
/// unset/unrecognized (caller should use the default card styling).
pub fn color_for(name: Option<&str>) -> Option<Color32> {
    let name = name?;
    PALETTE.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

const TAG_COLORS: &[Color32] = &[
    Color32::from_rgb(59, 130, 246),
    Color32::from_rgb(16, 185, 129),
    Color32::from_rgb(168, 85, 247),
    Color32::from_rgb(244, 63, 94),
    Color32::from_rgb(202, 138, 4),
    Color32::from_rgb(20, 184, 166),
];

/// A deterministic accent color for a tag chip, picked by hashing the tag
/// name — approximates §3.1.3's "beri warna tag" without needing a
/// persisted per-tag color assignment yet.
pub fn tag_color(tag: &str) -> Color32 {
    let mut hash: u32 = 2166136261;
    for b in tag.to_lowercase().as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    TAG_COLORS[(hash as usize) % TAG_COLORS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_for_known_and_unknown_names() {
        assert_eq!(color_for(Some("yellow")), Some(Color32::from_rgb(253, 224, 71)));
        assert_eq!(color_for(Some("not-a-color")), None);
        assert_eq!(color_for(None), None);
    }

    #[test]
    fn tag_color_is_deterministic() {
        assert_eq!(tag_color("rumah"), tag_color("rumah"));
        assert_eq!(tag_color("Rumah"), tag_color("rumah")); // case-insensitive
    }
}
