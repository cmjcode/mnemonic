//! Mermaid themes (`default`, `dark`, `forest`, `neutral`, `base`), the
//! `themeVariables` overrides that config may carry, CSS colour parsing and
//! the `fill:…,stroke:…` style strings used by `classDef`/`style`/
//! `linkStyle` (§3.7.4). Colours are plain RGBA bytes so scenes stay
//! egui-free. Callers: `mermaid::render`, every diagram scene builder.

use super::source::{Config, leading_number};

pub type Color = [u8; 4];

pub const TRANSPARENT: Color = [0, 0, 0, 0];

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub background: Color,
    pub font_size: f32,
    pub primary: Color,
    pub primary_border: Color,
    pub primary_text: Color,
    pub secondary: Color,
    pub tertiary: Color,
    pub line: Color,
    pub text: Color,
    pub cluster_bkg: Color,
    pub cluster_border: Color,
    pub note_bkg: Color,
    pub note_border: Color,
    pub note_text: Color,
    pub edge_label_bg: Color,
    pub grid: Color,
    /// Categorical colours: pie slices, git branches, chart series, …
    pub palette: Vec<Color>,
}

const PALETTE_LIGHT: [&str; 12] = [
    "#8b8bf5", "#f5d76e", "#9ed89c", "#f0a37a", "#7fc9d6", "#d98bd5", "#b5b5b5", "#f28b8b", "#a3c585",
    "#e9b872", "#8fa9e0", "#c5a3e0",
];
const PALETTE_DARK: [&str; 12] = [
    "#5b5bc8", "#b89b2e", "#4f9a4c", "#b8653c", "#3e8d9a", "#9a4f96", "#707070", "#b04e4e", "#6b8f4b",
    "#b08432", "#4f6fad", "#8a63ad",
];

impl Theme {
    /// A built-in theme by Mermaid name; unknown names fall back to
    /// `default` (or `dark` when `dark_ui`).
    pub fn named(name: &str, dark_ui: bool) -> Theme {
        match name {
            "dark" => Theme::dark(),
            "forest" => Theme::forest(),
            "neutral" => Theme::neutral(),
            "base" => Theme::base(),
            "default" => Theme::default_theme(),
            _ if dark_ui => Theme::dark(),
            _ => Theme::default_theme(),
        }
    }

    /// Theme for a diagram: config `theme` if set, else one matching the
    /// app's light/dark mode; then `themeVariables` overrides.
    pub fn resolve(config: &Config, dark_ui: bool) -> Theme {
        let mut theme = Theme::named(config.str(&["theme"]).unwrap_or(""), dark_ui);
        if let Some(vars) = config.get(&["themeVariables"]).and_then(|v| v.as_object()) {
            for (key, value) in vars {
                if let Some(s) = value.as_str() {
                    theme.set_variable(key, s);
                } else if let Some(n) = value.as_f64() {
                    theme.set_variable(key, &n.to_string());
                }
            }
        }
        if let Some(size) = config.f32(&["fontSize"]) {
            theme.font_size = size.clamp(6.0, 64.0);
        }
        theme
    }

