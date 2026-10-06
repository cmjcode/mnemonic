//! The editor's right sidebar (Obsidian's right panes): local graph,
//! linked & unlinked mentions, outgoing links, outline, properties and
//! AI-related notes (§3.2.2 backlinks/outline). Pure presentation: every
//! click becomes a `PanelAction` applied by `MnemonicApp::apply_panel_action`.
//! Callers: `app::editor::show_editor`.

use std::collections::HashSet;

use egui::{FontId, Id, Margin, RichText, Vec2};
use egui_icons::icons::{
    ICON_ADD_LINK, ICON_CHEVRON_RIGHT, ICON_CLOSE, ICON_DESCRIPTION, ICON_EXPAND_MORE, ICON_HUB,
    ICON_LINK, ICON_PICTURE_AS_PDF,
};

use super::{LOCAL_GRAPH_HEIGHT, LinksPanel, PanelAction};
use crate::graph::GraphNode;
use crate::i18n::LocaleManager;
use crate::markdown::wikilink;
use crate::ui::{pal, theme, widgets};

/// One Obsidian-style pane header: chevron, uppercase title, count.
/// Returns whether the section is open.
fn pane_header(
    ui: &mut egui::Ui,
    collapsed: &mut HashSet<&'static str>,
    key: &'static str,
    title: &str,
    count: Option<usize>,
) -> bool {
    let p = pal();
    let open = !collapsed.contains(key);
    ui.add_space(theme::SPACE_M);
    let resp = ui
        .horizontal(|ui| {
            ui.add_space(4.0);
            let icon = if open { ICON_EXPAND_MORE.codepoint } else { ICON_CHEVRON_RIGHT.codepoint };
            ui.label(RichText::new(icon).size(14.0).color(p.text_faint));
            ui.label(
                RichText::new(title.to_uppercase())
                    .font(theme::semibold(11.0))
                    .color(p.text_faint),
            );
            if let Some(n) = count {
                ui.label(RichText::new(n.to_string()).size(11.0).color(p.text_faint));
            }
        })
        .response
        .interact(egui::Sense::click());
    if resp.clicked() {
        if open {
            collapsed.insert(key);
        } else {
            collapsed.remove(key);
        }
    }
    ui.add_space(2.0);
    !collapsed.contains(key)
}

