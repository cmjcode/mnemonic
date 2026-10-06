//! `Scene` → standalone SVG string (§3.7.5), for `mnemonic-cli diagram
//! render` / the `render_diagram` MCP tool and export. Pure string
//! building: no DOM, no fonts — text uses the same measured extents as the
//! egui backend. Callers: `api::diagram`.

use std::fmt::Write;

use super::scene::{Anchor, MarkShape, Prim, Scene, Stroke, marker_shapes};
use super::text::line_height;
use super::theme::Color;

pub fn to_svg(scene: &Scene) -> String {
    let mut s = String::with_capacity(256 + scene.prims.len() * 96);
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.1} {h:.1}" font-family="Inter, 'Trebuchet MS', Verdana, Arial, sans-serif">"#,
        w = scene.width.max(1.0),
        h = scene.height.max(1.0)
    );
    if scene.background[3] > 0 {
        let _ = write!(s, r#"<rect width="100%" height="100%"{}/>"#, fill_attr(scene.background));
    }
    for prim in &scene.prims {
        write_prim(&mut s, prim);
    }
    s.push_str("</svg>\n");
    s
}

fn write_prim(s: &mut String, prim: &Prim) {
    match prim {
        Prim::Rect { min, size, radius, fill, stroke } => {
            let _ = write!(
                s,
                r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{:.1}"{}{}/>"#,
                min[0],
                min[1],
                size[0],
                size[1],
                radius,
                fill_attr(*fill),
                stroke_attr(stroke.as_ref())
            );
        }
        Prim::Ellipse { center, radius, fill, stroke } => {
            let _ = write!(
                s,
                r#"<ellipse cx="{:.1}" cy="{:.1}" rx="{:.1}" ry="{:.1}"{}{}/>"#,
                center[0],
                center[1],
                radius[0],
                radius[1],
                fill_attr(*fill),
                stroke_attr(stroke.as_ref())
            );
        }
        Prim::Polygon { points, fill, stroke } => {
            let _ = write!(
                s,
                r#"<polygon points="{}"{}{}/>"#,
                points_attr(points),
                fill_attr(*fill),
                stroke_attr(stroke.as_ref())
            );
        }
        Prim::Line { points, stroke } => {
            let _ = write!(
                s,
                r#"<polyline points="{}" fill="none" stroke-linejoin="round"{}/>"#,
                points_attr(points),
                stroke_attr(Some(stroke))
            );
        }
        Prim::Marker { at, angle, kind, color, bg, size } => {
            for shape in marker_shapes(*kind, *at, *angle, *size) {
                let stroke = Stroke::new(*color, 1.3);
                match shape {
                    MarkShape::Poly { points, filled } => {
                        let _ = write!(
                            s,
                            r#"<polygon points="{}"{}{}/>"#,
                            points_attr(&points),
                            fill_attr(if filled { *color } else { *bg }),
                            stroke_attr(Some(&stroke))
                        );
                    }
                    MarkShape::Circle { center, r, filled } => {
                        let _ = write!(
                            s,
                            r#"<circle cx="{:.1}" cy="{:.1}" r="{:.1}"{}{}/>"#,
                            center[0],
                            center[1],
                            r,
                            fill_attr(if filled { *color } else { *bg }),
                            stroke_attr(Some(&stroke))
                        );
                    }
                    MarkShape::Lines(lines) => {
                        for [a, b] in lines {
                            let _ = write!(
                                s,
                                r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"{}/>"#,
                                a[0],
                                a[1],
                                b[0],
                                b[1],
                                stroke_attr(Some(&stroke))
                            );
                        }
                    }
                }
            }
        }
        Prim::Text { pos, text, size, color, anchor, bold, italic, extent, angle } => {
            let anchor_attr = match anchor {
                Anchor::Start => "start",
                Anchor::Middle => "middle",
                Anchor::End => "end",
            };
            let lines: Vec<&str> = text.split('\n').collect();
            let lh = line_height(*size);
            let first_baseline = pos[1] - extent[1] / 2.0 + lh / 2.0;
            let _ = write!(
                s,
                r#"<text x="{:.1}" y="{:.1}" font-size="{:.1}" text-anchor="{anchor_attr}" dominant-baseline="central"{}{}{}"#,
                pos[0],
                first_baseline,
                size,
                fill_attr(*color),
                if *bold { r#" font-weight="bold""# } else { "" },
                if *italic { r#" font-style="italic""# } else { "" },
            );
            if angle.abs() > 1e-3 {
                let _ = write!(s, r#" transform="rotate({:.1} {:.1} {:.1})""#, angle.to_degrees(), pos[0], pos[1]);
            }
            s.push('>');
            if lines.len() == 1 {
                escape_into(s, text);
            } else {
                for (i, line) in lines.iter().enumerate() {
                    let _ = write!(s, r#"<tspan x="{:.1}" dy="{:.1}">"#, pos[0], if i == 0 { 0.0 } else { lh });
                    escape_into(s, line);
                    s.push_str("</tspan>");
                }
            }
            s.push_str("</text>");
        }
    }
}

fn points_attr(points: &[[f32; 2]]) -> String {
    let mut out = String::with_capacity(points.len() * 14);
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{:.1},{:.1}", p[0], p[1]);
    }
    out
}

fn css_color(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn fill_attr(c: Color) -> String {
    if c[3] == 0 {
        r#" fill="none""#.to_string()
    } else if c[3] == 255 {
        format!(r#" fill="{}""#, css_color(c))
    } else {
        format!(r#" fill="{}" fill-opacity="{:.2}""#, css_color(c), c[3] as f32 / 255.0)
    }
}

fn stroke_attr(stroke: Option<&Stroke>) -> String {
    let Some(st) = stroke else { return String::new() };
    if st.color[3] == 0 || st.width <= 0.0 {
        return String::new();
    }
    let mut out = format!(r#" stroke="{}" stroke-width="{:.1}""#, css_color(st.color), st.width);
    if st.color[3] < 255 {
        let _ = write!(out, r#" stroke-opacity="{:.2}""#, st.color[3] as f32 / 255.0);
    }
    if let Some([a, b]) = st.dash {
        let _ = write!(out, r#" stroke-dasharray="{a:.1} {b:.1}""#);
    }
    out
}

fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::scene::Marker;
    use crate::mermaid::text::ApproxMeasure;

    #[test]
    fn serialises_primitives_and_escapes_text() {
        let mut scene = Scene::new([255, 255, 255, 255]);
        scene.rect([0.0, 0.0], [10.0, 10.0], 2.0, [1, 2, 3, 255], Some(Stroke::new([0, 0, 0, 255], 1.0)));
        scene.text(&ApproxMeasure, [5.0, 5.0], "a<b & c\nd", 12.0, [0, 0, 0, 255], Anchor::Middle, false);
        scene.edge(
            vec![[0.0, 0.0], [20.0, 0.0]],
            Stroke::dashed([0, 0, 0, 255], 1.0, [3.0, 3.0]),
            None,
            Some(Marker::Arrow),
            [255; 4],
        );
        scene.fit(4.0);
        let svg = to_svg(&scene);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("a&lt;b &amp; c"));
        assert!(svg.contains("<tspan"));
        assert!(svg.contains(r#"stroke-dasharray="3.0 3.0""#));
        assert!(svg.contains("<polygon"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }
}
