//! Mermaid pie charts (`pie [showData] [title …]`) — parser and scene
//! (§3.7.6): `"label" : value` slices clockwise from 12 o'clock in theme
//! palette order, percentage labels inside the slices (hidden when too
//! thin), a legend (with values under `showData`) and the title.
//! Callers: `mermaid::render`/`mermaid::validate`.

use std::f32::consts::{PI, TAU};

use super::diag::Diagnostic;
use super::scene::{Anchor, Prim, Scene, Stroke};
use super::source::{Config, Source};
use super::text::{TextMeasure, clean_label};
use super::theme::{Theme, text_on};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pie {
    pub title: Option<String>,
    pub show_data: bool,
    /// (label, value, source line).
    pub slices: Vec<(String, f64, usize)>,
}

pub fn parse(src: &Source<'_>) -> (Pie, Vec<Diagnostic>) {
    let mut pie = Pie { title: src.title.clone(), ..Pie::default() };
    let mut diags = Vec::new();
    if let Some(h) = src.header() {
        let rest = h.text.trim_start_matches("pie").trim();
        let rest = match rest.strip_prefix("showData") {
            Some(r) => {
                pie.show_data = true;
                r.trim()
            }
            None => rest,
        };
        if let Some(t) = rest.strip_prefix("title") {
            pie.title = Some(clean_label(t.trim()));
        }
    }
    for line in src.body() {
        let t = line.text;
        if let Some(title) = t.strip_prefix("title") {
            pie.title = Some(clean_label(title.trim()));
            continue;
        }
        if t == "showData" {
            pie.show_data = true;
            continue;
        }
        let Some((label, value)) = t.rsplit_once(':') else {
            diags.push(Diagnostic::error(line.no, line.indent + 1, "expected `\"label\" : value`"));
            continue;
        };
        match value.trim().parse::<f64>().ok().filter(|v| v.is_finite()) {
            Some(v) if v >= 0.0 => pie.slices.push((clean_label(label.trim()), v, line.no)),
            Some(_) => diags.push(Diagnostic::error(line.no, line.indent + 1, "pie values must be positive")),
            None => diags.push(Diagnostic::error(
                line.no,
                line.indent + t.len() - value.len() + 1,
                format!("`{}` is not a number", value.trim()),
            )),
        }
    }
    (pie, diags)
}

pub fn build(pie: &Pie, config: &Config, theme: &Theme, measure: &dyn TextMeasure) -> Scene {
    let font = theme.font_size;
    let r = 150.0f32;
    let c = [r + 10.0, r + 10.0];
    let text_pos = config.f32(&["pie", "textPosition"]).unwrap_or(0.75).clamp(0.1, 1.0);
    let total: f64 = pie.slices.iter().map(|s| s.1).sum();
    let mut scene = Scene::new(theme.background);
    let outline = Stroke::new(theme.background, 2.0);

    if total <= 0.0 {
        scene.push(Prim::Ellipse { center: c, radius: [r, r], fill: theme.secondary, stroke: Some(outline) });
    }
    let mut start = -PI / 2.0;
    for (i, (label, value, line)) in pie.slices.iter().enumerate() {
        if total <= 0.0 || *value <= 0.0 {
            continue;
        }
        let frac = (*value / total) as f32;
        let sweep = frac * TAU;
        let fill = theme.palette_color(i);
        if frac >= 0.9999 {
            scene.push(Prim::Ellipse { center: c, radius: [r, r], fill, stroke: Some(outline) });
        } else {
            let steps = ((sweep / TAU) * 96.0).ceil().max(2.0) as usize;
            let mut pts = vec![c];
            for k in 0..=steps {
                let a = start + sweep * k as f32 / steps as f32;
                pts.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
            }
            scene.polygon(pts, fill, Some(outline));
        }
        if frac > 0.03 {
            let mid = start + sweep / 2.0;
            let pos = [c[0] + r * text_pos * mid.cos(), c[1] + r * text_pos * mid.sin()];
            let pct = format!("{:.0}%", frac * 100.0);
            scene.text(measure, pos, &pct, font * 0.9, text_on(fill), Anchor::Middle, false);
        }
        scene.hit([c[0] - r, c[1] - r, c[0] + r, c[1] + r], label.clone(), *line);
        start += sweep;
    }

    // Legend.
    let lx = c[0] + r + 40.0;
    let row = (font * 1.25).max(20.0) + 4.0;
    let ly0 = c[1] - row * pie.slices.len() as f32 / 2.0;
    for (i, (label, value, line)) in pie.slices.iter().enumerate() {
        let y = ly0 + i as f32 * row + row / 2.0;
        scene.rect([lx, y - 8.0], [16.0, 16.0], 2.0, theme.palette_color(i), None);
        let text = if pie.show_data { format!("{label} [{}]", fmt_value(*value)) } else { label.clone() };
        scene.text(measure, [lx + 24.0, y], &text, font, theme.text, Anchor::Start, false);
        let w = measure.size(&text, font).0;
        scene.hit([lx, y - row / 2.0, lx + 24.0 + w, y + row / 2.0], label.clone(), *line);
    }
    if let Some(t) = pie.title.as_deref().filter(|t| !t.is_empty()) {
        scene.text(measure, [c[0], -font], t, font * 1.25, theme.text, Anchor::Middle, true);
    }
    scene.fit(8.0);
    scene
}

fn fmt_value(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::source::preprocess;
    use crate::mermaid::text::ApproxMeasure;

    #[test]
    fn parses_and_draws_slices_and_legend() {
        let s = preprocess("pie showData title Pets adopted\n  \"Dogs\" : 386\n  \"Cats\" : 85.5\n  \"Rats\" : 15\n");
        let (pie, d) = parse(&s);
        assert!(d.is_empty(), "{d:?}");
        assert!(pie.show_data);
        assert_eq!(pie.title.as_deref(), Some("Pets adopted"));
        assert_eq!(pie.slices.len(), 3);
        let sc = build(&pie, &s.config, &Theme::default_theme(), &ApproxMeasure);
        let polys = sc.prims.iter().filter(|p| matches!(p, Prim::Polygon { .. })).count();
        assert_eq!(polys, 3);
        assert!(sc.prims.iter().any(|p| matches!(p, Prim::Text { text, .. } if text == "Dogs [386]")));
        assert!(sc.prims.iter().any(|p| matches!(p, Prim::Text { text, .. } if text == "Cats [85.5]")));
        assert!(sc.prims.iter().any(|p| matches!(p, Prim::Text { text, .. } if text == "79%")));
    }

    #[test]
    fn bad_values_are_diagnosed() {
        let (_, d) = parse(&preprocess("pie\n  \"A\" : lots\n  B\n"));
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|x| x.is_error()));
    }
}
