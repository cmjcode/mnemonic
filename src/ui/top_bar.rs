//! The app bar: a docked, context-aware top bar that stays visible on every
//! screen so navigation never "disappears".
//!
//! - Home: sidebar toggle + one global search field (⌘F).
//! - Editor: back, inline-editable title (single click), Write/Read/Canvas
//!   switcher, undo/redo, outline toggle, and a live save indicator.
//! - PDF: back + file name.
//!
//! The right side always has the indexing status, AI assistant toggle,
//! and a settings menu (theme, language, vault, shortcuts).

use egui::{Align, Id, Layout, RichText, Ui};
use egui_icons::icons::{
    ICON_ARROW_BACK, ICON_AUTO_AWESOME, ICON_CHECK, ICON_CLOUD_DONE, ICON_DARK_MODE, ICON_DRAW,
    ICON_EDIT_NOTE, ICON_ERROR, ICON_FOLDER_OPEN, ICON_HUB, ICON_KEYBOARD, ICON_LEFT_PANEL_CLOSE,
    ICON_LEFT_PANEL_OPEN, ICON_LIGHT_MODE, ICON_REDO, ICON_SEARCH, ICON_SETTINGS, ICON_SYNC,
    ICON_TOC, ICON_UNDO, ICON_VERTICAL_SPLIT, ICON_VISIBILITY,
};

use crate::i18n::LocaleManager;
use crate::ui::theme::{self, ThemeMode, pal};
use crate::ui::widgets;

/// Id of the global search field, so ⌘F can focus it from anywhere.
pub fn search_field_id() -> Id {
    Id::new("mnemonic_global_search")
}