    fn set_variable(&mut self, key: &str, value: &str) {
        if key == "fontSize" {
            if let Some(v) = leading_number(value) {
                self.font_size = v.clamp(6.0, 64.0);
            }
            return;
        }
        if key == "darkMode" {
            self.dark = value == "true";
            return;
        }
        let Some(c) = parse_color(value) else { return };
        match key {
            "background" => self.background = c,
            "primaryColor" | "mainBkg" | "nodeBkg" => self.primary = c,
            "primaryBorderColor" | "nodeBorder" => self.primary_border = c,
            "primaryTextColor" => self.primary_text = c,
            "secondaryColor" => self.secondary = c,
            "tertiaryColor" => self.tertiary = c,
            "lineColor" | "defaultLinkColor" => self.line = c,
            "textColor" => self.text = c,
            "clusterBkg" => self.cluster_bkg = c,
            "clusterBorder" => self.cluster_border = c,
            "noteBkgColor" => self.note_bkg = c,
            "noteBorderColor" => self.note_border = c,
            "noteTextColor" => self.note_text = c,
            "edgeLabelBackground" => self.edge_label_bg = c,
            _ => {
                // pie1…pie12, git0…git7, cScale0…cScale11 → palette.
                for prefix in ["pie", "git", "cScale"] {
                    if let Some(idx) = key.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok()) {
                        let idx = if prefix == "pie" { idx.saturating_sub(1) } else { idx };
                        if idx < 32 {
                            while self.palette.len() <= idx {
                                self.palette.push(self.primary);
                            }
                            self.palette[idx] = c;
                        }
                    }
                }
            }
        }
    }

    /// Palette colour `i`, cycling.
    pub fn palette_color(&self, i: usize) -> Color {
        if self.palette.is_empty() {
            self.primary
        } else {
            self.palette[i % self.palette.len()]
        }
    }

    pub fn default_theme() -> Theme {
        Theme {
            name: "default".into(),
            dark: false,
            background: hex("#ffffff"),
            font_size: 16.0,
            primary: hex("#ECECFF"),
            primary_border: hex("#9370DB"),
            primary_text: hex("#131300"),
            secondary: hex("#ffffde"),
            tertiary: hex("#f4ffe6"),
            line: hex("#333333"),
            text: hex("#333333"),
            cluster_bkg: hex("#ffffde"),
            cluster_border: hex("#aaaa33"),
            note_bkg: hex("#fff5ad"),
            note_border: hex("#aaaa33"),
            note_text: hex("#333333"),
            edge_label_bg: [232, 232, 232, 220],
            grid: hex("#e0e0e0"),
            palette: PALETTE_LIGHT.iter().map(|s| hex(s)).collect(),
        }
    }

    pub fn dark() -> Theme {
        Theme {
            name: "dark".into(),
            dark: true,
            background: hex("#1e1e1e"),
            font_size: 16.0,
            primary: hex("#1f2020"),
            primary_border: hex("#cccccc"),
            primary_text: hex("#e0dfdf"),
            secondary: hex("#3a3a3a"),
            tertiary: hex("#2b2b2b"),
            line: hex("#d3d3d3"),
            text: hex("#cccccc"),
            cluster_bkg: hex("#2c2c2c"),
            cluster_border: hex("#707070"),
            note_bkg: hex("#555044"),
            note_border: hex("#8a8466"),
            note_text: hex("#f0f0f0"),
            edge_label_bg: [70, 70, 70, 230],
            grid: hex("#444444"),
            palette: PALETTE_DARK.iter().map(|s| hex(s)).collect(),
        }
    }

    pub fn forest() -> Theme {
        Theme {
            name: "forest".into(),
            primary: hex("#cde498"),
            primary_border: hex("#13540c"),
            primary_text: hex("#000000"),
            secondary: hex("#cdffb2"),
            tertiary: hex("#e8f5d8"),
            line: hex("#008000"),
            text: hex("#000000"),
            cluster_bkg: hex("#cdffb2"),
            cluster_border: hex("#6eaa49"),
            note_border: hex("#6eaa49"),
            palette: ["#6eaa49", "#cde498", "#487e3a", "#a4d17a", "#2f6b24", "#e0efc2", "#8bbf6a", "#1f4e17"]
                .iter()
                .map(|s| hex(s))
                .collect(),
            ..Theme::default_theme()
        }
    }

    pub fn neutral() -> Theme {
        Theme {
            name: "neutral".into(),
            primary: hex("#eeeeee"),
            primary_border: hex("#999999"),
            primary_text: hex("#111111"),
            secondary: hex("#f4f4f4"),
            tertiary: hex("#fafafa"),
            line: hex("#666666"),
            text: hex("#333333"),
            cluster_bkg: hex("#f4f4f4"),
            cluster_border: hex("#999999"),
            note_bkg: hex("#f4f4f4"),
            note_border: hex("#999999"),
            palette: ["#555555", "#888888", "#aaaaaa", "#cccccc", "#666666", "#999999", "#bbbbbb", "#dddddd"]
                .iter()
                .map(|s| hex(s))
                .collect(),
            ..Theme::default_theme()
        }
    }

    pub fn base() -> Theme {
        Theme {
            name: "base".into(),
            primary: hex("#fff4dd"),
            primary_border: hex("#c9b68f"),
            primary_text: hex("#333333"),
            secondary: hex("#fbeede"),
            tertiary: hex("#fdf8ef"),
            line: hex("#555555"),
            cluster_bkg: hex("#fbeede"),
            cluster_border: hex("#c9b68f"),
            ..Theme::default_theme()
        }
    }
}

fn hex(s: &str) -> Color {
    parse_color(s).unwrap_or([0, 0, 0, 255])
}

