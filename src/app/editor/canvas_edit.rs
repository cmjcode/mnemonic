//! Inline editor popover for a canvas element (§3.9.2–3.9.3): the text box
//! (Markdown of a bound section, the Mermaid-like rows of an entity /
//! class, a connector label), plus relation controls for connectors (ER
//! cardinalities, UML relation, dashed) and bind / unbind / delete-from-note
//! for boxes. Callers: `app::editor::canvas_surface`.

use egui::{Id, Margin, RichText};
use egui_icons::icons::{ICON_DELETE, ICON_LINK};

use crate::canvas::diagram_kinds::{ClassRelKind, EdgeRelation, ErCardinality};
use crate::canvas::{CanvasDocument, CanvasElement, CanvasElementId};
use crate::i18n::LocaleManager;
use crate::ui::{pal, theme, widgets};

/// What the user did in the popover this frame.
#[derive(Default)]
pub(super) struct EditResult {
    pub(super) text_changed: bool,
    pub(super) meta_changed: bool,
    pub(super) close: bool,
    /// Bind (`true`) / unbind (`false`) the element.
    pub(super) bind: Option<bool>,
    pub(super) delete_from_note: bool,
}

fn card_key(c: ErCardinality) -> &'static str {
    match c {
        ErCardinality::ExactlyOne => "canvas-card-one",
        ErCardinality::ZeroOrOne => "canvas-card-zero-one",
        ErCardinality::OneOrMore => "canvas-card-one-many",
        ErCardinality::ZeroOrMore => "canvas-card-zero-many",
    }
}

fn rel_key(k: ClassRelKind) -> &'static str {
    match k {
        ClassRelKind::Inheritance => "canvas-rel-inheritance",
        ClassRelKind::Composition => "canvas-rel-composition",
        ClassRelKind::Aggregation => "canvas-rel-aggregation",
        ClassRelKind::Association => "canvas-rel-association",
        ClassRelKind::Dependency => "canvas-rel-dependency",
        ClassRelKind::Realization => "canvas-rel-realization",
        ClassRelKind::Link => "canvas-rel-link",
    }
}

/// Connector relation controls. Returns whether anything changed.
fn relation_controls(ui: &mut egui::Ui, tr: &LocaleManager, elem: &mut CanvasElement) -> bool {
    let CanvasElement::Connector { meta, .. } = elem else { return false };
    let t = |k: &str| tr.t(k, &[]);
    let mut changed = false;
    ui.horizontal(|ui| {
        let names = [t("canvas-rel-none"), t("canvas-rel-er"), t("canvas-rel-class")];
        let current = match &meta.relation {
            None => 0,
            Some(EdgeRelation::Er { .. }) => 1,
            Some(EdgeRelation::Class { .. }) => 2,
        };
        let mut pick = current;
        egui::ComboBox::from_id_salt("canvas_rel_kind")
            .selected_text(names[current].clone())
            .show_ui(ui, |ui| {
                for (i, name) in names.iter().enumerate() {
                    ui.selectable_value(&mut pick, i, name.clone());
                }
            });
        if pick != current {
            meta.relation = match pick {
                1 => Some(EdgeRelation::Er {
                    from: ErCardinality::ExactlyOne,
                    to: ErCardinality::ZeroOrMore,
                    identifying: true,
                }),
                2 => Some(EdgeRelation::Class {
                    kind: ClassRelKind::Association,
                    card_from: String::new(),
                    card_to: String::new(),
                }),
                _ => None,
            };
            changed = true;
        }
        changed |= ui.checkbox(&mut meta.dashed, t("canvas-rel-dashed")).changed();
    });
    match &mut meta.relation {
        Some(EdgeRelation::Er { from, to, identifying }) => {
            ui.horizontal(|ui| {
                for (salt, value) in [("er_from", from), ("er_to", to)] {
                    egui::ComboBox::from_id_salt(salt).selected_text(t(card_key(*value))).show_ui(ui, |ui| {
                        for c in ErCardinality::ALL {
                            changed |= ui.selectable_value(value, c, t(card_key(c))).changed();
                        }
                    });
                }
                changed |= ui.checkbox(identifying, t("canvas-rel-identifying")).changed();
            });
        }
        Some(EdgeRelation::Class { kind, card_from, card_to }) => {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("class_rel").selected_text(t(rel_key(*kind))).show_ui(ui, |ui| {
                    for k in ClassRelKind::ALL {
                        changed |= ui.selectable_value(kind, k, t(rel_key(k))).changed();
                    }
                });
                changed |= ui.add(egui::TextEdit::singleline(card_from).desired_width(36.0).hint_text("1")).changed();
                ui.label("→");
                changed |= ui.add(egui::TextEdit::singleline(card_to).desired_width(36.0).hint_text("*")).changed();
            });
        }
        None => {}
    }
    changed
}

