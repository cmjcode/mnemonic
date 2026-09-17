//! AI assistant panel, docked on the right so it never covers the note
//! being read: chat bubbles, clickable source citations, starter prompts,
//! and an input that sends on Enter.

use std::path::PathBuf;

use egui::{Align, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2};
use egui_icons::icons::{
    ICON_ARROW_UPWARD, ICON_AUTO_AWESOME, ICON_CLOSE, ICON_DESCRIPTION, ICON_PICTURE_AS_PDF,
    ICON_RESTART_ALT,
};

use crate::i18n::LocaleManager;
use crate::ui::theme::{self, pal};
use crate::ui::widgets;

/// Role pengirim pesan percakapan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

/// Satu sitasi referensi yang dihasilkan asisten AI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatCitationItem {
    pub file_path: PathBuf,
    pub page_index: Option<usize>,
    pub label: String,
}

/// Satu pesan di dalam riwayat percakapan chat.
#[derive(Debug, Clone)]
pub struct ChatMessageItem {
    pub role: ChatRole,
    pub text: String,
    pub citations: Vec<ChatCitationItem>,
}

impl ChatMessageItem {
    pub fn user(text: String) -> Self {
        Self {
            role: ChatRole::User,
            text,
            citations: Vec::new(),
        }
    }

    pub fn assistant(text: String, citations: Vec<ChatCitationItem>) -> Self {
        Self {
            role: ChatRole::Assistant,
            text,
            citations,
        }
    }
}

pub struct ChatSidebarState<'a> {
    pub messages: &'a [ChatMessageItem],
    pub busy: bool,
    pub input_text: &'a mut String,
}

#[derive(Debug, Clone)]
pub enum ChatSidebarEvent {
    SendMessage(String),
    ClearHistory,
    OpenCitation(ChatCitationItem),
    Close,
}

pub struct ChatSidebarDrawer;

impl ChatSidebarDrawer {
    pub fn input_id() -> egui::Id {
        egui::Id::new("mnemonic_chat_input")
    }

