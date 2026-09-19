//! Note editor screen: a centered, comfortable writing column showing the
//! note in Live mode (rendered, the clicked line editable in place,
//! §3.2.1) in the colours of the reading theme (§3.2.5), an optional side
//! panel (outline, backlinks with context, unlinked mentions, AI-related
//! notes, local graph), the `/` and `[[` autocomplete popup, and the
//! infinite canvas for Canvas / Split mode.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use egui::{Frame, Id, Margin, RichText, Vec2};
use uuid::Uuid;

mod canvas_edit;
mod canvas_io;
mod canvas_surface;
mod live;
mod panel;
mod source;

use canvas_surface::show_canvas_surface;
use canvas_surface::apply_canvas_outcome;
use panel::right_panel;
use source::source_editor;

use super::graph::GraphView;
use super::{MnemonicApp, RELATED_LIMIT};
use crate::core::Backlink;
use crate::core::embedding::RELATED_DOC_SIMILARITY;
use crate::core::ingestion::pdf_doc_id;
use crate::graph::{GraphNode, GraphOptions, model::note_key};
use crate::markdown::wikilink::title_key;
use crate::markdown::{EditorMode, wikilink};
use crate::notes::{Note, Vault};
use crate::ui::{self, ToastKind, pal, theme};

/// Most unlinked mentions listed in the side panel.
const MENTIONS_LIMIT: usize = 30;
/// Height of the local graph in the side panel.
pub(super) const LOCAL_GRAPH_HEIGHT: f32 = 220.0;

/// Per-open-note UI state that isn't part of the document itself.
#[derive(Default)]
pub(super) struct EditorUi {
    /// Working copy of the title shown in the top bar.
    pub(super) title_buffer: String,
    pub(super) save_failed: bool,
    /// Focus the title with its text selected on the next frame.
    pub(super) select_title: bool,
    /// Whether the autocomplete popup was showing last frame.
    pub(super) popup_visible: bool,
    pub(super) popup_index: usize,
    /// Byte offset of a trigger the user dismissed with Esc, so the popup
    /// stays closed until the cursor moves.
    pub(super) popup_dismissed_at: Option<usize>,
    /// Link-derived panel data, rebuilt when the note or index changes.
    links: Option<LinksPanel>,
    /// Title keys of existing notes/PDFs (for styling unresolved links),
    /// tagged with the index generation it was built for.
    resolvable: Option<(u64, HashSet<String>)>,
    /// Right-panel sections the user folded (Obsidian keeps these per view).
    collapsed: HashSet<&'static str>,
    /// Input buffers of the Properties section.
    new_tag: String,
    new_alias: String,
    /// The lines shown as raw Markdown in Live mode, if any.
    live: Option<live::LiveEdit>,
}

/// Everything the side panel shows that comes from the index.
pub(super) struct LinksPanel {
    key: (Uuid, u64, String),
    backlinks: Vec<Backlink>,
    mentions: Vec<Mention>,
    /// `[[links]]` going out of this note (Obsidian's "Outgoing links").
    outgoing: Vec<Outgoing>,
    related: Vec<Related>,
    local_graph: GraphView,
}

struct Outgoing {
    /// `Title` or `Title#Heading` as written.
    reference: String,
    /// Whether the target exists in the vault.
    resolved: bool,
}

struct Mention {
    title: String,
    path: PathBuf,
    line: usize,
    context: String,
}

struct Related {
    title: String,
    path: PathBuf,
    is_pdf: bool,
    similarity: f32,
}

/// What the side panel asks the app to do.
pub(super) enum PanelAction {
    OpenPath(PathBuf),
    /// Follow a `[[reference]]` (creates the note if missing).
    Navigate(String),
    AddTag(String),
    RemoveTag(String),
    AddAlias(String),
    RemoveAlias(String),
    /// Wrap an unlinked mention on `line` of the note at `path`.
    LinkMention { path: PathBuf, line: usize },
    /// Append `[[target]]` to the open note.
    InsertLink(String),
    OpenGraphNode(GraphNode),
    OpenFullGraph,
}

impl EditorUi {
    pub(super) fn for_title(title: &str) -> Self {
        EditorUi {
            title_buffer: title.to_string(),
            ..Default::default()
        }
    }
}