/// Shows the popover for `editing_id`; `None` when the element is gone.
pub(super) fn show_edit_popover(
    ctx: &egui::Context,
    canvas: &mut CanvasDocument,
    editing_id: CanvasElementId,
    screen_min: egui::Pos2,
    screen_width: f32,
    tr: &LocaleManager,
) -> Option<EditResult> {
    let elem = canvas.get_element(editing_id)?;
    let mut text_buf = elem.edit_text().unwrap_or_default();
    let bindable = matches!(elem, CanvasElement::StickyNote { .. } | CanvasElement::Shape { .. });
    let is_bound = elem.is_bound();
    let is_segment = elem.binding().is_some_and(|b| b.is_segment() && b.file.is_none());
    let is_connector = matches!(elem, CanvasElement::Connector { .. });
    let hint_key = match elem {
        CanvasElement::Entity { .. } => "canvas-edit-hint-entity",
        CanvasElement::ClassBox { .. } => "canvas-edit-hint-class",
        _ if is_segment => "canvas-edit-hint-section",
        _ if is_bound => "canvas-edit-hint-bound",
        _ => "canvas-edit-hint",
    };
    let rows = text_buf.lines().count().clamp(3, 18);
    let mut out = EditResult { close: ctx.input(|i| i.key_pressed(egui::Key::Escape)), ..Default::default() };

    egui::Area::new(Id::new("canvas_inline_text_edit_area"))
        .fixed_pos(screen_min)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            theme::popover_frame().inner_margin(Margin::same(8)).show(ui, |ui| {
                let width = screen_width.clamp(240.0, 560.0);
                ui.set_max_width(width + 16.0);
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut text_buf)
                        .id(Id::new("canvas_inline_text_edit"))
                        .code_editor()
                        .desired_width(width)
                        .desired_rows(rows),
                );
                if !resp.has_focus() && !resp.lost_focus() {
                    resp.request_focus();
                }
                out.text_changed = resp.changed();
                if is_connector && let Some(e) = canvas.get_element_mut(editing_id) {
                    out.meta_changed = relation_controls(ui, tr, e);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr.t(hint_key, &[])).size(theme::TEXT_XS).color(pal().text_faint));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary_button(ui, None, &tr.t("canvas-edit-done", &[])).clicked() {
                            out.close = true;
                        }
                        if is_segment
                            && widgets::ghost_button(
                                ui,
                                Some(ICON_DELETE.codepoint),
                                &tr.t("canvas-delete-from-note", &[]),
                            )
                            .clicked()
                        {
                            out.delete_from_note = true;
                            out.close = true;
                        }
                        if bindable && !is_segment {
                            let label = if is_bound { tr.t("canvas-unbind", &[]) } else { tr.t("canvas-bind", &[]) };
                            if widgets::ghost_button(ui, Some(ICON_LINK.codepoint), &label).clicked() {
                                out.bind = Some(!is_bound);
                                out.close = true;
                            }
                        }
                    });
                });
            });
        });
    if out.text_changed
        && let Some(e) = canvas.get_element_mut(editing_id)
    {
        e.apply_edit_text(&text_buf);
    }
    Some(out)
}
