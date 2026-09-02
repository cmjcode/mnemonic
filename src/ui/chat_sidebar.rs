//! Slide-over Glass AI Chat Sidebar Drawer bergaya Shapr3D / DUCAD.
//!
//! Menampilkan panel mengambang di sisi kanan (AI Copilot sidebar) untuk:
//! 1. Tanya jawab interaktif dengan Asisten AI lokal (RAG over Vault)
//! 2. Tampilan gelembung percakapan (user & assistant bubbles)
//! 3. Sitasi sumber dokumen catatan & PDF yang dapat langsung diklik
//! 4. Input teks percakapan dengan shortcut Enter

use std::path::PathBuf;

use egui::{
    Color32, CornerRadius, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke,
    Ui, Vec2,
};
use egui_icons::icons::{
    ICON_AUTO_AWESOME, ICON_CLOSE, ICON_DESCRIPTION, ICON_PICTURE_AS_PDF, ICON_RESTART_ALT,
};

use crate::ui::theme::{
    glass_panel_frame, ACCENT_BLUE, BG_CARD_DARK, BG_HOVER_DARK, BORDER_SUBTLE,
    CHAT_SIDEBAR_WIDTH, ROUNDING_MD, ROUNDING_SM, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
    TOPBAR_HEIGHT,
};

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

/// State untuk merender Chat Sidebar.
pub struct ChatSidebarState<'a> {
    pub is_open: bool,
    pub messages: &'a [ChatMessageItem],
    pub busy: bool,
    pub input_text: &'a mut String,
}

/// Event interaksi yang dihasilkan oleh Chat Sidebar.
#[derive(Debug, Clone)]
pub enum ChatSidebarEvent {
    SendMessage(String),
    ClearHistory,
    OpenCitation(ChatCitationItem),
    Close,
}

pub struct ChatSidebarDrawer;

impl ChatSidebarDrawer {
    /// Render slide-over AI chat sidebar drawer di sisi kanan layar.
    pub fn show(ctx: &egui::Context, state: &mut ChatSidebarState) -> Option<ChatSidebarEvent> {
        if !state.is_open {
            return None;
        }

        let mut event = None;
        let screen_rect = ctx.viewport_rect();
        let top_offset = TOPBAR_HEIGHT + 14.0;
        let sidebar_width = CHAT_SIDEBAR_WIDTH.min(screen_rect.width() - 24.0);

        let sidebar_rect = Rect::from_min_size(
            Pos2::new(
                screen_rect.max.x - sidebar_width - 12.0,
                screen_rect.min.y + top_offset,
            ),
            Vec2::new(
                sidebar_width,
                (screen_rect.height() - top_offset - 16.0).max(280.0),
            ),
        );

        egui::Window::new("ai_chat_glass_sidebar")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_rect(sidebar_rect)
            .frame(glass_panel_frame())
            .show(ctx, |ui| {
                ui.set_width(sidebar_width);
                ui.add_space(8.0);

                // ── Header AI Chat (Title, AI Badge, Clear & Close) ──
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(ICON_AUTO_AWESOME.codepoint)
                            .size(16.0)
                            .color(ACCENT_BLUE),
                    );
                    ui.label(
                        RichText::new("Asisten AI")
                            .size(14.0)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );

                    // Badge RAG Copilot
                    let badge_frame = Frame {
                        inner_margin: Margin::symmetric(6, 2),
                        outer_margin: Margin::ZERO,
                        corner_radius: CornerRadius::same(ROUNDING_SM),
                        fill: Color32::from_rgba_premultiplied(10, 132, 255, 30),
                        stroke: Stroke::new(0.5, ACCENT_BLUE),
                        shadow: egui::Shadow::NONE,
                    };
                    badge_frame.show(ui, |ui| {
                        ui.label(
                            RichText::new("RAG Vault")
                                .size(10.0)
                                .color(ACCENT_BLUE),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);

                        // Tombol Tutup
                        let close_btn = egui::Button::new(
                            RichText::new(ICON_CLOSE.codepoint)
                                .size(14.0)
                                .color(TEXT_SECONDARY),
                        )
                        .frame(false);

                        if ui.add(close_btn).on_hover_text("Tutup Sidebar AI (Esc)").clicked() {
                            event = Some(ChatSidebarEvent::Close);
                        }

                        // Tombol Bersihkan Percakapan
                        let clear_btn = egui::Button::new(
                            RichText::new(ICON_RESTART_ALT.codepoint)
                                .size(13.0)
                                .color(TEXT_MUTED),
                        )
                        .frame(false);

                        if ui
                            .add(clear_btn)
                            .on_hover_text("Bersihkan Riwayat Percakapan")
                            .clicked()
                        {
                            event = Some(ChatSidebarEvent::ClearHistory);
                        }
                    });
                });