/// CSS colour: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()/rgba()`,
/// `hsl()/hsla()`, `transparent`, `none` and common named colours.
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim().trim_end_matches(';').trim();
    let lower = s.to_ascii_lowercase();
    if let Some(h) = lower.strip_prefix('#') {
        let digit = |i: usize, len: usize| u8::from_str_radix(h.get(i..i + len)?, 16).ok();
        return match h.len() {
            3 | 4 => {
                let n = |i| digit(i, 1).map(|v| v * 17);
                Some([n(0)?, n(1)?, n(2)?, if h.len() == 4 { n(3)? } else { 255 }])
            }
            6 | 8 => Some([
                digit(0, 2)?,
                digit(2, 2)?,
                digit(4, 2)?,
                if h.len() == 8 { digit(6, 2)? } else { 255 },
            ]),
            _ => None,
        };
    }
    if let Some(args) = func_args(&lower, "rgba").or_else(|| func_args(&lower, "rgb")) {
        let v: Vec<f32> = args.iter().filter_map(|a| leading_number(a)).collect();
        if v.len() < 3 {
            return None;
        }
        let a = v.get(3).map_or(255, |a| if *a <= 1.0 { (a * 255.0) as u8 } else { *a as u8 });
        return Some([v[0] as u8, v[1] as u8, v[2] as u8, a]);
    }
    if let Some(args) = func_args(&lower, "hsla").or_else(|| func_args(&lower, "hsl")) {
        let v: Vec<f32> = args.iter().filter_map(|a| leading_number(a)).collect();
        if v.len() < 3 {
            return None;
        }
        let [r, g, b] = hsl_to_rgb(v[0], v[1] / 100.0, v[2] / 100.0);
        let a = v.get(3).map_or(255, |a| (a.clamp(0.0, 1.0) * 255.0) as u8);
        return Some([r, g, b, a]);
    }
    let named = match lower.as_str() {
        "none" | "transparent" => TRANSPARENT,
        "black" => [0, 0, 0, 255],
        "white" => [255, 255, 255, 255],
        "red" => [255, 0, 0, 255],
        "green" => [0, 128, 0, 255],
        "lime" => [0, 255, 0, 255],
        "blue" => [0, 0, 255, 255],
        "yellow" => [255, 255, 0, 255],
        "orange" => [255, 165, 0, 255],
        "purple" => [128, 0, 128, 255],
        "pink" => [255, 192, 203, 255],
        "gray" | "grey" => [128, 128, 128, 255],
        "lightgray" | "lightgrey" => [211, 211, 211, 255],
        "darkgray" | "darkgrey" => [169, 169, 169, 255],
        "silver" => [192, 192, 192, 255],
        "brown" => [165, 42, 42, 255],
        "cyan" | "aqua" => [0, 255, 255, 255],
        "magenta" | "fuchsia" => [255, 0, 255, 255],
        "navy" => [0, 0, 128, 255],
        "teal" => [0, 128, 128, 255],
        "olive" => [128, 128, 0, 255],
        "maroon" => [128, 0, 0, 255],
        "gold" => [255, 215, 0, 255],
        "coral" => [255, 127, 80, 255],
        "salmon" => [250, 128, 114, 255],
        "khaki" => [240, 230, 140, 255],
        "violet" => [238, 130, 238, 255],
        "indigo" => [75, 0, 130, 255],
        "tomato" => [255, 99, 71, 255],
        "skyblue" => [135, 206, 235, 255],
        "lightblue" => [173, 216, 230, 255],
        "lightgreen" => [144, 238, 144, 255],
        "lightyellow" => [255, 255, 224, 255],
        "darkgreen" => [0, 100, 0, 255],
        "darkblue" => [0, 0, 139, 255],
        "darkred" => [139, 0, 0, 255],
        "beige" => [245, 245, 220, 255],
        "lavender" => [230, 230, 250, 255],
        "crimson" => [220, 20, 60, 255],
        "steelblue" => [70, 130, 180, 255],
        "tan" => [210, 180, 140, 255],
        _ => return None,
    };
    Some(named)
}

fn func_args(s: &str, name: &str) -> Option<Vec<String>> {
    let rest = s.strip_prefix(name)?.trim_start().strip_prefix('(')?;
    let inner = rest.strip_suffix(')')?;
    Some(
        inner
            .split([',', ' ', '/'])
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(String::from)
            .collect(),
    )
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let ch = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    [ch(r), ch(g), ch(b)]
}

