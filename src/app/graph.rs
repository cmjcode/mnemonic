//! Graph view: draws a `graph::GraphData` with its force-directed
//! `graph::Layout` on an infinite pan/zoom surface (reusing the canvas
//! `Viewport`). Hovering a node highlights its neighbors, dragging moves
//! it, clicking opens it. The same widget renders the full-screen vault
//! graph and the compact "local graph" in the editor's side panel.

use std::collections::{HashMap, HashSet};

use egui::{Align2, Color32, FontId, Id, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use egui_icons::icons::ICON_FIT_SCREEN;

use super::MnemonicApp;
use crate::canvas::Viewport;
use crate::graph::{EdgeKind, ForceParams, GraphData, GraphOptions, Layout, NodeKind};
use crate::i18n::LocaleManager;
use crate::ui::{pal, theme, widgets};

/// Graphs up to this many nodes are pre-settled before the first frame so
/// they open already arranged; bigger ones animate into place.
const PRESETTLE_MAX_NODES: usize = 800;
const PRESETTLE_STEPS: usize = 400;
/// Zoom level above which every node shows its label.
const LABEL_ZOOM: f32 = 0.9;

pub(super) struct GraphView {
    pub(super) data: GraphData,
    pub(super) opts: GraphOptions,
    layout: Layout,
    viewport: Viewport,
    params: ForceParams,
    adjacency: Vec<HashSet<usize>>,
    hovered: Option<usize>,
    dragging: Option<usize>,
    /// Highlighted node (the open note, in the local graph).
    focus: Option<usize>,
    filter: String,
    needs_fit: bool,
    /// The user panned/zoomed; stop auto-fitting while the layout settles.
    view_touched: bool,
    compact: bool,
}

#[derive(Default)]
pub(super) struct GraphOutcome {
    /// Node the user clicked.
    pub(super) open: Option<usize>,
    /// Display options changed; the caller should rebuild `data`.
    pub(super) options_changed: bool,
}

impl GraphView {
    pub(super) fn new(
        data: GraphData,
        opts: GraphOptions,
        saved: &HashMap<String, [f32; 2]>,
        compact: bool,
    ) -> GraphView {
        let mut layout = Layout::for_graph(&data, |i| saved.get(&data.nodes[i].key).copied());
        let params = if compact {
            ForceParams {
                rest_length: 55.0,
                ..ForceParams::default()
            }
        } else {
            ForceParams::default()
        };
        if data.nodes.len() <= PRESETTLE_MAX_NODES {
            layout.settle(&data, &params, PRESETTLE_STEPS);
        }
        let adjacency = adjacency_sets(&data);
        GraphView {
            data,
            opts,
            layout,
            viewport: Viewport::default(),
            params,
            adjacency,
            hovered: None,
            dragging: None,
            focus: None,
            filter: String::new(),
            needs_fit: true,
            view_touched: false,
            compact,
        }
    }

    /// Replaces the data (after an index change or option toggle), keeping
    /// every surviving node where it was.
    pub(super) fn replace_data(&mut self, data: GraphData, opts: GraphOptions) {
        let saved: HashMap<String, [f32; 2]> = self.positions();
        let known = data.nodes.iter().filter(|n| saved.contains_key(&n.key)).count();
        self.layout = Layout::for_graph(&data, |i| saved.get(&data.nodes[i].key).copied());
        if known < data.nodes.len() {
            self.layout.reheat();
        }
        self.adjacency = adjacency_sets(&data);
        self.data = data;
        self.opts = opts;
        self.hovered = None;
        self.dragging = None;
        self.focus = None;
        if self.compact {
            self.view_touched = false;
        }
    }

    pub(super) fn set_focus(&mut self, key: &str) {
        self.focus = self.data.index_of(key);
    }

    /// Current node positions by node key (for persisting the layout).
    pub(super) fn positions(&self) -> HashMap<String, [f32; 2]> {
        self.data
            .nodes
            .iter()
            .zip(&self.layout.positions)
            .map(|(n, p)| (n.key.clone(), *p))
            .collect()
    }

    fn fit(&mut self, size: Vec2) {
        if let Some([x0, y0, x1, y1]) = self.layout.bounds() {
            let pad = 60.0;
            let rect = Rect::from_min_max(
                Pos2::new(x0 - pad, y0 - pad),
                Pos2::new(x1.max(x0 + 1.0) + pad, y1.max(y0 + 1.0) + pad),
            );
            self.viewport.fit_rect(rect, size);
            self.viewport.zoom = self.viewport.zoom.min(1.6);
            let c = rect.center();
            self.viewport.pan = [
                size.x / (2.0 * self.viewport.zoom) - c.x,
                size.y / (2.0 * self.viewport.zoom) - c.y,
            ];
        }
    }

    fn node_radius(&self, i: usize) -> f32 {
        let base = if self.compact { 3.5 } else { 4.5 };
        base + (self.data.nodes[i].degree as f32).sqrt() * 2.2
    }

    /// Renders the graph filling `size` and handles interaction.
    pub(super) fn show(&mut self, ui: &mut egui::Ui, tr: &LocaleManager, size: Vec2) -> GraphOutcome {
        let mut outcome = GraphOutcome::default();
        let p = pal();
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;
        let origin = rect.min;
        painter.rect_filled(rect, 0.0, p.bg);

        // ── Simulation ──
        let settling = !self.layout.is_settled();
        if settling || self.dragging.is_some() {
            let steps = if self.data.nodes.len() > 1500 { 1 } else { 2 };
            for _ in 0..steps {
                self.layout.step(&self.data, &self.params);
            }
            ui.ctx().request_repaint();
        }
        // Keep the whole graph framed while it is still moving, until the
        // user takes over the view.
        if rect.width() > 1.0 && (self.needs_fit || (settling && !self.view_touched)) {
            self.fit(rect.size());
            self.needs_fit = false;
        }

        // ── Pan & zoom ──
        if response.contains_pointer() {
            let (scroll, zoom_delta, command) =
                ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers.command));
            if let Some(pos) = response.hover_pos() {
                if (zoom_delta - 1.0).abs() > 1e-4 {
                    self.viewport.zoom_at(zoom_delta, pos, origin);
                    self.view_touched = true;
                } else if command && scroll.y != 0.0 {
                    self.viewport
                        .zoom_at(if scroll.y > 0.0 { 1.1 } else { 0.9 }, pos, origin);
                    self.view_touched = true;
                } else if scroll != Vec2::ZERO && !self.compact {
                    // In the side panel, plain scrolling scrolls the panel.
                    self.viewport.add_pan_vec(scroll / self.viewport.zoom);
                    self.view_touched = true;
                }
            }
        }

        // ── Hit testing ──
        let zoom = self.viewport.zoom;
        let screen: Vec<Pos2> = self
            .layout
            .positions
            .iter()
            .map(|q| self.viewport.world_to_screen(Pos2::new(q[0], q[1]), origin))
            .collect();
        let pointer = response.hover_pos();
        if self.dragging.is_none() {
            self.hovered = pointer.and_then(|pos| {
                (0..screen.len())
                    .map(|i| (i, screen[i].distance(pos)))
                    .filter(|(i, d)| *d <= (self.node_radius(*i) * zoom).max(5.0) + 3.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i)
            });
        }

        // ── Dragging ──
        if response.drag_started() {
            self.dragging = self.hovered;
            if let Some(i) = self.dragging {
                self.layout.pinned[i] = true;
                self.layout.reheat();
            }
        }
        if response.dragged() {
            let delta = response.drag_delta() / zoom;
            match self.dragging {
                Some(i) => {
                    self.layout.positions[i][0] += delta.x;
                    self.layout.positions[i][1] += delta.y;
                }
                None => {
                    self.viewport.add_pan_vec(delta);
                    self.view_touched = true;
                }
            }
        }
        if response.drag_stopped()
            && let Some(i) = self.dragging.take()
        {
            self.layout.pinned[i] = false;
            self.layout.reheat();
        }
        if response.clicked() {
            outcome.open = self.hovered;
        }
        if self.hovered.is_some() {
            ui.ctx().set_cursor_icon(if self.dragging.is_some() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::PointingHand
            });
        }

        // ── Highlight sets ──
        let filter = self.filter.trim().to_lowercase();
        let matches: Option<HashSet<usize>> = (!filter.is_empty()).then(|| {
            (0..self.data.nodes.len())
                .filter(|&i| self.data.nodes[i].label.to_lowercase().contains(&filter))
                .collect()
        });
        let spotlight = self.hovered.or(self.dragging);
        let lit = |i: usize| -> bool {
            match (spotlight, &matches) {
                (Some(h), _) => i == h || self.adjacency[h].contains(&i),
                (None, Some(m)) => m.contains(&i),
                (None, None) => true,
            }
        };

        // ── Edges ──
        let visible = rect.expand(40.0);
        for e in &self.data.edges {
            let (a, b) = (screen[e.a], screen[e.b]);
            if !visible.contains(a) && !visible.contains(b) && !visible.intersects(Rect::from_two_pos(a, b)) {
                continue;
            }
            let active = spotlight.is_some_and(|h| e.a == h || e.b == h);
            let dim = !(lit(e.a) && lit(e.b));
            let (color, width) = match (e.kind, active) {
                (_, true) => (p.accent, 1.6),
                (EdgeKind::Link, false) => (p.border_strong, 1.0),
                (EdgeKind::Semantic, false) => (p.accent.gamma_multiply(0.55), 1.0),
            };
            let color = if dim { color.gamma_multiply(0.18) } else { color };
            let stroke = Stroke::new(width, color);
            match e.kind {
                EdgeKind::Link => {
                    painter.line_segment([a, b], stroke);
                }
                EdgeKind::Semantic => {
                    painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 5.0, 4.0));
                }
            }
        }

        // ── Nodes & labels ──
        let font = FontId::proportional(if self.compact { 10.5 } else { 12.0 });
        let label_all = zoom >= LABEL_ZOOM || (self.compact && self.data.nodes.len() <= 12);
        for (i, node) in self.data.nodes.iter().enumerate() {
            let pos = screen[i];
            if !visible.contains(pos) {
                continue;
            }
            let r = (self.node_radius(i) * zoom).clamp(2.0, 28.0);
            let mut color = node_color(node.kind, node.tag.as_deref(), p.is_dark);
            let is_lit = lit(i);
            if !is_lit {
                color = color.gamma_multiply(0.2);
            }
            if self.focus == Some(i) {
                painter.circle_stroke(pos, r + 3.0, Stroke::new(2.0, p.accent));
            }
            if node.kind == NodeKind::Ghost {
                painter.circle_stroke(pos, r, Stroke::new(1.2, color));
            } else {
                painter.circle_filled(pos, r, color);
            }
            if spotlight == Some(i) {
                painter.circle_stroke(pos, r + 2.0, Stroke::new(1.5, p.text));
            }
            let show_label = spotlight == Some(i)
                || self.focus == Some(i)
                || (is_lit && (label_all || spotlight.is_some() || matches.is_some()));
            if show_label {
                let text_color = if is_lit { p.text_dim } else { p.text_faint.gamma_multiply(0.4) };
                painter.text(
                    pos + Vec2::new(0.0, r + 3.0),
                    Align2::CENTER_TOP,
                    widgets::truncate_chars(&node.label, 40),
                    font.clone(),
                    text_color,
                );
            }
        }

        if self.data.nodes.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                tr.t("graph-empty", &[]),
                FontId::proportional(theme::TEXT_SM),
                p.text_faint,
            );
        }

        if let Some(i) = self.hovered.filter(|_| !self.compact) {
            response.clone().on_hover_text_at_pointer(tooltip(tr, &self.data, i));
        }

        if !self.compact {
            outcome.options_changed = self.controls(ui, tr, rect);
        }
        outcome
    }

    /// Floating filter/options panel in the graph's top-left corner.
    /// Returns true when a data-affecting option changed.
    fn controls(&mut self, ui: &mut egui::Ui, tr: &LocaleManager, rect: Rect) -> bool {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let before = self.opts;
        let mut fit = false;
        egui::Area::new(Id::new("graph_controls"))
            .fixed_pos(rect.min + Vec2::new(12.0, 12.0))
            .order(egui::Order::Middle)
            .show(ui.ctx(), |ui| {
                theme::popover_frame()
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(220.0);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filter)
                                .hint_text(RichText::new(t("graph-filter")).color(p.text_faint))
                                .desired_width(f32::INFINITY),
                        );
                        ui.checkbox(&mut self.opts.show_orphans, t("graph-show-orphans"));
                        ui.checkbox(&mut self.opts.show_ghosts, t("graph-show-ghosts"));
                        ui.checkbox(&mut self.opts.show_pdfs, t("graph-show-pdfs"));
                        ui.checkbox(&mut self.opts.show_semantic, t("graph-show-semantic"))
                            .on_hover_text(t("graph-show-semantic-hint"));
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(tr.t(
                                    "graph-counts",
                                    &[
                                        ("nodes", &self.data.nodes.len().to_string()),
                                        ("edges", &self.data.edges.len().to_string()),
                                    ],
                                ))
                                .size(theme::TEXT_XS)
                                .color(p.text_faint),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if widgets::icon_button(ui, ICON_FIT_SCREEN.codepoint, &t("graph-fit"), false)
                                    .clicked()
                                {
                                    fit = true;
                                }
                            });
                        });
                    });
            });
        if fit {
            self.fit(rect.size());
            self.view_touched = false;
        }
        before != self.opts
    }
}