                ui.add_space(6.0);
                ui.separator();

                // ── Area Riwayat Pesan (Scrollable) ──
                let scroll_height = (ui.available_height() - 76.0).max(120.0);
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(scroll_height)
                    .show(ui, |ui| {
                        ui.add_space(6.0);

                        // State Kosong (Empty State)
                        if state.messages.is_empty() {
                            Self::render_empty_state(ui, state.input_text);
                        }

                        // Render Semua Gelembung Pesan
                        for msg in state.messages {
                            Self::render_message_bubble(ui, msg, &mut event);
                            ui.add_space(8.0);
                        }

                        // Indikator Berpikir / Streaming
                        if state.busy {
                            Self::render_thinking_bubble(ui);
                            ui.add_space(6.0);
                        }
                    });

                // ── Input Box Row di Bagian Bawah ──
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                let can_send = !state.busy && !state.input_text.trim().is_empty();

                ui.horizontal(|ui| {
                    ui.add_space(6.0);

                    let input_width = ui.available_width() - 54.0;
                    let input_field = egui::TextEdit::singleline(state.input_text)
                        .hint_text("Tanya asisten tentang vault...")
                        .desired_width(input_width)
                        .margin(Margin::symmetric(8, 6));

                    let resp = ui.add(input_field);
                    let enter_pressed = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    let send_btn = egui::Button::new(
                        RichText::new(ICON_AUTO_AWESOME.codepoint)
                            .size(13.0)
                            .color(if can_send { Color32::WHITE } else { TEXT_MUTED }),
                    )
                    .fill(if can_send { ACCENT_BLUE } else { BG_CARD_DARK })
                    .corner_radius(CornerRadius::same(ROUNDING_SM));

                    let send_resp = ui.add_sized(Vec2::new(38.0, 26.0), send_btn);

                    if can_send && (send_resp.clicked() || enter_pressed) {
                        let text = state.input_text.trim().to_string();
                        state.input_text.clear();
                        event = Some(ChatSidebarEvent::SendMessage(text));
                    }
                });
            });

        event
    }

    /// Render tampilan saat belum ada riwayat percakapan.
    fn render_empty_state(ui: &mut Ui, input_text: &mut String) {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.label(
                RichText::new(ICON_AUTO_AWESOME.codepoint)
                    .size(32.0)
                    .color(ACCENT_BLUE),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new("Asisten Pengetahuan Anda")
                    .size(13.5)
                    .strong()
                    .color(TEXT_PRIMARY),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("Ajukan pertanyaan atau minta bantuan ringkasan seputar isi catatan dan berkas PDF dalam vault Anda.")
                    .size(11.5)
                    .color(TEXT_SECONDARY),
            );

            ui.add_space(16.0);

            // Starter prompt chips
            let prompts = [
                "💡 Ringkas poin penting dari catatan saya",
                "🔍 Cari topik terkait konsep kunci",
                "📝 Bantu susun ide & hubungan catatan",
            ];

            for prompt in prompts {
                let chip_frame = Frame {
                    inner_margin: Margin::symmetric(10, 6),
                    outer_margin: Margin::symmetric(0, 2),
                    corner_radius: CornerRadius::same(ROUNDING_SM),
                    fill: BG_CARD_DARK,
                    stroke: Stroke::new(0.5, BORDER_SUBTLE),
                    shadow: egui::Shadow::NONE,
                };

                let resp = chip_frame
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width() - 20.0);
                        ui.label(
                            RichText::new(prompt)
                                .size(11.0)
                                .color(TEXT_SECONDARY),
                        );
                    })
                    .response;

                if resp.interact(Sense::click()).on_hover_text("Gunakan prompt ini").clicked() {
                    *input_text = prompt[4..].to_string(); // Potong emoji di awal
                }
            }
        });
    }

    /// Render satu bubble pesan (User atau Assistant).
    fn render_message_bubble(
        ui: &mut Ui,
        msg: &ChatMessageItem,
        event: &mut Option<ChatSidebarEvent>,
    ) {
        let is_user = msg.role == ChatRole::User;
        let align = if is_user {
            egui::Align::Max
        } else {
            egui::Align::Min
        };

        ui.with_layout(egui::Layout::top_down(align), |ui| {
            let max_w = ui.available_width() * 0.88;

            let bubble_frame = if is_user {
                Frame {
                    inner_margin: Margin::symmetric(10, 8),
                    outer_margin: Margin::ZERO,
                    corner_radius: CornerRadius::same(ROUNDING_MD),
                    fill: Color32::from_rgba_premultiplied(10, 132, 255, 40),
                    stroke: Stroke::new(0.8, ACCENT_BLUE),
                    shadow: egui::Shadow::NONE,
                }
            } else {
                Frame {
                    inner_margin: Margin::symmetric(10, 8),
                    outer_margin: Margin::ZERO,
                    corner_radius: CornerRadius::same(ROUNDING_MD),
                    fill: BG_CARD_DARK,
                    stroke: Stroke::new(0.5, BORDER_SUBTLE),
                    shadow: egui::Shadow::NONE,
                }
            };

            bubble_frame.show(ui, |ui| {
                ui.set_max_width(max_w);

                if !is_user {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(ICON_AUTO_AWESOME.codepoint)
                                .size(11.0)
                                .color(ACCENT_BLUE),
                        );
                        ui.label(
                            RichText::new("Mnemonic AI")
                                .size(10.5)
                                .strong()
                                .color(TEXT_SECONDARY),
                        );
                    });
                    ui.add_space(2.0);
                }

                ui.label(
                    RichText::new(&msg.text)
                        .size(12.0)
                        .color(if is_user { Color32::WHITE } else { TEXT_PRIMARY }),
                );

                // Render Sitasi Referensi jika ada
                if !msg.citations.is_empty() {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("Sumber Referensi:")
                            .size(10.0)
                            .color(TEXT_MUTED),
                    );

                    for citation in &msg.citations {
                        let is_pdf = citation.page_index.is_some();
                        let cit_icon = if is_pdf {
                            ICON_PICTURE_AS_PDF.codepoint
                        } else {
                            ICON_DESCRIPTION.codepoint
                        };

                        let cit_frame = Frame {
                            inner_margin: Margin::symmetric(6, 3),
                            outer_margin: Margin::symmetric(0, 1),
                            corner_radius: CornerRadius::same(ROUNDING_SM),
                            fill: BG_HOVER_DARK,
                            stroke: Stroke::new(0.5, BORDER_SUBTLE),
                            shadow: egui::Shadow::NONE,
                        };

                        let cit_resp = cit_frame
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(cit_icon)
                                            .size(11.0)
                                            .color(if is_pdf { Color32::from_rgb(255, 69, 58) } else { ACCENT_BLUE }),
                                    );
                                    ui.label(
                                        RichText::new(&citation.label)
                                            .size(10.5)
                                            .color(TEXT_SECONDARY),
                                    );
                                });
                            })
                            .response;

                        if cit_resp
                            .interact(Sense::click())
                            .on_hover_text("Buka berkas referensi ini")
                            .clicked()
                        {
                            *event = Some(ChatSidebarEvent::OpenCitation(citation.clone()));
                        }
                    }
                }
            });
        });
    }

    /// Render bubble indikator bahwa asisten sedang memikirkan jawaban.
    fn render_thinking_bubble(ui: &mut Ui) {
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            let bubble_frame = Frame {
                inner_margin: Margin::symmetric(10, 6),
                outer_margin: Margin::ZERO,
                corner_radius: CornerRadius::same(ROUNDING_MD),
                fill: BG_CARD_DARK,
                stroke: Stroke::new(0.5, BORDER_SUBTLE),
                shadow: egui::Shadow::NONE,
            };

            bubble_frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(ICON_AUTO_AWESOME.codepoint)
                            .size(11.0)
                            .color(ACCENT_BLUE),
                    );
                    ui.label(
                        RichText::new("Memikirkan jawaban & menelusuri vault...")
                            .size(11.0)
                            .italics()
                            .color(TEXT_MUTED),
                    );
                });
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
        assert_eq!(user_msg.text, "Halo AI");
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