/// The editor's right sidebar, in Obsidian's order: local graph, linked
/// & unlinked mentions, outgoing links, outline, properties, related.
#[allow(clippy::too_many_arguments)]
pub(super) fn right_panel(
    ui: &mut egui::Ui,
    tr: &LocaleManager,
    collapsed: &mut HashSet<&'static str>,
    links: Option<&mut LinksPanel>,
    outline: &[crate::markdown::renderer::Heading],
    frontmatter: &crate::notes::frontmatter::NoteFrontmatter,
    new_tag: &mut String,
    new_alias: &mut String,
    actions: &mut Vec<PanelAction>,
    scroll_to_slug: &mut Option<String>,
) {
    let p = pal();
    let t = |key: &str| tr.t(key, &[]);

    // ── Local graph (links between documents, always on top) ──
    ui.horizontal(|ui| {
        let open = pane_header(ui, collapsed, "graph", &t("editor-local-graph"), None);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if open && widgets::icon_button(ui, ICON_HUB.codepoint, &t("graph-title"), false).clicked() {
                actions.push(PanelAction::OpenFullGraph);
            }
        });
    });
    if !collapsed.contains("graph") {
        match links.as_ref() {
            Some(links) if !links.local_graph.data.nodes.is_empty() => {}
            _ => hint_text(ui, &t("editor-local-graph-empty")),
        }
    }
    let mut graph_open: Option<GraphNode> = None;
    let mut links = links;
    if let Some(links) = links.as_deref_mut() {
        if !collapsed.contains("graph") {
            let size = Vec2::new(ui.available_width(), LOCAL_GRAPH_HEIGHT);
            let outcome = links.local_graph.show(ui, tr, size);
            if let Some(i) = outcome.open {
                graph_open = Some(links.local_graph.data.nodes[i].clone());
            }
        }

        // ── Backlinks: linked mentions + unlinked mentions ──
        if pane_header(ui, collapsed, "backlinks", &t("editor-linked-mentions"), Some(links.backlinks.len())) {
            if links.backlinks.is_empty() {
                hint_text(ui, &t("editor-backlinks-empty"));
            }
            for b in &links.backlinks {
                let resp = widgets::list_row(
                    ui,
                    widgets::RowSpec {
                        icon: ICON_DESCRIPTION.codepoint,
                        icon_color: p.note_icon,
                        label: &b.src_title,
                        trailing: None,
                        selected: false,
                        indent: 0.0,
                        reserve_right: 0.0,
                    },
                );
                if resp.clicked() {
                    actions.push(PanelAction::OpenPath(b.src_path.clone()));
                }
                context_text(ui, &b.context);
            }
        }
        if pane_header(ui, collapsed, "mentions", &t("editor-unlinked-mentions"), Some(links.mentions.len())) {
            if links.mentions.is_empty() {
                hint_text(ui, &t("editor-unlinked-mentions-empty"));
            }
            for m in &links.mentions {
                let resp = widgets::list_row(
                    ui,
                    widgets::RowSpec {
                        icon: ICON_DESCRIPTION.codepoint,
                        icon_color: p.text_faint,
                        label: &m.title,
                        trailing: None,
                        selected: false,
                        indent: 0.0,
                        reserve_right: 28.0,
                    },
                );
                if resp.clicked() {
                    actions.push(PanelAction::OpenPath(m.path.clone()));
                }
                let action_center = egui::pos2(resp.rect.right() - 14.0, resp.rect.center().y);
                if widgets::row_action(
                    ui,
                    Id::new(("mention_link", &m.path, m.line)),
                    action_center,
                    ICON_ADD_LINK.codepoint,
                    &t("editor-link-mention"),
                )
                .clicked()
                {
                    actions.push(PanelAction::LinkMention {
                        path: m.path.clone(),
                        line: m.line,
                    });
                }
                context_text(ui, &m.context);
            }
        }

        // ── Outgoing links ──
        if pane_header(ui, collapsed, "outgoing", &t("editor-outgoing-links"), Some(links.outgoing.len())) {
            if links.outgoing.is_empty() {
                hint_text(ui, &t("editor-outgoing-empty"));
            }
            for o in &links.outgoing {
                let resp = widgets::list_row(
                    ui,
                    widgets::RowSpec {
                        icon: ICON_LINK.codepoint,
                        icon_color: if o.resolved { p.accent } else { p.text_faint },
                        label: &o.reference,
                        trailing: (!o.resolved).then_some("✎"),
                        selected: false,
                        indent: 0.0,
                        reserve_right: 0.0,
                    },
                );
                if resp.clicked() {
                    actions.push(PanelAction::Navigate(o.reference.clone()));
                }
            }
        }
    }

    // ── Outline ──
    if pane_header(ui, collapsed, "outline", &t("editor-outline"), Some(outline.len())) {
        if outline.is_empty() {
            hint_text(ui, &t("editor-outline-empty"));
        }
        for heading in outline {
            let resp = widgets::list_row(
                ui,
                widgets::RowSpec {
                    icon: "",
                    icon_color: p.text_faint,
                    label: &heading.title,
                    trailing: None,
                    selected: false,
                    indent: (heading.level.saturating_sub(1) as f32) * 12.0,
                    reserve_right: 0.0,
                },
            );
            if resp.clicked() {
                *scroll_to_slug = Some(heading.slug.clone());
            }
        }
    }

    // ── Properties (frontmatter) ──
    if pane_header(ui, collapsed, "properties", &t("editor-properties"), None) {
        properties_section(ui, tr, frontmatter, new_tag, new_alias, actions);
    }

    // ── Related (AI) — MNEMONIC's addition to Obsidian's panes ──
    if let Some(links) = links
        && pane_header(ui, collapsed, "related", &t("editor-related"), Some(links.related.len()))
    {
        if links.related.is_empty() {
            hint_text(ui, &t("editor-related-empty"));
        }
        for r in &links.related {
            let percent = format!("{:.0}%", r.similarity * 100.0);
            let resp = widgets::list_row(
                ui,
                widgets::RowSpec {
                    icon: if r.is_pdf {
                        ICON_PICTURE_AS_PDF.codepoint
                    } else {
                        ICON_DESCRIPTION.codepoint
                    },
                    icon_color: if r.is_pdf { p.pdf_icon } else { p.note_icon },
                    label: &r.title,
                    trailing: Some(&percent),
                    selected: false,
                    indent: 0.0,
                    reserve_right: 28.0,
                },
            );
            if resp.clicked() {
                actions.push(PanelAction::OpenPath(r.path.clone()));
            }
            if resp.hovered()
                && widgets::row_action(
                    ui,
                    Id::new(("related_link", &r.path)),
                    egui::pos2(resp.rect.right() - 14.0, resp.rect.center().y),
                    ICON_ADD_LINK.codepoint,
                    &t("editor-insert-link"),
                )
                .clicked()
            {
                actions.push(PanelAction::InsertLink(r.title.clone()));
            }
        }
    }
    if let Some(node) = graph_open {
        actions.push(PanelAction::OpenGraphNode(node));
    }
}