impl MnemonicApp {
    pub(super) fn show_editor(&mut self, ui: &mut egui::Ui) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let p = pal();
        let ctx = ui.ctx().clone();

        let mut navigate_to: Option<String> = None;
        let mut clicked_tag: Option<String> = None;
        let mut scroll_to_slug: Option<String> = None;
        let mut commit_title = false;
        let mut canvas_toast = None;
        let mut panel_actions: Vec<PanelAction> = Vec::new();

        if !editor.mode.shows_canvas() {
            if self.settings.show_outline {
                self.refresh_links_panel(&editor.note);
            }
            self.refresh_resolvable();
        }
        let colors = *self.reading_theme_for(&editor.note).colors(p.is_dark);
        let tr = &self.locales;
        let t = |key: &str| tr.t(key, &[]);

        // Split: the diagram takes the right half, the Markdown keeps the
        // left (§Fase 3). Edgeless: the diagram takes everything.
        if editor.mode == EditorMode::Split {
            editor.ensure_canvas();
            let half = (ui.available_width() * 0.5).max(320.0);
            let mut outcome = None;
            egui::Panel::right("split_canvas_panel")
                .resizable(true)
                .default_size(half)
                .size_range(280.0..=ui.available_width() - 320.0)
                .frame(egui::Frame::NONE)
                .show_separator_line(true)
                .show(ui, |ui| {
                    let orphans = editor.orphans().clone();
                    if let Some(canvas) = editor.canvas.as_mut() {
                        outcome = Some(show_canvas_surface(
                            canvas,
                            &mut editor.canvas_interaction,
                            &orphans,
                            ui,
                            p.is_dark,
                            tr,
                        ));
                    }
                });
            if let Some(outcome) = outcome {
                canvas_toast = apply_canvas_outcome(&mut editor, outcome, tr);
            }
        }