/// Relative luminance in 0..1, for picking readable text colours.
pub fn luminance(c: Color) -> f32 {
    (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0
}

/// Black-ish or white-ish, whichever reads on `fill`.
pub fn text_on(fill: Color) -> Color {
    if luminance(fill) > 0.55 { [30, 30, 30, 255] } else { [245, 245, 245, 255] }
}

/// Parsed `fill:#f9f,stroke:#333,stroke-width:4px,color:#fff,
/// stroke-dasharray: 5 5` — every field optional so styles layer
/// (`classDef default` → classes → `style`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleSpec {
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    pub stroke_width: Option<f32>,
    pub color: Option<Color>,
    pub dash: Option<[f32; 2]>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub font_size: Option<f32>,
}

impl StyleSpec {
    pub fn parse(s: &str) -> StyleSpec {
        let mut out = StyleSpec::default();
        for decl in split_top_level(s) {
            let Some((key, value)) = decl.split_once(':') else { continue };
            let value = value.trim().trim_end_matches("!important").trim();
            match key.trim() {
                "fill" | "background" | "background-color" => out.fill = parse_color(value),
                "stroke" | "border-color" => out.stroke = parse_color(value),
                "stroke-width" | "border-width" => out.stroke_width = leading_number(value),
                "color" => out.color = parse_color(value),
                "stroke-dasharray" => {
                    let v: Vec<f32> = value.split([' ', ',']).filter_map(leading_number).collect();
                    out.dash = match v.as_slice() {
                        [] => None,
                        [a] => Some([*a, *a]),
                        [a, b, ..] => Some([*a, *b]),
                    };
                }
                "font-weight" => {
                    out.bold = Some(value == "bold" || leading_number(value).is_some_and(|w| w >= 600.0))
                }
                "font-style" => out.italic = Some(value == "italic"),
                "font-size" => out.font_size = leading_number(value),
                _ => {}
            }
        }
        out
    }

    /// Layer `other` on top of `self` (set fields win).
    pub fn apply(&mut self, other: &StyleSpec) {
        macro_rules! take {
            ($($f:ident),*) => { $( if other.$f.is_some() { self.$f = other.$f; } )* };
        }
        take!(fill, stroke, stroke_width, color, dash, bold, italic, font_size);
    }
}

/// Split on `,`/`;` outside parentheses (so `rgb(1,2,3)` survives); a
/// piece without `:` continues the previous value (`stroke-dasharray: 5, 5`).
fn split_top_level(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if (c == ',' || c == ';') && depth == 0 {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    let mut merged: Vec<String> = Vec::new();
    for part in parts {
        if !part.contains(':')
            && !part.trim().is_empty()
            && let Some(last) = merged.last_mut()
        {
            last.push(',');
            last.push_str(&part);
            continue;
        }
        merged.push(part);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_css_colors() {
        assert_eq!(parse_color("#f9f"), Some([255, 153, 255, 255]));
        assert_eq!(parse_color("#11223344"), Some([0x11, 0x22, 0x33, 0x44]));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some([1, 2, 3, 255]));
        assert_eq!(parse_color("rgba(1,2,3,0.5)"), Some([1, 2, 3, 127]));
        assert_eq!(parse_color("hsl(0, 100%, 50%)"), Some([255, 0, 0, 255]));
        assert_eq!(parse_color("Red"), Some([255, 0, 0, 255]));
        assert_eq!(parse_color("nonsense"), None);
    }

    #[test]
    fn parses_style_strings_and_layers() {
        let mut s = StyleSpec::parse("fill:#f9f,stroke:#333,stroke-width:4px");
        assert_eq!(s.fill, Some([255, 153, 255, 255]));
        assert_eq!(s.stroke_width, Some(4.0));
        s.apply(&StyleSpec::parse("color:#fff,stroke-dasharray: 5, 5"));
        assert_eq!(s.color, Some([255, 255, 255, 255]));
        assert_eq!(s.dash, Some([5.0, 5.0]));
        assert_eq!(s.fill, Some([255, 153, 255, 255]));
        assert_eq!(StyleSpec::parse("fill:rgb(1,2,3)").fill, Some([1, 2, 3, 255]));
    }

    #[test]
    fn theme_variables_override_named_theme() {
        let cfg = Config {
            root: serde_json::json!({"theme": "forest", "themeVariables": {"primaryColor": "#ff0000", "pie2": "#00ff00", "fontSize": "20px"}}),
        };
        let t = Theme::resolve(&cfg, false);
        assert_eq!(t.name, "forest");
        assert_eq!(t.primary, [255, 0, 0, 255]);
        assert_eq!(t.palette_color(1), [0, 255, 0, 255]);
        assert_eq!(t.font_size, 20.0);
        assert!(Theme::resolve(&Config::default(), true).dark);
    }
}