    /// Renders the docked right panel (call only while it is open).
    pub fn show(
        ui: &mut Ui,
        tr: &LocaleManager,
        state: &mut ChatSidebarState,
    ) -> Option<ChatSidebarEvent> {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let mut event = None;

        egui::Panel::right("mnemonic_ai_panel")
            .resizable(true)
            .default_size(theme::CHAT_SIDEBAR_WIDTH)
            .size_range(300.0..=560.0)
            .frame(theme::side_panel_frame().inner_margin(Margin::same(12)))
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(ICON_AUTO_AWESOME.codepoint)
                            .size(18.0)
                            .color(p.accent),
                    );
                    ui.label(
                        RichText::new(t("chat-title"))
                            .font(theme::semibold(theme::TEXT_BODY + 1.0))
                            .color(p.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::icon_button(ui, ICON_CLOSE.codepoint, &t("chat-close"), false)
                            .clicked()
                        {
                            event = Some(ChatSidebarEvent::Close);
                        }
                        if !state.messages.is_empty()
                            && widgets::icon_button(
                                ui,
                                ICON_RESTART_ALT.codepoint,
                                &t("chat-clear"),
                                false,
                            )
                            .clicked()
                        {
                            event = Some(ChatSidebarEvent::ClearHistory);
                        }
                    });
                });
                ui.label(
                    RichText::new(t("chat-subtitle"))
                        .size(theme::TEXT_XS)
                        .color(p.text_faint),
                );
                ui.add_space(theme::SPACE_S);

                // Input pinned to the bottom of the panel.
                egui::Panel::bottom("mnemonic_ai_input")
                    .frame(Frame::NONE.inner_margin(Margin::symmetric(0, 8)))
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        Self::input_row(ui, tr, state, &mut event);
                    });

                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if state.messages.is_empty() {
                            Self::empty_state(ui, tr, state.input_text);
                        }
                        for msg in state.messages {
                            Self::bubble(ui, tr, msg, &mut event);
                            ui.add_space(theme::SPACE_M);
                        }
                        if state.busy {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().size(14.0).color(p.accent));
                                ui.label(
                                    RichText::new(t("chat-thinking"))
                                        .size(theme::TEXT_SM)
                                        .color(p.text_faint),
                                );
                            });
                        }
                    });
            });
        event
    }

    fn input_row(
        ui: &mut Ui,
        tr: &LocaleManager,
        state: &mut ChatSidebarState,
        event: &mut Option<ChatSidebarEvent>,
    ) {
        let p = pal();
        let can_send = !state.busy && !state.input_text.trim().is_empty();
        Frame::NONE
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(theme::RADIUS_LG))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(state.input_text)
                            .id(Self::input_id())
                            .hint_text(
                                RichText::new(tr.t("chat-placeholder", &[])).color(p.text_faint),
                            )
                            .frame(Frame::NONE)
                            .font(egui::FontId::proportional(theme::TEXT_BODY))
                            .desired_width(ui.available_width() - 38.0),
                    );
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    let (rect, send) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                    ui.painter().circle_filled(
                        rect.center(),
                        15.0,
                        if can_send { p.accent } else { p.hover },
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        ICON_ARROW_UPWARD.codepoint,
                        egui::FontId::proportional(18.0),
                        if can_send { p.on_accent } else { p.text_faint },
                    );
                    let send = send.on_hover_text(tr.t("chat-send", &[]));
                    if can_send && (send.clicked() || enter) {
                        let text = state.input_text.trim().to_string();
                        state.input_text.clear();
                        *event = Some(ChatSidebarEvent::SendMessage(text));
                        resp.request_focus();
                    }
                });
            });
    }

    fn empty_state(ui: &mut Ui, tr: &LocaleManager, input_text: &mut String) {
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        ui.add_space(theme::SPACE_XL);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(ICON_AUTO_AWESOME.codepoint)
                    .size(34.0)
                    .color(p.accent),
            );
            ui.add_space(theme::SPACE_S);
            ui.label(
                RichText::new(t("chat-empty-title"))
                    .font(theme::semibold(theme::TEXT_BODY + 1.0))
                    .color(p.text),
            );
            ui.add_space(theme::SPACE_XS);
            ui.label(
                RichText::new(t("chat-empty"))
                    .size(theme::TEXT_SM)
                    .color(p.text_dim),
            );
        });
        ui.add_space(theme::SPACE_L);
        for key in [
            "chat-starter-summary",
            "chat-starter-related",
            "chat-starter-ideas",
        ] {
            let prompt = t(key);
            let resp = Frame::NONE
                .fill(p.card)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(theme::RADIUS_MD))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&prompt).size(theme::TEXT_SM).color(p.text));
                })
                .response
                .interact(Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                *input_text = prompt;
                ui.ctx().memory_mut(|m| m.request_focus(Self::input_id()));
            }
            ui.add_space(6.0);
        }
    }

    fn bubble(
        ui: &mut Ui,
        tr: &LocaleManager,
        msg: &ChatMessageItem,
        event: &mut Option<ChatSidebarEvent>,
    ) {
        let p = pal();
        let is_user = msg.role == ChatRole::User;
        let align = if is_user { Align::Max } else { Align::Min };
        ui.with_layout(Layout::top_down(align), |ui| {
            let max_w = ui.available_width() * 0.9;
            let frame = if is_user {
                Frame::NONE.fill(p.accent_soft)
            } else {
                Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, p.border))
            };
            frame
                .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                .inner_margin(Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.set_max_width(max_w);
                    ui.label(
                        RichText::new(&msg.text)
                            .size(theme::TEXT_BODY)
                            .color(p.text),
                    );

                    if !msg.citations.is_empty() {
                        ui.add_space(theme::SPACE_S);
                        ui.label(
                            RichText::new(tr.t("chat-sources", &[]))
                                .size(theme::TEXT_XS)
                                .color(p.text_faint),
                        );
                        for citation in &msg.citations {
                            let (icon, color) = if citation.page_index.is_some() {
                                (ICON_PICTURE_AS_PDF.codepoint, p.pdf_icon)
                            } else {
                                (ICON_DESCRIPTION.codepoint, p.note_icon)
                            };
                            let resp = widgets::list_row(
                                ui,
                                widgets::RowSpec {
                                    icon,
                                    icon_color: color,
                                    label: &citation.label,
                                    trailing: None,
                                    selected: false,
                                    indent: 0.0,
                                    reserve_right: 0.0,
                                },
                            )
                            .on_hover_text(tr.t("chat-open-source", &[]));
                            if resp.clicked() {
                                *event = Some(ChatSidebarEvent::OpenCitation(citation.clone()));
                            }
                        }
                    }
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_creation() {
        let user_msg = ChatMessageItem::user("Halo AI".to_string());
        assert_eq!(user_msg.role, ChatRole::User);
        assert!(user_msg.citations.is_empty());

        let cit = ChatCitationItem {
            file_path: PathBuf::from("note.md"),
            page_index: None,
            label: "note.md".to_string(),
        };
        let asst_msg = ChatMessageItem::assistant("Ini jawaban".to_string(), vec![cit]);
        assert_eq!(asst_msg.role, ChatRole::Assistant);
        assert_eq!(asst_msg.citations.len(), 1);
    }
}