        if editor.mode == EditorMode::Edgeless {
            editor.ensure_canvas();
            let orphans = editor.orphans().clone();
            if let Some(canvas) = editor.canvas.as_mut() {
                let outcome =
                    show_canvas_surface(canvas, &mut editor.canvas_interaction, &orphans, ui, p.is_dark, tr);
                canvas_toast = apply_canvas_outcome(&mut editor, outcome, tr);
            }
            self.editor_ui.popup_visible = false;
        } else {
            let editor_ui = &mut self.editor_ui;

            // ── Status bar (Obsidian: backlinks · words · characters) ──
            let backlink_count = editor_ui.links.as_ref().map(|l| l.backlinks.len()).unwrap_or(0);
            let status = format!(
                "{} · {} · {}",
                tr.t("editor-status-backlinks", &[("count", &backlink_count.to_string())]),
                tr.t("editor-word-count", &[("count", &editor.word_count().to_string())]),
                tr.t(
                    "editor-char-count",
                    &[("count", &editor.note.body.chars().count().to_string())]
                ),
            );
            egui::Panel::bottom("editor_status_bar")
                .exact_size(22.0)
                .frame(egui::Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(12, 2)))
                .show_separator_line(true)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(&status).size(theme::TEXT_XS).color(p.text_faint));
                    });
                });

            // ── Right sidebar: local graph, backlinks, outgoing links,
            //    outline, properties, related (Obsidian's right panes) ──
            if self.settings.show_outline && editor.mode != EditorMode::Split {
                let outline = editor.outline();
                let frontmatter = editor.note.frontmatter.clone();
                let EditorUi {
                    links,
                    collapsed,
                    new_tag,
                    new_alias,
                    ..
                } = editor_ui;

                egui::Panel::right("editor_outline_panel")
                    .resizable(true)
                    .default_size(290.0)
                    .size_range(220.0..=480.0)
                    .frame(theme::side_panel_frame().fill(p.bg))
                    .show_separator_line(true)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            right_panel(
                                ui,
                                tr,
                                collapsed,
                                links.as_mut(),
                                &outline,
                                &frontmatter,
                                new_tag,
                                new_alias,
                                &mut panel_actions,
                                &mut scroll_to_slug,
                            );
                        });
                    });
            }

            // ── Writing column, on the reading theme's page colour ──
            let editor_ui = &mut self.editor_ui;
            let cache = &mut self.markdown_cache;
            let vault = self.vault.as_ref();
            let pdfs = &self.pdf_documents;
            let untitled = t("editor-untitled");
            ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, colors.background);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_viewport(ui, |ui, viewport| {
                    let content_top = ui.min_rect().top();
                    let avail = ui.available_width();
                    let col = (avail - 2.0 * theme::SPACE_XL).clamp(200.0, theme::EDITOR_MAX_WIDTH);
                    let margin = ((avail - col) / 2.0).max(0.0);
                    ui.add_space(theme::SPACE_XL * 1.5);
                    ui.horizontal_top(|ui| {
                        ui.add_space(margin);
                        ui.vertical(|ui| {
                            ui.set_width(col);

                            // Inline title, as in Obsidian: the note name
                            // sits above the body and is edited in place.
                            let title_resp = egui::TextEdit::singleline(&mut editor_ui.title_buffer)
                                .id(Id::new("note_inline_title"))
                                .frame(Frame::NONE)
                                .font(theme::semibold(theme::TEXT_DISPLAY))
                                .text_color(colors.h1.into())
                                .hint_text(RichText::new(&untitled).color(p.text_faint))
                                .desired_width(col)
                                .margin(Margin::ZERO)
                                .show(ui)
                                .response;
                            if title_resp.lost_focus() {
                                commit_title = true;
                            }
                            ui.add_space(theme::SPACE_M);

                            if editor.mode == EditorMode::Source {
                                let rows = ((viewport.height() - 160.0) / 24.0).max(12.0) as usize;
                                source_editor(ui, &ctx, tr, &mut editor, editor_ui, vault, pdfs, &colors, col, rows);
                                ui.add_space(theme::SPACE_XL * 4.0);
                            } else {
                                // The renderer virtualizes against heights
                                // measured from its own first line, so shift
                                // the viewport past the padding above it.
                                let offset = ui.cursor().top() - content_top;
                                let resolvable = editor_ui.resolvable.as_ref().map(|(_, s)| s.clone());
                                let is_resolved = |target: &str| {
                                    resolvable.as_ref().is_none_or(|s| s.contains(&title_key(target)))
                                };
                                let resolve_embed = |target: &str| resolve_embed_target(vault, target);
                                let inputs = live::LiveInputs {
                                    tr,
                                    vault,
                                    pdfs,
                                    viewport: viewport.translate(Vec2::new(0.0, -offset)),
                                    colors: &colors,
                                    is_resolved: &is_resolved,
                                    resolve_embed: &resolve_embed,
                                };
                                let outcome = live::live_view(ui, cache, &mut editor, editor_ui, &inputs);
                                navigate_to = outcome.navigate;
                                clicked_tag = outcome.tag;
                            }
                        });
                    });
                });
        }

        if let Some(slug) = scroll_to_slug {
            // The outline scrolls the rendered view.
            if editor.mode == EditorMode::Source {
                editor.mode = EditorMode::Live;
            }
            editor.scroll_to_heading(&slug);
        }
        self.editor = Some(editor);
        if commit_title {
            self.commit_title_buffer();
        }
        if let Some((kind, msg)) = canvas_toast {
            self.toasts.push(kind, msg);
        }
        if let Some(title) = navigate_to {
            self.navigate_wikilink(&title);
        }
        if let Some(tag) = clicked_tag {
            // Obsidian: clicking a tag searches for it — here, filter the
            // library by that tag.
            self.close_document();
            self.close_graph_view();
            self.doc_filter = ui::SidebarDocFilter::Tag(tag);
        }
        for action in panel_actions {
            self.apply_panel_action(action);
        }
    }

    /// Rebuilds the side panel's index-derived data when the open note,
    /// its title, or the index changed since it was last built.
    fn refresh_links_panel(&mut self, note: &Note) {
        let key = (
            note.frontmatter.id,
            self.index_generation,
            note.frontmatter.title.clone(),
        );
        if self.editor_ui.links.as_ref().is_some_and(|l| l.key == key) {
            return;
        }
        let (Some(vault), Some(index)) = (self.vault.as_ref(), self.index.as_ref()) else {
            return;
        };
        let id = note.frontmatter.id;
        let title = note.frontmatter.title.clone();

        let keys = wikilink::link_keys_for(note);
        let backlinks = index.backlinks_for_keys(&keys, id).unwrap_or_else(|e| {
            log::warn!("app: loading backlinks failed: {e:#}");
            Vec::new()
        });

        let mentions: Vec<Mention> = vault
            .notes
            .iter()
            .filter(|n| n.frontmatter.id != id && !n.frontmatter.trashed && !n.is_canvas())
            .flat_map(|n| {
                wikilink::unlinked_mentions(&n.body, &title)
                    .into_iter()
                    .map(|m| Mention {
                        title: n.frontmatter.title.clone(),
                        path: n.path.clone(),
                        line: m.line,
                        context: m.context,
                    })
            })
            .take(MENTIONS_LIMIT)
            .collect();

        // Related by meaning, minus what is already connected by links.
        let mut connected: HashSet<String> = wikilink::extract_wikilinks(&note.body)
            .iter()
            .map(|t| title_key(t))
            .collect();
        connected.extend(backlinks.iter().map(|b| title_key(&b.src_title)));
        let notes_by_id: HashMap<Uuid, &Note> =
            vault.notes.iter().map(|n| (n.frontmatter.id, n)).collect();
        let pdfs_by_id: HashMap<Uuid, &PathBuf> =
            self.pdf_documents.iter().map(|p| (pdf_doc_id(p), p)).collect();
        let related = index
            .similar_documents(id, RELATED_LIMIT * 3)
            .unwrap_or_else(|e| {
                log::warn!("app: loading related documents failed: {e:#}");
                Vec::new()
            })
            .into_iter()
            .filter(|(_, sim)| *sim >= RELATED_DOC_SIMILARITY)
            .filter_map(|(doc, similarity)| {
                if let Some(n) = notes_by_id.get(&doc).filter(|n| !n.frontmatter.trashed) {
                    return Some(Related {
                        title: n.frontmatter.title.clone(),
                        path: n.path.clone(),
                        is_pdf: false,
                        similarity,
                    });
                }
                pdfs_by_id.get(&doc).map(|p| Related {
                    title: p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    path: (*p).clone(),
                    is_pdf: true,
                    similarity,
                })
            })
            .filter(|r| !connected.contains(&title_key(&r.title)))
            .take(RELATED_LIMIT)
            .collect();

        let opts = GraphOptions {
            show_orphans: true,
            show_ghosts: true,
            show_pdfs: true,
            show_semantic: false,
        };
        let full = self.build_graph_data(opts);
        let focus_key = note_key(id);
        let local = full
            .index_of(&focus_key)
            .map(|center| full.neighborhood(center, 1))
            .unwrap_or_default();
        // Keep node positions stable across refreshes of the same note.
        let mut local_graph = match self.editor_ui.links.take() {
            Some(mut old) if old.key.0 == id => {
                old.local_graph.replace_data(local, opts);
                old.local_graph
            }
            _ => GraphView::new(local, opts, &HashMap::new(), true),
        };
        local_graph.set_focus(&focus_key);

        let resolvable: HashSet<String> = vault
            .notes
            .iter()
            .filter(|n| !n.frontmatter.trashed)
            .flat_map(wikilink::link_keys_for)
            .chain(
                self.pdf_documents
                    .iter()
                    .chain(&self.derived.sheets)
                    .filter_map(|p| p.file_name().map(|n| title_key(&n.to_string_lossy()))),
            )
            .collect();
        let mut outgoing: Vec<Outgoing> = Vec::new();
        for occ in wikilink::parse_wikilinks(&note.body) {
            let reference = occ.link.reference();
            if outgoing.iter().any(|o| o.reference == reference) {
                continue;
            }
            outgoing.push(Outgoing {
                resolved: resolvable.contains(&title_key(&occ.link.target)),
                reference,
            });
        }

        self.editor_ui.links = Some(LinksPanel {
            key,
            backlinks,
            mentions,
            outgoing,
            related,
            local_graph,
        });
    }

    fn refresh_resolvable(&mut self) {
        if self
            .editor_ui
            .resolvable
            .as_ref()
            .is_some_and(|(generation, _)| *generation == self.index_generation)
        {
            return;
        }
        let Some(vault) = self.vault.as_ref() else {
            return;
        };
        let set = vault
            .notes
            .iter()
            .filter(|n| !n.frontmatter.trashed)
            .map(|n| title_key(&n.frontmatter.title))
            .chain(
                self.pdf_documents
                    .iter()
                    .chain(&self.derived.sheets)
                    .filter_map(|p| p.file_name().map(|n| title_key(&n.to_string_lossy()))),
            )
            .collect();
        self.editor_ui.resolvable = Some((self.index_generation, set));
    }

    fn apply_panel_action(&mut self, action: PanelAction) {
        match action {
            PanelAction::OpenPath(path) => self.open_file_by_path(path),
            PanelAction::Navigate(reference) => self.navigate_wikilink(&reference),
            PanelAction::AddTag(tag) => {
                let tag = tag.trim().trim_start_matches('#').trim_matches('/').to_string();
                if let Some(editor) = self.editor.as_mut()
                    && !tag.is_empty()
                    && !editor.note.frontmatter.tags.iter().any(|t| t.eq_ignore_ascii_case(&tag))
                {
                    editor.note.frontmatter.tags.push(tag);
                    editor.mark_metadata_dirty();
                }
            }
            PanelAction::RemoveTag(tag) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.note.frontmatter.tags.retain(|t| !t.eq_ignore_ascii_case(&tag));
                    editor.mark_metadata_dirty();
                }
            }
            PanelAction::AddAlias(alias) => {
                let alias = alias.trim().to_string();
                if let Some(editor) = self.editor.as_mut()
                    && !alias.is_empty()
                    && !editor.note.frontmatter.aliases.iter().any(|a| a.eq_ignore_ascii_case(&alias))
                {
                    editor.note.frontmatter.aliases.push(alias);
                    editor.mark_metadata_dirty();
                }
            }
            PanelAction::RemoveAlias(alias) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.note.frontmatter.aliases.retain(|a| !a.eq_ignore_ascii_case(&alias));
                    editor.mark_metadata_dirty();
                }
            }
            PanelAction::OpenGraphNode(node) => self.open_graph_node(&node),
            PanelAction::OpenFullGraph => self.open_graph_view(),
            PanelAction::InsertLink(target) => {
                if let Some(editor) = self.editor.as_mut() {
                    let mut body = editor.note.body.clone();
                    if !body.is_empty() && !body.ends_with('\n') {
                        body.push('\n');
                    }
                    body.push_str(&format!("\n[[{target}]]\n"));
                    editor.set_body(body);
                }
                self.save_editor_now();
            }
            PanelAction::LinkMention { path, line } => {
                let Some(title) = self.editor.as_ref().map(|e| e.note.frontmatter.title.clone())
                else {
                    return;
                };
                let Some(mut note) = self.note_by_path(&path) else {
                    return;
                };
                let Some(body) = wikilink::link_mention_on_line(&note.body, line, &title) else {
                    return;
                };
                note.body = body;
                if let Err(e) = note.save() {
                    self.report_error("error-context-save-note", e);
                    return;
                }
                self.ignore_watcher_until =
                    Some(std::time::Instant::now() + super::SELF_WRITE_GRACE);
                if let Some(vault) = self.vault.as_mut()
                    && let Some(existing) = vault.notes.iter_mut().find(|n| n.path == path)
                {
                    *existing = note.clone();
                }
                if let Some(index) = self.index.as_mut()
                    && let Err(e) = index.upsert_note(&note)
                {
                    log::warn!("app: updating links in index failed: {e:#}");
                }
                self.reindex_note(&note);
                self.index_changed();
                self.toast(ToastKind::Success, "toast-mention-linked", &[("title", &note.frontmatter.title)]);
            }
        }
    }
}

/// Resolves `![[target]]` against the open vault (see
/// `export::resolve_embed`).
pub(super) fn resolve_embed_target(
    vault: Option<&Vault>,
    target: &str,
) -> Option<crate::markdown::renderer::EmbedContent> {
    crate::export::resolve_embed(vault?, target)
}

