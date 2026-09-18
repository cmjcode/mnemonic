//! Note editor screen: a centered, comfortable writing column (Write /
//! Read modes), an optional side panel (outline, backlinks with context,
//! unlinked mentions, AI-related notes, local graph), an inline
//! autocomplete popup for `/` commands and `[[wikilinks]]`, and the
//! infinite canvas for Canvas mode.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use egui::{Align2, FontId, Frame, Id, Margin, Modifiers, RichText, Vec2};
use egui_icons::icons::{
    ICON_ADD_LINK, ICON_CHEVRON_RIGHT, ICON_CLOSE, ICON_DESCRIPTION, ICON_EXPAND_MORE, ICON_HUB,
    ICON_LINK, ICON_PICTURE_AS_PDF,
};
use uuid::Uuid;

use super::graph::GraphView;
use super::{MnemonicApp, RELATED_LIMIT};
use crate::canvas::{
    self, BlockBinding, CanvasDocument, CanvasElement, CanvasElementId, CanvasTool, InteractionState,
};
use crate::core::Backlink;
use crate::core::embedding::RELATED_DOC_SIMILARITY;
use crate::core::ingestion::pdf_doc_id;
use crate::graph::{GraphNode, GraphOptions, model::note_key};
use crate::i18n::LocaleManager;
use crate::markdown::editor::{
    char_index_to_byte_offset, slash_menu_triggered, slash_templates, wikilink_autocomplete_query,
};
use crate::markdown::wikilink::title_key;
use crate::markdown::{EditorMode, MarkdownEditor, WikilinkIndex, wikilink};
use crate::notes::{Note, Vault};
use crate::ui::{self, ToastKind, pal, theme, widgets};

/// Most unlinked mentions listed in the side panel.
const MENTIONS_LIMIT: usize = 30;
/// Height of the local graph in the side panel.
const LOCAL_GRAPH_HEIGHT: f32 = 220.0;

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
    popup_index: usize,
    /// Byte offset of a trigger the user dismissed with Esc, so the popup
    /// stays closed until the cursor moves.
    popup_dismissed_at: Option<usize>,
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
}

/// Everything the side panel shows that comes from the index.
struct LinksPanel {
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
enum PanelAction {
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

enum Completion {
    Slash(&'static str),
    Wikilink { query: String, title: String },
}

/// What the canvas surface asks the app to do after rendering.
pub(super) struct CanvasOutcome {
    pub(super) modified: bool,
    /// A Draw.io file replaced the canvas contents: `imported_blocks` are
    /// the text vertices that became bound nodes and need their paragraphs
    /// appended to the Markdown (`MarkdownEditor::import_bound_canvas`).
    pub(super) imported: Option<(CanvasDocument, Vec<(BlockBinding, String)>)>,
    /// The user asked to bind / unbind the element being text-edited.
    pub(super) bind: Option<(CanvasElementId, bool)>,
    pub(super) toast: Option<(ToastKind, String)>,
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
                    if let Some(canvas) = editor.canvas.as_mut() {
                        outcome = Some(show_canvas_surface(
                            canvas,
                            &mut editor.canvas_interaction,
                            ui,
                            p.is_dark,
                            tr,
                        ));
                    }
                });
            if let Some(outcome) = outcome {
                canvas_toast = apply_canvas_outcome(&mut editor, outcome);
            }
        }