/// Id of the editor title field.
pub fn title_field_id() -> Id {
    Id::new("mnemonic_title_edit")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorModeTab {
    Write,
    Read,
    Canvas,
    /// Markdown beside its diagram.
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    Saved,
    Pending,
    Failed,
}

pub enum TopBarContext<'a> {
    Welcome,
    Home {
        search: &'a mut String,
    },
    Editor {
        title: &'a mut String,
        mode: EditorModeTab,
        save_state: SaveState,
        can_undo: bool,
        can_redo: bool,
        outline_open: bool,
    },
    Pdf {
        title: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopBarEvent {
    ToggleSidebar,
    ToggleChat,
    OpenCommandPalette,
    SetTheme(ThemeMode),
    SetLanguage(String),
    OpenVaultPicker,
    ShowShortcuts,
    SearchChanged,
    Back,
    CommitTitle,
    SetEditorMode(EditorModeTab),
    Undo,
    Redo,
    ToggleOutline,
    ToggleGraph,
}

pub struct TopBarState<'a> {
    pub context: TopBarContext<'a>,
    pub vault_open: bool,
    pub sidebar_open: bool,
    pub chat_open: bool,
    /// The full-screen graph view is showing.
    pub graph_open: bool,
    pub theme_mode: ThemeMode,
    /// Background indexing jobs still running (0 hides the indicator).
    pub indexing_jobs: usize,
}

pub struct TopBar;

impl TopBar {
    /// Renders the bar as a docked top panel and returns this frame's events.
    pub fn show(ui: &mut Ui, tr: &LocaleManager, state: &mut TopBarState) -> Vec<TopBarEvent> {
        let t = |key: &str| tr.t(key, &[]);
        let mut events = Vec::new();
        let p = pal();

        egui::Panel::top("mnemonic_top_bar")
            .exact_size(theme::TOPBAR_HEIGHT)
            .frame(theme::top_bar_frame())
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    if state.vault_open {
                        let (icon, tip) = if state.sidebar_open {
                            (ICON_LEFT_PANEL_CLOSE.codepoint, t("topbar-hide-sidebar"))
                        } else {
                            (ICON_LEFT_PANEL_OPEN.codepoint, t("topbar-show-sidebar"))
                        };
                        if widgets::icon_button(ui, icon, &format!("{tip}  ⌘\\"), false).clicked()
                        {
                            events.push(TopBarEvent::ToggleSidebar);
                        }
                    }

                    match &mut state.context {
                        TopBarContext::Welcome => {
                            ui.add_space(6.0);
                            let logo = crate::ui::logo::logo_texture(ui.ctx());
                            ui.add(
                                egui::Image::new((logo.id(), egui::Vec2::splat(22.0)))
                                    .corner_radius(5.0),
                            );
                            ui.label(
                                RichText::new("MNEMONIC")
                                    .font(theme::semibold(theme::TEXT_BODY))
                                    .color(p.text),
                            );
                        }
                        TopBarContext::Home { search } => {
                            ui.add_space(6.0);
                            let width = (ui.available_width() * 0.5).clamp(200.0, 480.0);
                            let resp = widgets::search_field(
                                ui,
                                search_field_id(),
                                search,
                                &t("search-placeholder"),
                                width,
                                Some("⌘F"),
                            );
                            if resp.changed() {
                                events.push(TopBarEvent::SearchChanged);
                            }
                        }
                        TopBarContext::Editor { title, .. } => {
                            if widgets::icon_button(
                                ui,
                                ICON_ARROW_BACK.codepoint,
                                &format!("{}  Esc", t("editor-back")),
                                false,
                            )
                            .clicked()
                            {
                                events.push(TopBarEvent::Back);
                            }
                            ui.add_space(4.0);
                            let width = (ui.available_width() - 460.0).clamp(120.0, 520.0);
                            let resp = ui.add(
                                egui::TextEdit::singleline(*title)
                                    .id(title_field_id())
                                    .font(theme::semibold(theme::TEXT_LG - 1.0))
                                    .text_color(p.text)
                                    .frame(egui::Frame::NONE)
                                    .hint_text(t("editor-untitled"))
                                    .desired_width(width),
                            );
                            if resp.hovered() && !resp.has_focus() {
                                ui.painter().rect_stroke(
                                    resp.rect.expand2(egui::vec2(6.0, 4.0)),
                                    egui::CornerRadius::same(theme::RADIUS_SM),
                                    egui::Stroke::new(1.0, p.border),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            let resp = resp.on_hover_text(t("editor-rename-hint"));
                            // Leaving the field by any means (Enter, Esc, click
                            // elsewhere) keeps what was typed — never discard it.
                            if resp.lost_focus() {
                                events.push(TopBarEvent::CommitTitle);
                            }
                        }
                        TopBarContext::Pdf { title } => {
                            if widgets::icon_button(
                                ui,
                                ICON_ARROW_BACK.codepoint,
                                &format!("{}  Esc", t("editor-back")),
                                false,
                            )
                            .clicked()
                            {
                                events.push(TopBarEvent::Back);
                            }
                            ui.add_space(4.0);
                            let galley = widgets::elided_galley(
                                ui,
                                title,
                                theme::semibold(theme::TEXT_LG - 1.0),
                                p.text,
                                (ui.available_width() - 200.0).max(80.0),
                            );
                            ui.label(galley);
                        }
                    }

                    // Right side is laid out right-to-left: first added = rightmost.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        Self::settings_menu(ui, tr, state, &mut events);

                        if state.vault_open {
                            if widgets::icon_button(
                                ui,
                                ICON_AUTO_AWESOME.codepoint,
                                &format!("{}  ⌘J", t("topbar-ai-assistant")),
                                state.chat_open,
                            )
                            .clicked()
                            {
                                events.push(TopBarEvent::ToggleChat);
                            }
                            if widgets::icon_button(
                                ui,
                                ICON_HUB.codepoint,
                                &format!("{}  ⌘G", t("graph-title")),
                                state.graph_open,
                            )
                            .clicked()
                            {
                                events.push(TopBarEvent::ToggleGraph);
                            }
                            if !matches!(state.context, TopBarContext::Home { .. })
                                && widgets::icon_button(
                                    ui,
                                    ICON_SEARCH.codepoint,
                                    &format!("{}  ⌘K", t("settings-command-palette")),
                                    false,
                                )
                                .clicked()
                            {
                                events.push(TopBarEvent::OpenCommandPalette);
                            }
                        }

                        if state.indexing_jobs > 0 {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(tr.t(
                                    "topbar-indexing",
                                    &[("count", &state.indexing_jobs.to_string())],
                                ))
                                .size(theme::TEXT_XS)
                                .color(p.text_faint),
                            )
                            .on_hover_text(t("topbar-indexing-hint"));
                            ui.add(egui::Spinner::new().size(14.0).color(p.text_faint));
                        }

                        if let TopBarContext::Editor {
                            mode,
                            save_state,
                            can_undo,
                            can_redo,
                            outline_open,
                            ..
                        } = &state.context
                        {
                            Self::editor_controls(
                                ui,
                                tr,
                                *mode,
                                *save_state,
                                *can_undo,
                                *can_redo,
                                *outline_open,
                                &mut events,
                            );
                        }
                    });
                });
            });

        events
    }

    #[allow(clippy::too_many_arguments)]
    fn editor_controls(
        ui: &mut Ui,
        tr: &LocaleManager,
        mode: EditorModeTab,
        save_state: SaveState,
        can_undo: bool,
        can_redo: bool,
        outline_open: bool,
        events: &mut Vec<TopBarEvent>,
    ) {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        ui.add_space(4.0);
        if mode != EditorModeTab::Canvas
            && widgets::icon_button(
                ui,
                ICON_TOC.codepoint,
                &t("editor-outline-toggle"),
                outline_open,
            )
            .clicked()
        {
            events.push(TopBarEvent::ToggleOutline);
        }
        if mode == EditorModeTab::Write {
            ui.add_enabled_ui(can_redo, |ui| {
                if widgets::icon_button(
                    ui,
                    ICON_REDO.codepoint,
                    &format!("{}  ⇧⌘Z", t("editor-redo")),
                    false,
                )
                .clicked()
                {
                    events.push(TopBarEvent::Redo);
                }
            });
            ui.add_enabled_ui(can_undo, |ui| {
                if widgets::icon_button(
                    ui,
                    ICON_UNDO.codepoint,
                    &format!("{}  ⌘Z", t("editor-undo")),
                    false,
                )
                .clicked()
                {
                    events.push(TopBarEvent::Undo);
                }
            });
        }
        ui.add_space(6.0);

        let labels = [
            t("editor-mode-write"),
            t("editor-mode-read"),
            t("editor-mode-split"),
            t("editor-mode-edgeless"),
        ];
        let options = [
            (ICON_EDIT_NOTE.codepoint, labels[0].as_str()),
            (ICON_VISIBILITY.codepoint, labels[1].as_str()),
            (ICON_VERTICAL_SPLIT.codepoint, labels[2].as_str()),
            (ICON_DRAW.codepoint, labels[3].as_str()),
        ];
        let selected = match mode {
            EditorModeTab::Write => 0,
            EditorModeTab::Read => 1,
            EditorModeTab::Split => 2,
            EditorModeTab::Canvas => 3,
        };
        if let Some(i) = widgets::segmented(ui, Id::new("editor_mode_switch"), &options, selected) {
            events.push(TopBarEvent::SetEditorMode(match i {
                0 => EditorModeTab::Write,
                1 => EditorModeTab::Read,
                2 => EditorModeTab::Split,
                _ => EditorModeTab::Canvas,
            }));
        }

        ui.add_space(8.0);
        let (icon, label, color) = match save_state {
            SaveState::Saved => (ICON_CLOUD_DONE.codepoint, t("editor-saved"), p.text_faint),
            SaveState::Pending => (ICON_SYNC.codepoint, t("editor-saving"), p.text_faint),
            SaveState::Failed => (ICON_ERROR.codepoint, t("editor-save-failed"), p.danger),
        };
        ui.label(RichText::new(label).size(theme::TEXT_XS).color(color));
        ui.label(RichText::new(icon).size(15.0).color(color));
    }

    fn settings_menu(
        ui: &mut Ui,
        tr: &LocaleManager,
        state: &TopBarState,
        events: &mut Vec<TopBarEvent>,
    ) {
        let t = |key: &str| tr.t(key, &[]);
        let resp = widgets::icon_button(ui, ICON_SETTINGS.codepoint, &t("topbar-settings"), false);
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(250.0);
            widgets::section_header(ui, &t("settings-appearance"));
            for (mode, icon, key) in [
                (
                    ThemeMode::Light,
                    ICON_LIGHT_MODE.codepoint,
                    "settings-theme-light",
                ),
                (
                    ThemeMode::Dark,
                    ICON_DARK_MODE.codepoint,
                    "settings-theme-dark",
                ),
            ] {
                let checked = state.theme_mode == mode;
                if widgets::menu_item(ui, icon, &t(key), checked.then_some(ICON_CHECK.codepoint))
                    .clicked()
                {
                    events.push(TopBarEvent::SetTheme(mode));
                    ui.close();
                }
            }

            widgets::section_header(ui, &t("settings-language"));
            for (locale, label) in [("id-ID", "Bahasa Indonesia"), ("en-US", "English")] {
                let checked = tr.active_locale() == locale;
                if widgets::menu_item(ui, "", label, checked.then_some(ICON_CHECK.codepoint))
                    .clicked()
                {
                    events.push(TopBarEvent::SetLanguage(locale.to_string()));
                    ui.close();
                }
            }

            ui.add_space(4.0);
            ui.separator();
            if state.vault_open {
                if widgets::menu_item(
                    ui,
                    ICON_FOLDER_OPEN.codepoint,
                    &t("settings-switch-vault"),
                    None,
                )
                .clicked()
                {
                    events.push(TopBarEvent::OpenVaultPicker);
                    ui.close();
                }
                if widgets::menu_item(
                    ui,
                    ICON_SEARCH.codepoint,
                    &t("settings-command-palette"),
                    Some("⌘K"),
                )
                .clicked()
                {
                    events.push(TopBarEvent::OpenCommandPalette);
                    ui.close();
                }
            }
            if widgets::menu_item(
                ui,
                ICON_KEYBOARD.codepoint,
                &t("settings-shortcuts"),
                Some("⌘/"),
            )
            .clicked()
            {
                events.push(TopBarEvent::ShowShortcuts);
                ui.close();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_mode_tabs_are_distinct() {
        assert_ne!(EditorModeTab::Write, EditorModeTab::Read);
        assert_ne!(SaveState::Saved, SaveState::Pending);
    }
}
