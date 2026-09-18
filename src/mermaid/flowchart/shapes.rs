//! Flowchart node geometry: size from the measured label (Mermaid's
//! padding rules), edge-clipping outline, and the drawing of every shape
//! as scene primitives. Non-convex outlines are emitted as polygons; the
//! egui backend triangulates them. Callers: `flowchart::build`.

use std::f32::consts::PI;

use super::Shape;
use crate::mermaid::scene::{ClipShape, P, Prim, Scene, Stroke};
use crate::mermaid::theme::Color;

/// Shapes drawn without their label (markers such as start/stop dots).
pub(super) fn shows_label(shape: Shape) -> bool {
    !matches!(shape, Shape::SmallCircle | Shape::FilledCircle | Shape::FramedCircle | Shape::ForkBar)
}

/// Node size for a label of `tw`×`th` px. `horizontal` = LR/RL flow
/// (fork bars turn vertical).
pub(super) fn node_size(shape: Shape, tw: f32, th: f32, pad: f32, horizontal: bool) -> [f32; 2] {
    match shape {
        Shape::Rect | Shape::Round | Shape::Flag => [tw + 2.0 * pad, th + 2.0 * pad],
        Shape::Subroutine | Shape::LinedRect => [tw + 2.0 * pad + 16.0, th + 2.0 * pad],
        Shape::NotchRect => [tw + 2.0 * pad + 10.0, th + 2.0 * pad],
        Shape::Document => [tw + 2.0 * pad, th + 2.0 * pad + 10.0],
        Shape::Stacked => [tw + 2.0 * pad + 8.0, th + 2.0 * pad + 8.0],
        Shape::Stadium => {
            let h = th + pad * 1.3;
            [tw + h + 10.0, h]
        }
        Shape::Delay => {
            let h = th + 2.0 * pad;
            [tw + 2.0 * pad + h / 2.0, h]
        }
        Shape::Cylinder => {
            let w = tw + 2.0 * pad;
            let ry = cylinder_ry(w);
            [w, th + pad * 1.4 + 2.0 * ry]
        }
        Shape::HCylinder => {
            let h = th + 2.0 * pad;
            [tw + 2.0 * pad + h / 2.0, h]
        }
        Shape::Circle => {
            let d = tw.max(th) + 2.0 * pad * 0.7;
            [d, d]
        }
        Shape::DoubleCircle => {
            let d = tw.max(th) + 2.0 * pad * 0.7 + 10.0;
            [d, d]
        }
        Shape::Cloud => [tw + 2.0 * pad + 24.0, th + 2.0 * pad + 12.0],
        Shape::Diamond => {
            let s = (tw + pad) + (th + pad);
            [s, s]
        }
        Shape::Hexagon => {
            let h = th + pad * 1.3;
            [tw + pad + h / 2.0, h]
        }
        Shape::LeanRight | Shape::LeanLeft | Shape::Trapezoid | Shape::InvTrapezoid => {
            let h = th + pad * 1.3;
            [tw + pad + h, h]
        }
        Shape::Asymmetric => {
            let h = th + pad * 1.3;
            [tw + pad + h / 2.0, h]
        }
        Shape::Triangle | Shape::FlippedTriangle => [(tw + pad) * 2.0, (th + pad) * 2.0],
        Shape::SmallCircle | Shape::FilledCircle => [14.0, 14.0],
        Shape::FramedCircle => [18.0, 18.0],
        Shape::ForkBar => {
            if horizontal {
                [10.0, 70.0]
            } else {
                [70.0, 10.0]
            }
        }
        Shape::Hourglass => [tw.max(30.0) + pad, th.max(30.0) + 2.0 * pad],
        Shape::Brace => [tw + pad + 12.0, th + pad],
        Shape::Bolt => [tw.max(24.0) + 16.0, th.max(40.0) + 12.0],
        Shape::Text => [tw + 8.0, th + 8.0],
    }
}

fn cylinder_ry(w: f32) -> f32 {
    (w / 2.0) / (2.5 + w / 50.0)
}

pub(super) fn clip_shape(shape: Shape) -> ClipShape {
    match shape {
        Shape::Diamond => ClipShape::Diamond,
        Shape::Circle
        | Shape::DoubleCircle
        | Shape::SmallCircle
        | Shape::FramedCircle
        | Shape::FilledCircle
        | Shape::Cloud => ClipShape::Ellipse,
        _ => ClipShape::Rect,
    }
}

