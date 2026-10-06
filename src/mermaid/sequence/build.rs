//! Sequence diagram layout + scene (§3.7.6): participant columns spaced so
//! every message text, self-message and note fits between its lifelines
//! (Mermaid's actorMargin/width rules), then a single top-to-bottom sweep
//! over the events that places messages, notes, activations (nested,
//! offset like Mermaid's), block frames with their `else`/`and` sections,
//! `create`d participants mid-diagram and `destroy` crosses. Linear time in
//! the number of events. Callers: `mermaid::render`.

use super::{ActorKind, BlockKind, Event, NotePlace, Sequence};
use crate::mermaid::scene::{Anchor, Hit, Marker, P, Prim, Scene, Stroke};
use crate::mermaid::source::Config;
use crate::mermaid::text::TextMeasure;
use crate::mermaid::theme::{Color, Theme, text_on};

struct Frame {
    kind: BlockKind,
    label: String,
    color: Option<Color>,
    y0: f32,
    y1: f32,
    sections: Vec<(f32, String)>,
    minx: f32,
    maxx: f32,
    depth: usize,
    line: usize,
}

struct Msg {
    y: f32,
    from: f32,
    to: f32,
    text: String,
    dotted: bool,
    head: Option<Marker>,
    both: bool,
    self_loop: bool,
    number: Option<i64>,
    line: usize,
}

/// Grow every open frame to cover `lo..hi`.
fn touch(open: &mut [Frame], lo: f32, hi: f32) {
    for f in open {
        f.minx = f.minx.min(lo);
        f.maxx = f.maxx.max(hi);
    }
}

/// Make the gaps between columns `a..b` sum to at least `need`.
fn need_between(gap: &mut [f32], a: usize, b: usize, need: f32) {
    let (lo, hi) = (a.min(b), a.max(b));
    if hi == lo || hi > gap.len() {
        return;
    }
    let sum: f32 = gap[lo..hi].iter().sum();
    if sum < need {
        let add = (need - sum) / (hi - lo) as f32;
        for g in &mut gap[lo..hi] {
            *g += add;
        }
    }
}