/// Tags, aliases and any other frontmatter properties, editable in place
/// like Obsidian's Properties view.
fn properties_section(
    ui: &mut egui::Ui,
    tr: &LocaleManager,
    fm: &crate::notes::frontmatter::NoteFrontmatter,
    new_tag: &mut String,
    new_alias: &mut String,
    actions: &mut Vec<PanelAction>,
) {
    let p = pal();
    let t = |key: &str| tr.t(key, &[]);
    let label = |ui: &mut egui::Ui, text: &str| {
        ui.label(RichText::new(text).size(theme::TEXT_XS).color(p.text_dim));
    };

    egui::Frame::NONE
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;

            label(ui, &t("editor-tags"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for tag in &fm.tags {
                    theme::tag_chip_frame(theme::tag_color(tag)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            ui.label(RichText::new(format!("#{tag}")).size(theme::TEXT_XS).color(p.text));
                            if ui
                                .add(egui::Label::new(RichText::new(ICON_CLOSE.codepoint).size(11.0).color(p.text_faint)).sense(egui::Sense::click()))
                                .on_hover_text(t("editor-remove"))
                                .clicked()
                            {
                                actions.push(PanelAction::RemoveTag(tag.clone()));
                            }
                        });
                    });
                }
                let resp = egui::TextEdit::singleline(new_tag)
                    .id(Id::new("props_new_tag"))
                    .hint_text(RichText::new(t("editor-add-tag")).color(p.text_faint))
                    .font(FontId::proportional(theme::TEXT_XS))
                    .desired_width(90.0)
                    .show(ui)
                    .response;
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !new_tag.trim().is_empty() {
                    actions.push(PanelAction::AddTag(std::mem::take(new_tag)));
                    resp.request_focus();
                }
            });

            label(ui, &t("editor-aliases"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for alias in &fm.aliases {
                    theme::tag_chip_frame(p.surface).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            ui.label(RichText::new(alias).size(theme::TEXT_XS).color(p.text));
                            if ui
                                .add(egui::Label::new(RichText::new(ICON_CLOSE.codepoint).size(11.0).color(p.text_faint)).sense(egui::Sense::click()))
                                .clicked()
                            {
                                actions.push(PanelAction::RemoveAlias(alias.clone()));
                            }
                        });
                    });
                }
                let resp = egui::TextEdit::singleline(new_alias)
                    .id(Id::new("props_new_alias"))
                    .hint_text(RichText::new(t("editor-add-alias")).color(p.text_faint))
                    .font(FontId::proportional(theme::TEXT_XS))
                    .desired_width(110.0)
                    .show(ui)
                    .response;
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !new_alias.trim().is_empty() {
                    actions.push(PanelAction::AddAlias(std::mem::take(new_alias)));
                    resp.request_focus();
                }
            });

            egui::Grid::new("props_grid").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
                label(ui, &t("editor-created"));
                ui.label(RichText::new(fm.created.format("%Y-%m-%d %H:%M").to_string()).size(theme::TEXT_XS).color(p.text));
                ui.end_row();
                label(ui, &t("editor-modified"));
                ui.label(RichText::new(fm.modified.format("%Y-%m-%d %H:%M").to_string()).size(theme::TEXT_XS).color(p.text));
                ui.end_row();
                for (k, v) in &fm.extra {
                    label(ui, k);
                    let shown = match v {
                        serde_yaml::Value::String(s) => s.clone(),
                        other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
                    };
                    ui.label(RichText::new(shown).size(theme::TEXT_XS).color(p.text));
                    ui.end_row();
                }
            });
        });
}

/// The line a link/mention appears on, under its row.
fn context_text(ui: &mut egui::Ui, text: &str) {
    egui::Frame::NONE
        .inner_margin(Margin {
            left: 30,
            right: 8,
            top: 0,
            bottom: 4,
        })
        .show(ui, |ui| {
            ui.label(
                RichText::new(wikilink::display_text(text))
                    .size(theme::TEXT_XS)
                    .color(pal().text_faint),
            );
        });
}

fn hint_text(ui: &mut egui::Ui, text: &str) {
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(theme::TEXT_XS)
                    .color(pal().text_faint),
            );
        });
}