        if editor.mode == EditorMode::Edgeless {
            editor.ensure_canvas();
            if let Some(canvas) = editor.canvas.as_mut() {
                let outcome =
                    show_canvas_surface(canvas, &mut editor.canvas_interaction, ui, p.is_dark, tr);
                canvas_toast = apply_canvas_outcome(&mut editor, outcome);
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

            // ── Writing column ──
            let editor_ui = &mut self.editor_ui;
            let cache = &mut self.markdown_cache;
            let vault = self.vault.as_ref();
            let pdfs = &self.pdf_documents;
            let untitled = t("editor-untitled");

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
                                .text_color(p.text)
                                .hint_text(RichText::new(&untitled).color(p.text_faint))
                                .desired_width(col)
                                .margin(Margin::ZERO)
                                .show(ui)
                                .response;
                            if title_resp.lost_focus() {
                                commit_title = true;
                            }
                            ui.add_space(theme::SPACE_M);

                            match editor.mode {
                                EditorMode::Source => {
                                    let rows =
                                        ((viewport.height() - 160.0) / 24.0).max(12.0) as usize;
                                    source_editor(
                                        ui,
                                        &ctx,
                                        tr,
                                        &mut editor,
                                        editor_ui,
                                        vault,
                                        pdfs,
                                        col,
                                        rows,
                                    );
                                }
                                EditorMode::Reading => {
                                    editor_ui.popup_visible = false;
                                    // The renderer virtualizes against heights
                                    // measured from its own first line, so shift
                                    // the viewport past the padding above it.
                                    let offset = ui.cursor().top() - content_top;
                                    let local = viewport.translate(Vec2::new(0.0, -offset));
                                    let resolvable = editor_ui.resolvable.as_ref().map(|(_, s)| s);
                                    let is_resolved = |target: &str| {
                                        resolvable.is_none_or(|s| s.contains(&title_key(target)))
                                    };
                                    let resolve_embed = |target: &str| resolve_embed_target(vault, target);
                                    let outcome = editor.render(ui, cache, local, &is_resolved, &resolve_embed);
                                    if let Some(new_body) = outcome.updated_body {
                                        editor.set_body(new_body);
                                    }
                                    if let Some(title) = outcome.clicked_wikilink {
                                        navigate_to = Some(title);
                                    }
                                    if let Some(tag) = outcome.clicked_tag {
                                        clicked_tag = Some(tag);
                                    }
                                }
                                EditorMode::Edgeless => {}
                                EditorMode::Split => {
                                    let rows =
                                        ((viewport.height() - 160.0) / 24.0).max(12.0) as usize;
                                    source_editor(
                                        ui,
                                        &ctx,
                                        tr,
                                        &mut editor,
                                        editor_ui,
                                        vault,
                                        pdfs,
                                        col,
                                        rows,
                                    );
                                }
                            }
                            ui.add_space(theme::SPACE_XL * 4.0);
                        });
                    });
                });
        }

        if let Some(slug) = scroll_to_slug {
            // The outline scrolls the rendered view, so jump to Read mode.
            editor.mode = EditorMode::Reading;
            self.markdown_cache.scroll_to_id_target_mut().replace(slug);
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

/// Resolves `![[target]]`: a note by title/stem/alias (its body, for
/// transclusion) or an attachment file anywhere in the vault by name.
fn resolve_embed_target(
    vault: Option<&Vault>,
    target: &str,
) -> Option<crate::markdown::renderer::EmbedContent> {
    use crate::markdown::renderer::EmbedContent;
    let vault = vault?;
    let key = title_key(target);
    if let Some(note) = vault
        .notes
        .iter()
        .filter(|n| !n.frontmatter.trashed)
        .find(|n| wikilink::link_keys_for(n).contains(&key))
    {
        return Some(EmbedContent::Note {
            title: note.frontmatter.title.clone(),
            body: note.body.clone(),
        });
    }
    // Attachment: exact file name, first match under the vault root
    // (hidden folders skipped, like the note scan).
    let wanted = target.trim();
    let found = walkdir::WalkDir::new(&vault.root)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !(e.file_type().is_dir()
                    && e.file_name().to_str().is_some_and(crate::notes::vault::is_skipped_dir_name))
        })
        .flatten()
        .find(|e| e.file_type().is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(wanted))
        .map(|e| e.path().to_path_buf())?;
    Some(EmbedContent::Image(found))
}

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
fn right_panel(
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

/// The Markdown source editor plus its `/` and `[[` autocomplete popup.
#[allow(clippy::too_many_arguments)]
fn source_editor(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    tr: &LocaleManager,
    editor: &mut MarkdownEditor,
    state: &mut EditorUi,
    vault: Option<&Vault>,
    pdfs: &[PathBuf],
    width: f32,
    rows: usize,
) {
    let p = pal();
    let edit_id = Id::new("note_body_editor");

    // Popup navigation keys must be consumed before the TextEdit sees them.
    let (mut down, mut up, mut accept, mut dismiss) = (false, false, false, false);
    if state.popup_visible {
        ctx.input_mut(|i| {
            down = i.consume_key(Modifiers::NONE, egui::Key::ArrowDown);
            up = i.consume_key(Modifiers::NONE, egui::Key::ArrowUp);
            accept = i.consume_key(Modifiers::NONE, egui::Key::Enter)
                || i.consume_key(Modifiers::NONE, egui::Key::Tab);
            dismiss = i.consume_key(Modifiers::NONE, egui::Key::Escape);
        });
    }

    let mut body = editor.note.body.clone();
    // Obsidian-style styled source: headings large, links/tags accented,
    // code monospace — the raw Markdown stays fully editable.
    let style = crate::markdown::highlight::HighlightStyle {
        base_size: 15.5,
        text: p.text,
        dim: p.text_dim,
        faint: p.text_faint,
        accent: p.accent,
        code_bg: p.surface,
        highlight_bg: p.accent_soft,
        semibold: egui::FontFamily::Name(theme::SEMIBOLD_FAMILY.into()),
        line_height: 24.0,
    };
    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
        let job = crate::markdown::highlight::layout_job(text.as_str(), wrap_width, &style);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let output = egui::TextEdit::multiline(&mut body)
        .id(edit_id)
        .frame(Frame::NONE)
        .font(FontId::proportional(15.5))
        .text_color(p.text)
        .hint_text(RichText::new(tr.t("editor-placeholder", &[])).color(p.text_faint))
        .desired_width(width)
        .desired_rows(rows)
        .lock_focus(true)
        .margin(Margin::ZERO)
        .layouter(&mut layouter)
        .show(ui);
    if body != editor.note.body {
        editor.set_body(body.clone());
    }

    let mut visible = false;
    if let Some(range) = output.cursor_range
        && output.response.has_focus()
    {
        let cursor = range.primary;
        let char_idx = cursor.index.0;
        let byte = char_index_to_byte_offset(&body, char_idx);
        let before = &body[..byte];

        let mut items: Vec<(String, Completion)> = if slash_menu_triggered(before) {
            slash_templates()
                .iter()
                .map(|tpl| (tr.t(tpl.key, &[]), Completion::Slash(tpl.insert)))
                .collect()
        } else if let Some(query) = wikilink_autocomplete_query(before) {
            vault
                .map(|v| {
                    WikilinkIndex::build(&v.notes)
                        .with_files(pdfs)
                        .suggestions(&query, 8)
                })
                .unwrap_or_default()
                .into_iter()
                .map(|title| {
                    (
                        title.clone(),
                        Completion::Wikilink {
                            query: query.clone(),
                            title,
                        },
                    )
                })
                .collect()
        } else {
            Vec::new()
        };

        match state.popup_dismissed_at {
            Some(at) if at == byte => items.clear(),
            Some(_) => state.popup_dismissed_at = None,
            None => {}
        }
        if dismiss && !items.is_empty() {
            state.popup_dismissed_at = Some(byte);
            items.clear();
        }

        if !items.is_empty() {
            visible = true;
            if !state.popup_visible {
                state.popup_index = 0;
            }
            state.popup_index = state.popup_index.min(items.len() - 1);
            if down {
                state.popup_index = (state.popup_index + 1) % items.len();
            }
            if up {
                state.popup_index = (state.popup_index + items.len() - 1) % items.len();
            }

            let cursor_rect = output
                .galley
                .pos_from_cursor(cursor)
                .translate(output.galley_pos.to_vec2());
            let mut chosen = accept.then_some(state.popup_index);
            let selected_index = state.popup_index;
            egui::Area::new(Id::new("editor_autocomplete_popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(cursor_rect.left_bottom() + Vec2::new(-6.0, 6.0))
                .show(ctx, |ui| {
                    theme::popover_frame()
                        .inner_margin(Margin::same(6))
                        .show(ui, |ui| {
                            ui.set_min_width(240.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let header = match items[0].1 {
                                Completion::Slash(_) => tr.t("editor-slash-header", &[]),
                                Completion::Wikilink { .. } => tr.t("editor-link-header", &[]),
                            };
                            ui.label(
                                RichText::new(header)
                                    .size(theme::TEXT_XS)
                                    .color(p.text_faint),
                            );
                            for (i, (label, completion)) in items.iter().enumerate() {
                                let icon = match completion {
                                    Completion::Slash(_) => "/",
                                    Completion::Wikilink { .. } => ICON_LINK.codepoint,
                                };
                                let resp = widgets::list_row(
                                    ui,
                                    widgets::RowSpec {
                                        icon,
                                        icon_color: p.text_faint,
                                        label,
                                        trailing: None,
                                        selected: i == selected_index,
                                        indent: 0.0,
                                        reserve_right: 0.0,
                                    },
                                );
                                if resp.clicked() {
                                    chosen = Some(i);
                                }
                            }
                            ui.label(
                                RichText::new(tr.t("editor-popup-hint", &[]))
                                    .size(theme::TEXT_XS)
                                    .color(p.text_faint),
                            );
                        });
                });

            if let Some(i) = chosen {
                let (new_body, new_cursor) = match &items[i].1 {
                    Completion::Slash(insert) => {
                        let mut s = body.clone();
                        s.replace_range(byte - 1..byte, insert);
                        (s, char_idx - 1 + insert.chars().count())
                    }
                    Completion::Wikilink { query, title } => {
                        let mut s = body.clone();
                        let replacement = format!("{title}]]");
                        s.replace_range(byte - query.len()..byte, &replacement);
                        (
                            s,
                            char_idx - query.chars().count() + replacement.chars().count(),
                        )
                    }
                };
                editor.set_body(new_body);
                if let Some(mut edit_state) = egui::TextEdit::load_state(ctx, edit_id) {
                    edit_state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(
                            egui::text::CCursor::new(new_cursor),
                        )));
                    edit_state.store(ctx, edit_id);
                }
                ctx.memory_mut(|m| m.request_focus(edit_id));
                visible = false;
            }
        }
    }
    state.popup_visible = visible;
}

/// Renders the interactive infinite canvas (Canvas mode). Tool docks and
/// HUDs are positioned inside the canvas rect so they never overlap the
/// sidebar or other panels.
pub(super) fn show_canvas_surface(
    canvas: &mut CanvasDocument,
    interaction: &mut InteractionState,
    ui: &mut egui::Ui,
    is_dark: bool,
    tr: &LocaleManager,
) -> CanvasOutcome {
    let (response, painter) = ui.allocate_painter(
        ui.available_size_before_wrap(),
        egui::Sense::click_and_drag(),
    );
    let screen_rect = response.rect;
    let origin = screen_rect.min;
    let mut modified = false;
    let mut imported_doc: Option<(CanvasDocument, Vec<(BlockBinding, String)>)> = None;
    let mut bind: Option<(CanvasElementId, bool)> = None;
    let mut toast = None;
    let ctx = ui.ctx().clone();

    // 0. Deferred fit-to-content (set when a Draw.io note is opened: its
    // coordinates can sit anywhere, and the viewport isn't persisted).
    if std::mem::take(&mut interaction.pending_fit) {
        let bounds = canvas
            .elements
            .iter()
            .map(|e| e.bounding_rect())
            .fold(egui::Rect::NOTHING, |acc, r| acc.union(r));
        if bounds.is_positive() {
            canvas.viewport.fit_rect(bounds, screen_rect.size());
        }
    }

    // Single-letter tool shortcuts (V, H, S, R, ...) while nothing is focused.
    if !ctx.egui_wants_keyboard_input()
        && interaction.editing_text_elem.is_none()
        && let Some(tool) = ui::left_toolbar::tool_shortcut_pressed(&ctx)
    {
        interaction.active_tool = tool;
    }

    // 1. Zoom & pan — only while the pointer is over the canvas. The viewport
    // isn't part of the saved body, so these don't set `modified`: doing so
    // re-serialized the whole diagram every frame of a zoom or pan.
    if response.contains_pointer() {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        let zoom_delta = ui.input(|i| i.zoom_delta());
        let ctrl_pressed = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let hover_pos = response.hover_pos();

        if (zoom_delta - 1.0).abs() > 1e-4 {
            if let Some(pos) = hover_pos {
                canvas.viewport.zoom_at(zoom_delta, pos, origin);
            }
        } else if ctrl_pressed && scroll_delta.y != 0.0 {
            let factor = if scroll_delta.y > 0.0 { 1.1 } else { 0.9 };
            if let Some(pos) = hover_pos {
                canvas.viewport.zoom_at(factor, pos, origin);
            }
        } else if scroll_delta != Vec2::ZERO {
            canvas
                .viewport
                .add_pan_vec(scroll_delta / canvas.viewport.zoom);
        }
    }

    // 2. Dot grid.
    canvas.viewport.draw_grid(&painter, screen_rect, is_dark);

    // 3. Tool drags.
    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let world_pos = canvas.viewport.screen_to_world(pos, origin);
            interaction.is_dragging = true;
            interaction.drag_start_world = Some([world_pos.x, world_pos.y]);
            interaction.drag_current_world = Some([world_pos.x, world_pos.y]);
            if interaction.active_tool == CanvasTool::Pen {
                interaction.current_freehand_points = vec![[world_pos.x, world_pos.y]];
            }
            interaction.dragged_elem = if interaction.active_tool == CanvasTool::Select {
                canvas.element_at(world_pos).map(|e| e.id())
            } else {
                None
            };
        }
    } else if response.dragged() {
        let drag_delta = response.drag_delta();
        if let Some(pos) = response.interact_pointer_pos() {
            let world_pos = canvas.viewport.screen_to_world(pos, origin);
            interaction.drag_current_world = Some([world_pos.x, world_pos.y]);
            match interaction.active_tool {
                CanvasTool::Pan => {
                    canvas
                        .viewport
                        .add_pan_vec(drag_delta / canvas.viewport.zoom);
                }
                CanvasTool::Pen => {
                    interaction
                        .current_freehand_points
                        .push([world_pos.x, world_pos.y]);
                }
                CanvasTool::Select => {
                    // Moving an element only syncs the body once, on drag
                    // stop — not on every frame of the drag.
                    let zoom = canvas.viewport.zoom;
                    match interaction.dragged_elem {
                        Some(elem_id) => {
                            if let Some(target) = canvas.get_element_mut(elem_id) {
                                target.translate(drag_delta / zoom);
                            }
                        }
                        None => canvas.viewport.add_pan_vec(drag_delta / zoom),
                    }
                }
                _ => {}
            }
        }
    } else if response.drag_stopped() {
        if let (Some(start), Some(curr)) =
            (interaction.drag_start_world, interaction.drag_current_world)
        {
            let min_x = start[0].min(curr[0]);
            let min_y = start[1].min(curr[1]);
            let w = (start[0].max(curr[0]) - min_x).max(80.0);
            let h = (start[1].max(curr[1]) - min_y).max(50.0);

            match interaction.active_tool {
                CanvasTool::StickyNote => {
                    canvas.add_element(CanvasElement::StickyNote {
                        id: CanvasElementId::new(),
                        pos: [min_x, min_y],
                        size: [w.max(180.0), h.max(120.0)],
                        text: tr.t("canvas-new-sticky", &[]),
                        color: interaction.primary_color,
                        binding: None,
                    });
                    interaction.active_tool = CanvasTool::Select;
                    modified = true;
                }
                CanvasTool::Shape(kind) => {
                    canvas.add_element(CanvasElement::Shape {
                        id: CanvasElementId::new(),
                        kind,
                        rect: [min_x, min_y, min_x + w.max(140.0), min_y + h.max(80.0)],
                        stroke_color: interaction.primary_color,
                        stroke_width: interaction.stroke_width,
                        fill_color: None,
                        text: String::new(),
                        text_color: None,
                        binding: None,
                    });
                    interaction.active_tool = CanvasTool::Select;
                    modified = true;
                }
                CanvasTool::Connector => {
                    canvas.add_element(CanvasElement::Connector {
                        id: CanvasElementId::new(),
                        from_elem: None,
                        to_elem: None,
                        from_pos: start,
                        to_pos: curr,
                        routing: canvas::ConnectorRouting::Straight,
                        stroke_color: interaction.primary_color,
                        stroke_width: interaction.stroke_width,
                        label: String::new(),
                        arrow_end: true,
                        waypoints: Vec::new(),
                    });
                    interaction.active_tool = CanvasTool::Select;
                    modified = true;
                }
                CanvasTool::Pen => {
                    if interaction.current_freehand_points.len() >= 2 {
                        canvas.add_element(CanvasElement::FreehandStroke {
                            id: CanvasElementId::new(),
                            points: std::mem::take(&mut interaction.current_freehand_points),
                            color: interaction.primary_color,
                            width: interaction.stroke_width,
                        });
                        modified = true;
                    }
                }
                CanvasTool::Select if interaction.dragged_elem.is_some() => {
                    modified = true;
                }
                CanvasTool::Eraser => {
                    if let Some(id) = canvas
                        .element_at(egui::Pos2::new(start[0], start[1]))
                        .map(|e| e.id())
                    {
                        canvas.remove_element(id);
                        modified = true;
                    }
                }
                _ => {}
            }
        }
        interaction.is_dragging = false;
        interaction.dragged_elem = None;
        interaction.drag_start_world = None;
        interaction.drag_current_world = None;
    }

    // The eraser also works with a simple click.
    if response.clicked()
        && interaction.active_tool == CanvasTool::Eraser
        && let Some(pos) = response.interact_pointer_pos()
    {
        let world = canvas.viewport.screen_to_world(pos, origin);
        if let Some(id) = canvas.element_at(world).map(|e| e.id()) {
            canvas.remove_element(id);
            modified = true;
        }
    }

    // 4. Elements.
    let hovered_id = response.hover_pos().and_then(|pos| {
        let world = canvas.viewport.screen_to_world(pos, origin);
        canvas.element_at(world).map(|e| e.id())
    });
    // Skip elements entirely off-screen; the margin keeps connector labels and
    // selection handles that poke past an element's bounds from popping.
    let visible_world = canvas
        .viewport
        .screen_rect_to_world(screen_rect.expand(64.0), origin);
    for elem in canvas
        .elements
        .iter()
        .filter(|e| visible_world.intersects(e.bounding_rect()))
    {
        canvas::draw_element(
            &painter,
            &canvas.viewport,
            origin,
            elem,
            hovered_id == Some(elem.id()),
            is_dark,
        );
    }

    if interaction.active_tool == CanvasTool::Pen && interaction.current_freehand_points.len() >= 2
    {
        let c = interaction.primary_color;
        let stroke_c = egui::Color32::from_rgb(
            (c[0] * 255.0) as u8,
            (c[1] * 255.0) as u8,
            (c[2] * 255.0) as u8,
        );
        let points: Vec<egui::Pos2> = interaction
            .current_freehand_points
            .iter()
            .map(|pt| {
                canvas
                    .viewport
                    .world_to_screen(egui::Pos2::new(pt[0], pt[1]), origin)
            })
            .collect();
        for w in points.windows(2) {
            painter.line_segment(
                [w[0], w[1]],
                (interaction.stroke_width * canvas.viewport.zoom, stroke_c),
            );
        }
    }

    if response.hovered() {
        ctx.set_cursor_icon(match interaction.active_tool {
            CanvasTool::Pan => egui::CursorIcon::Grab,
            CanvasTool::Select if hovered_id.is_some() => egui::CursorIcon::Move,
            CanvasTool::Select => egui::CursorIcon::Default,
            _ => egui::CursorIcon::Crosshair,
        });
    }

    if canvas.elements.is_empty() {
        painter.text(
            screen_rect.center(),
            Align2::CENTER_CENTER,
            tr.t("canvas-empty-hint", &[]),
            FontId::proportional(theme::TEXT_BODY),
            pal().text_faint,
        );
    }

    // 5. Double-click to edit text.
    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let world = canvas.viewport.screen_to_world(pos, origin);
        if let Some(elem) = canvas.element_at(world) {
            interaction.editing_text_elem = Some(elem.id());
        }
    }

    if let Some(editing_id) = interaction.editing_text_elem {
        let info = canvas.get_element(editing_id).map(|e| {
            let text = match e {
                CanvasElement::StickyNote { text, .. } | CanvasElement::Shape { text, .. } => {
                    text.clone()
                }
                CanvasElement::Connector { label, .. } => label.clone(),
                _ => String::new(),
            };
            let bindable = matches!(e, CanvasElement::StickyNote { .. } | CanvasElement::Shape { .. });
            (e.bounding_rect(), text, bindable, e.is_bound())
        });
        match info {
            Some((bounds, mut text_buf, bindable, is_bound)) => {
                let s_rect = canvas.viewport.world_rect_to_screen(bounds, origin);
                let mut close_edit = ctx.input(|i| i.key_pressed(egui::Key::Escape));
                let mut changed = false;
                egui::Area::new(Id::new("canvas_inline_text_edit_area"))
                    .fixed_pos(s_rect.min)
                    .order(egui::Order::Foreground)
                    .show(&ctx, |ui| {
                        theme::popover_frame()
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_max_width(s_rect.width().max(220.0));
                                let resp = ui.add(
                                    egui::TextEdit::multiline(&mut text_buf)
                                        .id(Id::new("canvas_inline_text_edit"))
                                        .desired_width(s_rect.width().max(200.0))
                                        .desired_rows(3),
                                );
                                if !resp.has_focus() && !resp.lost_focus() {
                                    resp.request_focus();
                                }
                                changed = resp.changed();
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(if is_bound {
                                            tr.t("canvas-edit-hint-bound", &[])
                                        } else {
                                            tr.t("canvas-edit-hint", &[])
                                        })
                                        .size(theme::TEXT_XS)
                                        .color(pal().text_faint),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if widgets::primary_button(
                                                ui,
                                                None,
                                                &tr.t("canvas-edit-done", &[]),
                                            )
                                            .clicked()
                                            {
                                                close_edit = true;
                                            }
                                            if bindable {
                                                let label = if is_bound {
                                                    tr.t("canvas-unbind", &[])
                                                } else {
                                                    tr.t("canvas-bind", &[])
                                                };
                                                if widgets::ghost_button(ui, Some(ICON_LINK.codepoint), &label)
                                                    .clicked()
                                                {
                                                    bind = Some((editing_id, !is_bound));
                                                    close_edit = true;
                                                }
                                            }
                                        },
                                    );
                                });
                            });
                    });
                if changed {
                    modified = true;
                    if let Some(elem) = canvas.get_element_mut(editing_id) {
                        match elem {
                            CanvasElement::StickyNote { text, .. }
                            | CanvasElement::Shape { text, .. } => *text = text_buf,
                            CanvasElement::Connector { label, .. } => *label = text_buf,
                            _ => {}
                        }
                    }
                }
                if close_edit {
                    interaction.editing_text_elem = None;
                }
            }
            None => interaction.editing_text_elem = None,
        }
    }

    // 6. Tool dock (top-left of the canvas).
    egui::Area::new(Id::new("mnemonic_canvas_tool_dock"))
        .fixed_pos(screen_rect.min + Vec2::new(12.0, 12.0))
        .order(egui::Order::Middle)
        .show(&ctx, |ui| {
            match ui::LeftToolbar::show_canvas(ui, tr, interaction.active_tool) {
                Some(ui::LeftToolbarEvent::SelectCanvasTool(tool)) => {
                    interaction.active_tool = tool
                }
                Some(ui::LeftToolbarEvent::ExportDrawio) => {
                    if let Some(save_path) = rfd::FileDialog::new()
                        .add_filter("Draw.io", &["drawio", "xml"])
                        .set_file_name(format!("{}.drawio", canvas.title.replace(' ', "_")))
                        .save_file()
                    {
                        toast = Some(match std::fs::write(&save_path, canvas.to_drawio_xml()) {
                            Ok(()) => (ToastKind::Success, tr.t("canvas-export-success", &[])),
                            Err(e) => (
                                ToastKind::Error,
                                format!("{}: {e}", tr.t("canvas-export-failed", &[])),
                            ),
                        });
                    }
                }
                Some(ui::LeftToolbarEvent::ImportDrawio) => {
                    if let Some(load_path) = rfd::FileDialog::new()
                        .add_filter("Draw.io", &["drawio", "xml"])
                        .pick_file()
                    {
                        let title = load_path
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| canvas.title.clone());
                        // Bound import (§Fase 3): Draw.io "text" shapes become
                        // Markdown blocks, every other shape stays diagram-only.
                        let result = std::fs::read_to_string(&load_path)
                            .map_err(anyhow::Error::from)
                            .and_then(|xml| canvas::DrawioImporter::from_xml_bound(&title, &xml));
                        toast = Some(match result {
                            Ok((imported, _)) if imported.elements.is_empty() => {
                                (ToastKind::Error, tr.t("canvas-import-empty", &[]))
                            }
                            Ok((mut imported, new_blocks)) => {
                                let bounds = imported
                                    .elements
                                    .iter()
                                    .map(|e| e.bounding_rect())
                                    .fold(egui::Rect::NOTHING, |acc, r| acc.union(r));
                                imported.viewport = canvas.viewport.clone();
                                imported.viewport.fit_rect(bounds, screen_rect.size());
                                let count = imported.elements.len().to_string();
                                let bound = new_blocks.len().to_string();
                                let message = format!(
                                    "{} · {}",
                                    tr.t("canvas-import-success", &[("count", &count)]),
                                    tr.t("canvas-import-bound", &[("count", &bound)])
                                );
                                imported_doc = Some((imported, new_blocks));
                                (ToastKind::Success, message)
                            }
                            Err(e) => (
                                ToastKind::Error,
                                format!("{}: {e}", tr.t("canvas-import-failed", &[])),
                            ),
                        });
                    }
                }
                None => {}
            }
        });

    // 7. Zoom HUD (bottom-right) and style HUD (bottom-center).
    egui::Area::new(Id::new("mnemonic_canvas_zoom_hud"))
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(screen_rect.right_bottom() - Vec2::new(16.0, 16.0))
        .order(egui::Order::Middle)
        .show(&ctx, |ui| {
            match ui::CanvasHud::show_zoom_hud(ui, tr, canvas.viewport.zoom) {
                Some(ui::CanvasHudEvent::ZoomIn) => {
                    canvas.viewport.zoom = (canvas.viewport.zoom * 1.15).min(5.0);
                    modified = true;
                }
                Some(ui::CanvasHudEvent::ZoomOut) => {
                    canvas.viewport.zoom = (canvas.viewport.zoom / 1.15).max(0.2);
                    modified = true;
                }
                Some(ui::CanvasHudEvent::ResetZoom) => {
                    canvas.viewport.zoom = 1.0;
                    modified = true;
                }
                _ => {}
            }
        });

    egui::Area::new(Id::new("mnemonic_canvas_style_hud"))
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(screen_rect.center_bottom() - Vec2::new(0.0, 16.0))
        .order(egui::Order::Middle)
        .show(&ctx, |ui| {
            match ui::CanvasHud::show_style_hud(
                ui,
                tr,
                interaction.primary_color,
                interaction.stroke_width,
            ) {
                Some(ui::CanvasHudEvent::SetStrokeColor(col)) => interaction.primary_color = col,
                Some(ui::CanvasHudEvent::SetStrokeWidth(w)) => interaction.stroke_width = w,
                _ => {}
            }
        });

    CanvasOutcome {
        modified,
        imported: imported_doc,
        bind,
        toast,
    }
}

/// Applies what the canvas surface asked for to the editor and returns
/// the toast to show, if any.
fn apply_canvas_outcome(editor: &mut MarkdownEditor, outcome: CanvasOutcome) -> Option<(ToastKind, String)> {
    if let Some((doc, blocks)) = outcome.imported {
        editor.import_bound_canvas(doc, blocks);
    } else if outcome.modified {
        editor.sync_canvas_to_body();
    }
    if let Some((id, bind)) = outcome.bind {
        if bind {
            editor.bind_element_to_note(id);
        } else {
            editor.unbind_element(id);
        }
    }
    outcome.toast
}