pub fn build(seq: &Sequence, config: &Config, theme: &Theme, measure: &dyn TextMeasure) -> Scene {
    let font = theme.font_size;
    let n = seq.participants.len();
    let min_w = config.f32(&["sequence", "width"]).unwrap_or(150.0).clamp(40.0, 600.0);
    let min_h = config.f32(&["sequence", "height"]).unwrap_or(65.0).clamp(30.0, 300.0);
    let margin = config.f32(&["sequence", "actorMargin"]).unwrap_or(50.0).clamp(10.0, 300.0);
    let msg_gap = config.f32(&["sequence", "messageMargin"]).unwrap_or(30.0).clamp(5.0, 200.0);
    let mirror = config.bool(&["sequence", "mirrorActors"]).unwrap_or(true);
    let text_size = |t: &str| if t.is_empty() { (0.0, 0.0) } else { measure.size(t, font) };

    // Participant boxes.
    let label_sizes: Vec<(f32, f32)> = seq.participants.iter().map(|p| text_size(&p.label)).collect();
    let widths: Vec<f32> = label_sizes.iter().map(|(w, _)| (w + 30.0).max(min_w)).collect();
    let actor_h = label_sizes
        .iter()
        .zip(&seq.participants)
        .map(|((_, h), p)| if p.kind == ActorKind::Participant { h + 24.0 } else { h + 52.0 })
        .fold(min_h, f32::max);

    // Column spacing from message/note widths.
    let mut gap: Vec<f32> = (0..n.saturating_sub(1)).map(|i| (widths[i] + widths[i + 1]) / 2.0 + margin).collect();
    let (mut left_extra, mut right_extra) = (0.0f32, 0.0f32);
    for e in &seq.events {
        match e {
            Event::Message { from, to, text, .. } => {
                let tw = text_size(text).0;
                if from == to {
                    if *from + 1 < n {
                        need_between(&mut gap, *from, from + 1, tw + 50.0 + widths[from + 1] / 2.0);
                    } else {
                        right_extra = right_extra.max(tw + 50.0);
                    }
                } else {
                    need_between(&mut gap, *from, *to, tw + 30.0);
                }
            }
            Event::Note { place, a, b, text, .. } => {
                let nw = (text_size(text).0 + 20.0).max(60.0);
                match (place, b) {
                    (NotePlace::Right, _) => {
                        if a + 1 < n {
                            need_between(&mut gap, *a, a + 1, nw + 20.0 + widths[a + 1] / 2.0);
                        } else {
                            right_extra = right_extra.max(nw + 20.0);
                        }
                    }
                    (NotePlace::Left, _) => {
                        if *a > 0 {
                            need_between(&mut gap, a - 1, *a, nw + 20.0 + widths[a - 1] / 2.0);
                        } else {
                            left_extra = left_extra.max(nw + 20.0);
                        }
                    }
                    (NotePlace::Over, Some(b)) => need_between(&mut gap, *a, *b, nw - 40.0),
                    (NotePlace::Over, None) => {
                        if nw > widths[*a] {
                            if *a > 0 {
                                need_between(&mut gap, a - 1, *a, nw / 2.0 + widths[a - 1] / 2.0 + 10.0);
                            }
                            if a + 1 < n {
                                need_between(&mut gap, *a, a + 1, nw / 2.0 + widths[a + 1] / 2.0 + 10.0);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let _ = right_extra; // `fit` sizes the canvas from what is drawn.
    let mut xs = vec![0.0f32; n];
    if n > 0 {
        xs[0] = left_extra + widths[0] / 2.0;
        for i in 1..n {
            xs[i] = xs[i - 1] + gap[i - 1];
        }
    }

    let group_title_h = if seq.groups.iter().any(|g| !g.title.is_empty()) { font * 1.25 + 10.0 } else { 0.0 };
    let top = group_title_h + if seq.groups.is_empty() { 0.0 } else { 6.0 };
    let mut y = top + actor_h + 24.0;
    let mut box_y: Vec<Option<f32>> =
        seq.participants.iter().map(|p| p.created_at.is_none().then_some(top)).collect();
    let mut life_start: Vec<f32> = vec![top + actor_h; n];
    let mut life_end: Vec<Option<f32>> = vec![None; n];
    let mut active: Vec<Vec<f32>> = vec![Vec::new(); n];
    let mut activations: Vec<(usize, usize, f32, f32)> = Vec::new();
    let mut open: Vec<Frame> = Vec::new();
    let mut frames: Vec<Frame> = Vec::new();
    let mut msgs: Vec<Msg> = Vec::new();
    let mut notes: Vec<([f32; 4], String, usize)> = Vec::new();
    let mut number = seq.autonumber.map(|(s, _)| s);
    let step = seq.autonumber.map_or(1, |(_, s)| s);

    // Where a message leaves/enters a lifeline carrying activation bars.
    let act_offset = |active: &[Vec<f32>], p: usize, towards: f32| -> f32 {
        let k = active[p].len();
        if k == 0 {
            return 0.0;
        }
        let right = 5.0 + (k - 1) as f32 * 5.0;
        if towards >= xs[p] { right } else { -5.0 + (k - 1) as f32 * 5.0 }
    };

    for (idx, e) in seq.events.iter().enumerate() {
        match e {
            Event::Message { from, to, text, dotted, head, both, activate_target, deactivate_source, line } => {
                let th = text_size(text).1;
                y += th + 6.0;
                let (f, t) = (*from, *to);
                let self_loop = f == t;
                let created = seq.participants[t].created_at == Some(idx);
                let fx = xs[f] + act_offset(&active, f, xs[t]);
                let mut tx = xs[t] + if self_loop { 0.0 } else { act_offset(&active, t, xs[f]) };
                if created {
                    box_y[t] = Some(y - actor_h / 2.0);
                    life_start[t] = y + actor_h / 2.0;
                    tx = if xs[t] > xs[f] { xs[t] - widths[t] / 2.0 } else { xs[t] + widths[t] / 2.0 };
                }
                msgs.push(Msg {
                    y,
                    from: fx,
                    to: tx,
                    text: text.clone(),
                    dotted: *dotted,
                    head: *head,
                    both: *both,
                    self_loop,
                    number,
                    line: *line,
                });
                if let Some(nb) = number.as_mut() {
                    *nb += step;
                }
                let tw = text_size(text).0;
                if self_loop {
                    touch(&mut open, fx, fx + 40.0 + tw);
                } else {
                    touch(&mut open, fx.min(tx), fx.max(tx));
                }
                for p in [f, t] {
                    if seq.participants[p].destroyed_at == Some(idx) {
                        life_end[p] = Some(y);
                    }
                }
                if *activate_target {
                    active[t].push(y);
                }
                if *deactivate_source && let Some(y0) = active[f].pop() {
                    activations.push((f, active[f].len(), y0, y));
                }
                y += if self_loop { 24.0 } else { 0.0 } + if created { actor_h / 2.0 } else { 0.0 } + msg_gap - 8.0;
            }
            Event::Note { place, a, b, text, line } => {
                let (tw, th) = text_size(text);
                let nw = (tw + 20.0).max(60.0);
                let nh = th + 16.0;
                let (x0, x1) = match (place, b) {
                    (NotePlace::Right, _) => (xs[*a] + 10.0, xs[*a] + 10.0 + nw),
                    (NotePlace::Left, _) => (xs[*a] - 10.0 - nw, xs[*a] - 10.0),
                    (NotePlace::Over, Some(b)) => {
                        let (lo, hi) = (xs[*a].min(xs[*b]), xs[*a].max(xs[*b]));
                        let w = nw.max(hi - lo + 40.0);
                        let c = (lo + hi) / 2.0;
                        (c - w / 2.0, c + w / 2.0)
                    }
                    (NotePlace::Over, None) => (xs[*a] - nw / 2.0, xs[*a] + nw / 2.0),
                };
                y += 4.0;
                notes.push(([x0, y, x1, y + nh], text.clone(), *line));
                touch(&mut open, x0 - 10.0, x1 + 10.0);
                y += nh + 14.0;
            }
            Event::Activate { who, on, .. } => {
                if *on {
                    active[*who].push(y - msg_gap / 2.0);
                } else if let Some(y0) = active[*who].pop() {
                    activations.push((*who, active[*who].len(), y0, y - msg_gap / 2.0));
                }
            }
            Event::BlockStart { kind, label, color, line } => {
                y += 6.0;
                let header = if *kind == BlockKind::Rect { 4.0 } else { text_size(label).1.max(font * 1.25) + 14.0 };
                open.push(Frame {
                    kind: *kind,
                    label: label.clone(),
                    color: *color,
                    y0: y,
                    y1: y,
                    sections: Vec::new(),
                    minx: f32::MAX,
                    maxx: f32::MIN,
                    depth: open.len(),
                    line: *line,
                });
                y += header + 6.0;
            }
            Event::BlockSection { label, .. } => {
                y += 4.0;
                if let Some(f) = open.last_mut() {
                    f.sections.push((y, label.clone()));
                }
                y += text_size(label).1.max(font * 1.25) + 16.0;
            }
            Event::BlockEnd { .. } => {
                if let Some(mut f) = open.pop() {
                    f.y1 = y + 2.0;
                    // Parents contain their children.
                    if f.minx <= f.maxx {
                        touch(&mut open, f.minx - 10.0, f.maxx + 10.0);
                    }
                    frames.push(f);
                }
                y += 14.0;
            }
        }
    }
    for mut f in open.drain(..) {
        f.y1 = y;
        frames.push(f);
    }
    y += 10.0;
    for (p, stack) in active.iter().enumerate() {
        for (level, y0) in stack.iter().enumerate() {
            activations.push((p, level, *y0, y));
        }
    }
    let bottom = y;

    // ---- Emit ----
    let mut scene = Scene::new(theme.background);
    let bg = theme.background;
    let line_color = theme.line;
    let life_color = [line_color[0], line_color[1], line_color[2], 120];

    for g in &seq.groups {
        let mut members: Vec<usize> = g.members.iter().copied().filter(|&m| m < n).collect();
        members.sort_by(|a, b| xs[*a].total_cmp(&xs[*b]));
        let (Some(&first), Some(&last)) = (members.first(), members.last()) else { continue };
        let x0 = xs[first] - widths[first] / 2.0 - 10.0;
        let x1 = xs[last] + widths[last] / 2.0 + 10.0;
        let fill = g.color.map_or([0, 0, 0, 0], |c| [c[0], c[1], c[2], c[3].min(90)]);
        let y_end = if mirror { bottom + actor_h + 8.0 } else { bottom + 4.0 };
        scene.rect([x0, 0.0], [x1 - x0, y_end], 0.0, fill, Some(Stroke::new(theme.cluster_border, 1.0)));
        if !g.title.is_empty() {
            let pos = [(x0 + x1) / 2.0, group_title_h / 2.0 + 2.0];
            scene.text(measure, pos, &g.title, font, theme.text, Anchor::Middle, true);
        }
    }

    let frame_x = |f: &Frame| -> (f32, f32) {
        let pad = (40.0 - 8.0 * f.depth as f32).max(12.0);
        if f.minx <= f.maxx {
            (f.minx - pad, f.maxx + pad)
        } else if n > 0 {
            (xs[0] - pad, xs[n - 1] + pad)
        } else {
            (0.0, 100.0)
        }
    };
    // Filled `rect` blocks sit behind everything else.
    for f in frames.iter().filter(|f| f.kind == BlockKind::Rect) {
        let (x0, x1) = frame_x(f);
        let fill = f.color.unwrap_or([200, 200, 200, 80]);
        scene.rect([x0, f.y0], [x1 - x0, f.y1 - f.y0], 0.0, fill, None);
    }

    for i in 0..n {
        let end = life_end[i].unwrap_or(if mirror { bottom } else { bottom + 4.0 });
        scene.line(vec![[xs[i], life_start[i]], [xs[i], end]], Stroke::new(life_color, 1.0));
    }

    for &(p, level, y0, y1) in &activations {
        let x = xs[p] - 5.0 + level as f32 * 5.0;
        let border = Some(Stroke::new(theme.primary_border, 1.0));
        scene.rect([x, y0], [10.0, (y1 - y0).max(4.0)], 0.0, theme.tertiary, border);
    }

    for f in frames.iter().filter(|f| f.kind != BlockKind::Rect) {
        let (x0, x1) = frame_x(f);
        let border = Stroke::dashed(theme.primary_border, 1.3, [4.0, 3.0]);
        scene.rect([x0, f.y0], [x1 - x0, f.y1 - f.y0], 0.0, [0, 0, 0, 0], Some(border));
        let kw = f.kind.keyword();
        let (kw_w, kw_h) = text_size(kw);
        let (tw, th) = (kw_w + 16.0, kw_h + 8.0);
        scene.polygon(
            vec![[x0, f.y0], [x0 + tw, f.y0], [x0 + tw, f.y0 + th - 6.0], [x0 + tw - 7.0, f.y0 + th], [x0, f.y0 + th]],
            theme.primary,
            Some(Stroke::new(theme.primary_border, 1.0)),
        );
        let kw_pos = [x0 + tw / 2.0 - 3.0, f.y0 + th / 2.0];
        scene.text(measure, kw_pos, kw, font * 0.85, theme.primary_text, Anchor::Middle, true);
        if !f.label.is_empty() {
            let text = format!("[{}]", f.label);
            let cx = ((x0 + tw + x1) / 2.0).max(x0 + tw + text_size(&text).0 / 2.0 + 6.0);
            scene.text(measure, [cx, f.y0 + th / 2.0], &text, font * 0.9, theme.text, Anchor::Middle, false);
        }
        for (sy, label) in &f.sections {
            scene.line(vec![[x0, *sy], [x1, *sy]], Stroke::dashed(theme.primary_border, 1.0, [4.0, 3.0]));
            if !label.is_empty() {
                let pos = [(x0 + x1) / 2.0, sy + text_size(label).1 / 2.0 + 6.0];
                scene.text(measure, pos, &format!("[{label}]"), font * 0.9, theme.text, Anchor::Middle, false);
            }
        }
        scene.hit([x0, f.y0, x0 + tw, f.y0 + th], kw, f.line);
    }

    for (k, m) in msgs.iter().enumerate() {
        let stroke = if m.dotted { Stroke::dashed(line_color, 1.5, [4.0, 3.0]) } else { Stroke::new(line_color, 1.5) };
        let (tw, th) = text_size(&m.text);
        if m.self_loop {
            let pts: Vec<P> =
                vec![[m.from, m.y], [m.from + 34.0, m.y], [m.from + 34.0, m.y + 22.0], [m.from + 4.0, m.y + 22.0]];
            scene.edge(pts, stroke, None, m.head, bg);
            scene.text(measure, [m.from + 8.0, m.y - th / 2.0 - 3.0], &m.text, font, theme.text, Anchor::Start, false);
            scene.hit([m.from, m.y - th - 4.0, m.from + 40.0 + tw, m.y + 24.0], format!("message{k}"), m.line);
        } else {
            let start = if m.both { m.head } else { None };
            scene.edge(vec![[m.from, m.y], [m.to, m.y]], stroke, start, m.head, bg);
            let cx = (m.from + m.to) / 2.0;
            scene.text(measure, [cx, m.y - th / 2.0 - 3.0], &m.text, font, theme.text, Anchor::Middle, false);
            let (lo, hi) = (m.from.min(m.to), m.from.max(m.to));
            let rect = [lo.min(cx - tw / 2.0), m.y - th - 4.0, hi.max(cx + tw / 2.0), m.y + 4.0];
            scene.hit(rect, format!("message{k}"), m.line);
        }
        if let Some(nb) = m.number {
            scene.push(Prim::Ellipse { center: [m.from, m.y], radius: [9.0, 9.0], fill: line_color, stroke: None });
            scene.text(measure, [m.from, m.y], &nb.to_string(), 11.0, text_on(line_color), Anchor::Middle, true);
        }
    }

    for (r, text, line) in &notes {
        let border = Some(Stroke::new(theme.note_border, 1.0));
        scene.rect([r[0], r[1]], [r[2] - r[0], r[3] - r[1]], 0.0, theme.note_bkg, border);
        let pos = [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
        scene.text(measure, pos, text, font, theme.note_text, Anchor::Middle, false);
        scene.hit(*r, "note", *line);
    }

    for (i, p) in seq.participants.iter().enumerate() {
        let w = widths[i];
        let mut tops = Vec::new();
        if let Some(ty) = box_y[i] {
            tops.push(ty);
        }
        if mirror && life_end[i].is_none() {
            tops.push(bottom);
        }
        for ty in tops {
            draw_participant(&mut scene, measure, p.kind, &p.label, [xs[i], ty], [w, actor_h], theme, font);
            scene.hits.push(Hit {
                rect: [xs[i] - w / 2.0, ty, xs[i] + w / 2.0, ty + actor_h],
                id: p.id.clone(),
                line: p.line,
                link: None,
                tooltip: None,
            });
        }
        if let Some(ey) = life_end[i] {
            let s = Stroke::new(line_color, 2.0);
            scene.line(vec![[xs[i] - 9.0, ey - 9.0], [xs[i] + 9.0, ey + 9.0]], s);
            scene.line(vec![[xs[i] + 9.0, ey - 9.0], [xs[i] - 9.0, ey + 9.0]], s);
        }
    }

    if let Some(t) = seq.title.as_deref().filter(|t| !t.is_empty())
        && let Some(b) = scene.bounds()
    {
        scene.text(measure, [(b[0] + b[2]) / 2.0, b[1] - font * 1.4], t, font * 1.15, theme.text, Anchor::Middle, true);
    }
    scene.fit(8.0);
    scene
}

#[allow(clippy::too_many_arguments)]
fn draw_participant(
    scene: &mut Scene,
    measure: &dyn TextMeasure,
    kind: ActorKind,
    label: &str,
    top_center: P,
    size: [f32; 2],
    theme: &Theme,
    font: f32,
) {
    let (cx, y0) = (top_center[0], top_center[1]);
    let (w, h) = (size[0], size[1]);
    let stroke = Some(Stroke::new(theme.primary_border, 1.2));
    let s = Stroke::new(theme.primary_border, 1.4);
    let label_below = |scene: &mut Scene| {
        let th = measure.size(label, font).1;
        scene.text(measure, [cx, y0 + h - th / 2.0 - 2.0], label, font, theme.text, Anchor::Middle, false);
    };
    match kind {
        ActorKind::Participant => {
            scene.rect([cx - w / 2.0, y0], [w, h], 3.0, theme.primary, stroke);
            scene.text(measure, [cx, y0 + h / 2.0], label, font, theme.primary_text, Anchor::Middle, false);
        }
        ActorKind::Actor => {
            scene.push(Prim::Ellipse { center: [cx, y0 + 9.0], radius: [7.5, 7.5], fill: theme.primary, stroke: Some(s) });
            scene.line(vec![[cx, y0 + 16.5], [cx, y0 + 32.0]], s);
            scene.line(vec![[cx - 12.0, y0 + 22.0], [cx + 12.0, y0 + 22.0]], s);
            scene.line(vec![[cx - 10.0, y0 + 44.0], [cx, y0 + 32.0], [cx + 10.0, y0 + 44.0]], s);
            label_below(scene);
        }
        ActorKind::Database => {
            let (rx, ry, top, bot) = (16.0, 5.0, y0 + 4.0, y0 + 40.0);
            scene.rect([cx - rx, top + ry], [2.0 * rx, bot - top - ry], 0.0, theme.primary, None);
            scene.line(vec![[cx - rx, top + ry], [cx - rx, bot]], s);
            scene.line(vec![[cx + rx, top + ry], [cx + rx, bot]], s);
            scene.push(Prim::Ellipse { center: [cx, bot], radius: [rx, ry], fill: theme.primary, stroke });
            scene.push(Prim::Ellipse { center: [cx, top + ry], radius: [rx, ry], fill: theme.primary, stroke });
            label_below(scene);
        }
        ActorKind::Boundary | ActorKind::Control | ActorKind::Entity => {
            let c = [cx + if kind == ActorKind::Boundary { 6.0 } else { 0.0 }, y0 + 22.0];
            scene.push(Prim::Ellipse { center: c, radius: [15.0, 15.0], fill: theme.primary, stroke });
            match kind {
                ActorKind::Boundary => {
                    scene.line(vec![[cx - 20.0, y0 + 10.0], [cx - 20.0, y0 + 34.0]], s);
                    scene.line(vec![[cx - 20.0, c[1]], [c[0] - 15.0, c[1]]], s);
                }
                ActorKind::Control => scene.line(vec![[cx + 5.0, y0 + 3.0], [cx, y0 + 7.0], [cx + 5.0, y0 + 11.0]], s),
                _ => scene.line(vec![[cx - 15.0, y0 + 40.0], [cx + 15.0, y0 + 40.0]], s),
            }
            label_below(scene);
        }
        ActorKind::Collections => {
            scene.rect([cx - w / 2.0 + 6.0, y0 - 6.0], [w - 6.0, h], 3.0, theme.primary, stroke);
            scene.rect([cx - w / 2.0, y0], [w - 6.0, h], 3.0, theme.primary, stroke);
            scene.text(measure, [cx - 3.0, y0 + h / 2.0], label, font, theme.primary_text, Anchor::Middle, false);
        }
        ActorKind::Queue => {
            scene.rect([cx - w / 2.0, y0], [w, h], h / 2.0, theme.primary, stroke);
            scene.text(measure, [cx, y0 + h / 2.0], label, font, theme.primary_text, Anchor::Middle, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::scene::prim_bounds;
    use crate::mermaid::sequence::parse;
    use crate::mermaid::source::preprocess;
    use crate::mermaid::text::ApproxMeasure;

    fn scene(src: &str) -> Scene {
        let s = preprocess(src);
        let (seq, d) = parse(&s);
        assert!(d.iter().all(|x| !x.is_error()), "{d:?}");
        build(&seq, &s.config, &Theme::default_theme(), &ApproxMeasure)
    }

    #[test]
    fn lays_out_columns_messages_and_mirrored_actors() {
        let sc = scene("sequenceDiagram\n  Alice->>John: Hello John, how are you doing today my friend?\n  John-->>Alice: Great!\n  Alice-)John: See you later!\n");
        let alice: Vec<&Hit> = sc.hits.iter().filter(|h| h.id == "Alice").collect();
        assert_eq!(alice.len(), 2, "top and bottom boxes");
        let john = sc.hits.iter().find(|h| h.id == "John").unwrap();
        let gap = (john.rect[0] + john.rect[2]) / 2.0 - (alice[0].rect[0] + alice[0].rect[2]) / 2.0;
        let long = ApproxMeasure.size("Hello John, how are you doing today my friend?", 16.0).0;
        assert!(gap >= long, "columns widen to fit message text: {gap} < {long}");
        assert_eq!(sc.hits.iter().filter(|h| h.id.starts_with("message")).count(), 3);
        for p in &sc.prims {
            let b = prim_bounds(p);
            assert!(b[0] >= -0.5 && b[2] <= sc.width + 0.5 && b[3] <= sc.height + 0.5);
        }
    }

    #[test]
    fn draws_blocks_notes_activations_and_numbers() {
        let sc = scene("sequenceDiagram\n  autonumber\n  participant A\n  participant B\n  A->>+B: start\n  loop Every minute\n    B->>A: tick\n    Note right of A: note\n  end\n  alt yes\n    A->>B: ok\n  else no\n    A->>B: nope\n  end\n  B-->>-A: done\n  A->>A: self\n");
        assert!(sc.hits.iter().any(|h| h.id == "loop"));
        assert!(sc.hits.iter().any(|h| h.id == "alt"));
        assert!(sc.hits.iter().any(|h| h.id == "note"));
        let numbers: Vec<&str> = sc
            .prims
            .iter()
            .filter_map(|p| if let Prim::Text { text, size, .. } = p { (*size == 11.0).then_some(text.as_str()) } else { None })
            .collect();
        assert_eq!(numbers, vec!["1", "2", "3", "4", "5", "6"]);
        // One activation bar (A->>+B … B-->>-A).
        let bars = sc.prims.iter().filter(|p| matches!(p, Prim::Rect { size, .. } if size[0] == 10.0)).count();
        assert_eq!(bars, 1);
    }
}