fn adjacency_sets(data: &GraphData) -> Vec<HashSet<usize>> {
    data.adjacency()
        .into_iter()
        .map(|v| v.into_iter().collect())
        .collect()
}

fn tooltip(tr: &LocaleManager, data: &GraphData, i: usize) -> String {
    let node = &data.nodes[i];
    let kind = match node.kind {
        NodeKind::Note => tr.t("graph-kind-note", &[]),
        NodeKind::Canvas => tr.t("graph-kind-canvas", &[]),
        NodeKind::Pdf => tr.t("graph-kind-pdf", &[]),
        NodeKind::Ghost => tr.t("graph-kind-ghost", &[]),
    };
    let links = tr.t("graph-node-links", &[("count", &node.degree.to_string())]);
    match &node.tag {
        Some(tag) => format!("{}\n{kind} · {links} · #{tag}", node.label),
        None => format!("{}\n{kind} · {links}", node.label),
    }
}

/// Notes are colored by their first tag (stable hue per tag), other kinds
/// by type.
fn node_color(kind: NodeKind, tag: Option<&str>, is_dark: bool) -> Color32 {
    let p = pal();
    match (kind, tag) {
        (NodeKind::Pdf, _) => p.pdf_icon,
        (NodeKind::Canvas, _) => p.canvas_icon,
        (NodeKind::Ghost, _) => p.text_faint,
        (NodeKind::Note, Some(tag)) => {
            let hash = tag
                .to_lowercase()
                .bytes()
                .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
            let hue = (hash % 360) as f32 / 360.0;
            let value = if is_dark { 0.85 } else { 0.7 };
            egui::ecolor::Hsva::new(hue, 0.5, value, 1.0).into()
        }
        (NodeKind::Note, None) => p.text_dim,
    }
}