/// Offset of the label centre for shapes whose text area is not the
/// geometric centre.
pub(super) fn label_offset(shape: Shape, size: [f32; 2]) -> P {
    match shape {
        Shape::Cylinder => [0.0, cylinder_ry(size[0]) / 2.0],
        Shape::Document => [0.0, -4.0],
        Shape::Triangle => [0.0, size[1] * 0.18],
        Shape::FlippedTriangle => [0.0, -size[1] * 0.18],
        Shape::Asymmetric => [size[1] / 8.0, 0.0],
        Shape::Brace => [6.0, 0.0],
        _ => [0.0, 0.0],
    }
}

fn arc(center: P, r: P, from: f32, to: f32, steps: usize) -> Vec<P> {
    (0..=steps)
        .map(|i| {
            let t = from + (to - from) * i as f32 / steps as f32;
            [center[0] + r[0] * t.cos(), center[1] + r[1] * t.sin()]
        })
        .collect()
}

pub(super) fn draw(scene: &mut Scene, shape: Shape, c: P, size: [f32; 2], fill: Color, stroke: Stroke, line_color: Color) {
    let (w, h) = (size[0], size[1]);
    let (x0, y0, x1, y1) = (c[0] - w / 2.0, c[1] - h / 2.0, c[0] + w / 2.0, c[1] + h / 2.0);
    let st = Some(stroke);
    match shape {
        Shape::Rect | Shape::Flag => scene.rect([x0, y0], size, 0.0, fill, st),
        Shape::Round => scene.rect([x0, y0], size, 5.0, fill, st),
        Shape::Stadium => scene.rect([x0, y0], size, h / 2.0, fill, st),
        Shape::Subroutine => {
            scene.rect([x0, y0], size, 0.0, fill, st);
            scene.line(vec![[x0 + 8.0, y0], [x0 + 8.0, y1]], stroke);
            scene.line(vec![[x1 - 8.0, y0], [x1 - 8.0, y1]], stroke);
        }
        Shape::LinedRect => {
            scene.rect([x0, y0], size, 0.0, fill, st);
            scene.line(vec![[x0 + 8.0, y0], [x0 + 8.0, y1]], stroke);
        }
        Shape::Stacked => {
            for k in [2.0f32, 1.0] {
                let o = 4.0 * k;
                scene.rect([x0 + o, y0 - o + 8.0], [w - 8.0, h - 8.0], 0.0, fill, st);
            }
            scene.rect([x0, y0 + 8.0], [w - 8.0, h - 8.0], 0.0, fill, st);
        }
        Shape::NotchRect => {
            scene.polygon(vec![[x0 + 10.0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0 + 10.0]], fill, st);
        }
        Shape::Document => {
            let mut pts = vec![[x0, y0], [x1, y0], [x1, y1 - 6.0]];
            for i in 0..=16 {
                let t = i as f32 / 16.0;
                pts.push([x1 - t * w, y1 - 6.0 + 6.0 * (t * 2.0 * PI).sin()]);
            }
            scene.polygon(pts, fill, st);
        }
        Shape::Delay => {
            let r = h / 2.0;
            let mut pts = vec![[x0, y0], [x1 - r, y0]];
            pts.extend(arc([x1 - r, c[1]], [r, r], -PI / 2.0, PI / 2.0, 12));
            pts.push([x0, y1]);
            scene.polygon(pts, fill, st);
        }
        Shape::Cylinder => {
            let (rx, ry) = (w / 2.0, cylinder_ry(w));
            let mut body = vec![[x0, y0 + ry], [x0, y1 - ry]];
            body.extend(arc([c[0], y1 - ry], [rx, ry], PI, 0.0, 16));
            body.push([x1, y0 + ry]);
            body.extend(arc([c[0], y0 + ry], [rx, ry], 0.0, -PI, 16));
            scene.polygon(body, fill, st);
            scene.line(arc([c[0], y0 + ry], [rx, ry], PI, 0.0, 16), stroke);
        }
        Shape::HCylinder => {
            let (rx, ry) = (h / 4.0, h / 2.0);
            let mut body = vec![[x0 + rx, y0], [x1 - rx, y0]];
            body.extend(arc([x1 - rx, c[1]], [rx, ry], -PI / 2.0, PI / 2.0, 16));
            body.push([x0 + rx, y1]);
            body.extend(arc([x0 + rx, c[1]], [rx, ry], PI / 2.0, 3.0 * PI / 2.0, 16));
            scene.polygon(body, fill, st);
            scene.line(arc([x1 - rx, c[1]], [rx, ry], PI / 2.0, 3.0 * PI / 2.0, 16), stroke);
        }
        Shape::Circle | Shape::Cloud => {
            scene.push(Prim::Ellipse { center: c, radius: [w / 2.0, h / 2.0], fill, stroke: st });
        }
        Shape::DoubleCircle => {
            scene.push(Prim::Ellipse { center: c, radius: [w / 2.0, h / 2.0], fill, stroke: st });
            scene.push(Prim::Ellipse { center: c, radius: [w / 2.0 - 5.0, h / 2.0 - 5.0], fill, stroke: st });
        }
        Shape::SmallCircle | Shape::FilledCircle => {
            scene.push(Prim::Ellipse { center: c, radius: [w / 2.0, h / 2.0], fill: line_color, stroke: st });
        }
        Shape::FramedCircle => {
            scene.push(Prim::Ellipse { center: c, radius: [w / 2.0, h / 2.0], fill, stroke: st });
            scene.push(Prim::Ellipse {
                center: c,
                radius: [w / 2.0 - 4.0, h / 2.0 - 4.0],
                fill: line_color,
                stroke: None,
            });
        }
        Shape::ForkBar => scene.rect([x0, y0], size, 2.0, line_color, None),
        Shape::Diamond => scene.polygon(vec![[c[0], y0], [x1, c[1]], [c[0], y1], [x0, c[1]]], fill, st),
        Shape::Hexagon => {
            let m = h / 4.0;
            scene.polygon(
                vec![[x0 + m, y0], [x1 - m, y0], [x1, c[1]], [x1 - m, y1], [x0 + m, y1], [x0, c[1]]],
                fill,
                st,
            );
        }
        Shape::LeanRight => {
            let s = h / 2.0;
            scene.polygon(vec![[x0 + s, y0], [x1, y0], [x1 - s, y1], [x0, y1]], fill, st);
        }
        Shape::LeanLeft => {
            let s = h / 2.0;
            scene.polygon(vec![[x0, y0], [x1 - s, y0], [x1, y1], [x0 + s, y1]], fill, st);
        }
        Shape::Trapezoid => {
            let s = h / 2.0;
            scene.polygon(vec![[x0 + s, y0], [x1 - s, y0], [x1, y1], [x0, y1]], fill, st);
        }
        Shape::InvTrapezoid => {
            let s = h / 2.0;
            scene.polygon(vec![[x0, y0], [x1, y0], [x1 - s, y1], [x0 + s, y1]], fill, st);
        }
        Shape::Asymmetric => {
            scene.polygon(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0 + h / 4.0, c[1]]], fill, st);
        }
        Shape::Triangle => scene.polygon(vec![[c[0], y0], [x1, y1], [x0, y1]], fill, st),
        Shape::FlippedTriangle => scene.polygon(vec![[x0, y0], [x1, y0], [c[0], y1]], fill, st),
        Shape::Hourglass => {
            scene.polygon(vec![[x0, y0], [x1, y0], [c[0], c[1]]], fill, st);
            scene.polygon(vec![[c[0], c[1]], [x1, y1], [x0, y1]], fill, st);
        }
        Shape::Brace => {
            let bx = x0 + 6.0;
            let pts = vec![
                [bx + 6.0, y0],
                [bx, y0 + 4.0],
                [bx, c[1] - 4.0],
                [bx - 6.0, c[1]],
                [bx, c[1] + 4.0],
                [bx, y1 - 4.0],
                [bx + 6.0, y1],
            ];
            scene.line(pts, stroke);
        }
        Shape::Bolt => {
            let pts = vec![
                [c[0] + w * 0.15, y0],
                [x0 + w * 0.2, c[1] + h * 0.05],
                [c[0], c[1] + h * 0.05],
                [c[0] - w * 0.15, y1],
                [x1 - w * 0.2, c[1] - h * 0.05],
                [c[0], c[1] - h * 0.05],
            ];
            scene.polygon(pts, fill, st);
        }
        Shape::Text => {}
    }
}