impl MnemonicApp {
    /// Full-screen vault graph (top bar, ⌘G, command palette).
    pub(super) fn show_graph(&mut self, ui: &mut egui::Ui) {
        if self.graph_dirty {
            self.graph_dirty = false;
            if let Some(opts) = self.graph.as_ref().map(|g| g.opts) {
                let data = self.build_graph_data(opts);
                if let Some(view) = self.graph.as_mut() {
                    view.replace_data(data, opts);
                }
            }
        }
        let Some(mut view) = self.graph.take() else {
            return;
        };
        let outcome = view.show(ui, &self.locales, ui.available_size());
        if outcome.options_changed {
            let data = self.build_graph_data(view.opts);
            let opts = view.opts;
            view.replace_data(data, opts);
        }
        let open = outcome.open.map(|i| view.data.nodes[i].clone());
        self.graph = Some(view);
        if let Some(node) = open {
            self.open_graph_node(&node);
        }
    }

    pub(super) fn open_graph_view(&mut self) {
        self.close_document();
        if self.editor.is_some() || self.vault.is_none() {
            return;
        }
        let opts = GraphOptions::default();
        let data = self.build_graph_data(opts);
        let saved = self
            .index
            .as_ref()
            .and_then(|i| i.load_graph_layout().ok())
            .unwrap_or_default();
        self.graph = Some(GraphView::new(data, opts, &saved, false));
        self.graph_dirty = false;
    }

    /// Saves the layout and leaves the graph screen.
    pub(super) fn close_graph_view(&mut self) {
        if let Some(view) = self.graph.take()
            && let Some(index) = self.index.as_mut()
        {
            let positions: Vec<(String, [f32; 2])> = view.positions().into_iter().collect();
            if let Err(e) = index.save_graph_layout(&positions) {
                log::warn!("app: saving graph layout failed: {e:#}");
            }
        }
    }

    pub(super) fn open_graph_node(&mut self, node: &crate::graph::GraphNode) {
        match (&node.kind, &node.path) {
            (NodeKind::Ghost, _) => self.navigate_wikilink(&node.label),
            (_, Some(path)) => {
                let path = path.clone();
                self.close_graph_view();
                self.open_file_by_path(path);
            }
            _ => {}
        }
    }

    /// Nodes/edges for the current vault. Semantic edges come from each
    /// document's nearest neighbors by mean embedding.
    pub(super) fn build_graph_data(&self, opts: GraphOptions) -> GraphData {
        let (Some(vault), Some(index)) = (self.vault.as_ref(), self.index.as_ref()) else {
            return GraphData::default();
        };
        let links = index.link_edges().unwrap_or_else(|e| {
            log::warn!("app: loading link graph failed: {e:#}");
            Vec::new()
        });
        let mut semantic = Vec::new();
        if opts.show_semantic {
            let docs = vault
                .notes
                .iter()
                .filter(|n| !n.frontmatter.trashed)
                .map(|n| n.frontmatter.id)
                .chain(self.pdf_documents.iter().map(|p| crate::core::ingestion::pdf_doc_id(p)));
            for id in docs {
                match index.similar_documents(id, super::SEMANTIC_NEIGHBORS) {
                    Ok(similar) => semantic.extend(
                        similar
                            .into_iter()
                            .filter(|(_, sim)| *sim >= crate::core::embedding::RELATED_DOC_SIMILARITY)
                            .map(|(other, sim)| (id, other, sim)),
                    ),
                    Err(e) => {
                        log::warn!("app: similarity lookup failed: {e:#}");
                        break;
                    }
                }
            }
        }
        GraphData::build(&vault.notes, &self.pdf_documents, &links, &semantic, opts)
    }
}
