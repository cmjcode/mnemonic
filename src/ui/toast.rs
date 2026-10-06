//! Non-blocking toast notifications (bottom-center), replacing the old red
//! status banner: success confirmations, errors, and — most importantly —
//! an "Undo" action right after reversible operations like moving a note
//! to the trash, so the app never needs a "are you sure?" dialog for them.
//!
//! Generic over the action payload `A` so `ui` stays independent of app
//! types: `app.rs` decides what an action means when `show` returns it.

use std::time::{Duration, Instant};

use egui::{Align2, CornerRadius, FontId, Id, Margin, RichText, Sense, Stroke, Vec2};
use egui_icons::icons::{ICON_CHECK_CIRCLE, ICON_CLOSE, ICON_ERROR, ICON_INFO};

use super::theme::{self, pal};

const INFO_DURATION: Duration = Duration::from_secs(4);
const ACTION_DURATION: Duration = Duration::from_secs(7);
const ERROR_DURATION: Duration = Duration::from_secs(10);
const MAX_VISIBLE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

struct Toast<A> {
    id: u64,
    kind: ToastKind,
    text: String,
    action: Option<(String, A)>,
    shown_at: Instant,
    duration: Duration,
}

pub struct Toasts<A> {
    items: Vec<Toast<A>>,
    next_id: u64,
}

impl<A> Default for Toasts<A> {
    fn default() -> Self {
        Toasts {
            items: Vec::new(),
            next_id: 0,
        }
    }
}

impl<A: Clone> Toasts<A> {
    pub fn push(&mut self, kind: ToastKind, text: impl Into<String>) {
        let duration = match kind {
            ToastKind::Error => ERROR_DURATION,
            _ => INFO_DURATION,
        };
        self.insert(kind, text.into(), None, duration);
    }

    /// A toast with a clickable action (e.g. "Urungkan").
    pub fn push_with_action(
        &mut self,
        kind: ToastKind,
        text: impl Into<String>,
        action_label: impl Into<String>,
        action: A,
    ) {
        self.insert(
            kind,
            text.into(),
            Some((action_label.into(), action)),
            ACTION_DURATION,
        );
    }

    fn insert(
        &mut self,
        kind: ToastKind,
        text: String,
        action: Option<(String, A)>,
        duration: Duration,
    ) {
        self.next_id += 1;
        self.items.push(Toast {
            id: self.next_id,
            kind,
            text,
            action,
            shown_at: Instant::now(),
            duration,
        });
        if self.items.len() > MAX_VISIBLE {
            self.items.remove(0);
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Drops expired toasts. Split out from `show` so expiry is testable.
    fn expire(&mut self, now: Instant) {
        self.items
            .retain(|t| now.saturating_duration_since(t.shown_at) < t.duration);
    }

    /// Renders all toasts; returns the action of a clicked action button.
    /// Hovering a toast keeps it alive. Toasts always use the dark palette
    /// so they stand out against either theme.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<A> {
        self.expire(Instant::now());
        if self.items.is_empty() {
            return None;
        }

        let d = &theme::DARK;
        let screen = ctx.viewport_rect();
        let mut clicked_action = None;
        let mut dismissed: Vec<u64> = Vec::new();
        let mut hovered_ids: Vec<u64> = Vec::new();

        egui::Area::new(Id::new("mnemonic_toasts"))
            .order(egui::Order::Tooltip)
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -24.0))
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width((screen.width() - 32.0).min(560.0));
                ui.spacing_mut().item_spacing.y = 8.0;
                for toast in &self.items {
                    let (icon, icon_color) = match toast.kind {
                        ToastKind::Info => (ICON_INFO.codepoint, d.accent_hover),
                        ToastKind::Success => (ICON_CHECK_CIRCLE.codepoint, d.success),
                        ToastKind::Error => (ICON_ERROR.codepoint, d.danger),
                    };
                    let frame = egui::Frame::NONE
                        .fill(d.hover)
                        .stroke(Stroke::new(1.0, d.border_strong))
                        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                        .inner_margin(Margin::symmetric(14, 8))
                        .shadow(egui::Shadow {
                            offset: [0, 6],
                            blur: 20,
                            spread: 0,
                            color: pal().shadow,
                        });

                    let resp = frame
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon).size(18.0).color(icon_color));
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(&toast.text)
                                        .size(theme::TEXT_SM + 0.5)
                                        .color(d.text),
                                );
                                if let Some((label, action)) = &toast.action {
                                    ui.add_space(8.0);
                                    let action_resp = ui.add(
                                        egui::Label::new(
                                            RichText::new(label)
                                                .font(theme::semibold(theme::TEXT_SM + 0.5))
                                                .color(d.accent_hover),
                                        )
                                        .sense(Sense::click()),
                                    );
                                    if action_resp
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        clicked_action = Some(action.clone());
                                        dismissed.push(toast.id);
                                    }
                                }
                                ui.add_space(4.0);
                                let close = ui.add(
                                    egui::Label::new(
                                        RichText::new(ICON_CLOSE.codepoint)
                                            .font(FontId::proportional(15.0))
                                            .color(d.text_dim),
                                    )
                                    .sense(Sense::click()),
                                );
                                if close
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    dismissed.push(toast.id);
                                }
                            });
                        })
                        .response;
                    if resp.contains_pointer() {
                        hovered_ids.push(toast.id);
                    }
                }
            });

        let now = Instant::now();
        for toast in &mut self.items {
            if hovered_ids.contains(&toast.id) {
                toast.shown_at = now;
            }
        }
        self.items.retain(|t| !dismissed.contains(&t.id));

        if let Some(next_expiry) = self
            .items
            .iter()
            .map(|t| {
                t.duration
                    .saturating_sub(now.saturating_duration_since(t.shown_at))
            })
            .min()
        {
            ctx.request_repaint_after(next_expiry.min(Duration::from_millis(250)));
        }
        clicked_action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toasts_expire_after_their_duration_and_cap_visible_count() {
        let mut toasts: Toasts<u8> = Toasts::default();
        toasts.push(ToastKind::Info, "halo");
        toasts.push_with_action(ToastKind::Success, "dipindah", "Urungkan", 7);
        assert_eq!(toasts.len(), 2);

        toasts.expire(Instant::now() + INFO_DURATION + Duration::from_millis(10));
        assert_eq!(toasts.len(), 1, "info expires before the action toast");

        toasts.expire(Instant::now() + ACTION_DURATION + Duration::from_millis(10));
        assert!(toasts.is_empty());

        for i in 0..10 {
            toasts.push(ToastKind::Error, format!("e{i}"));
        }
        assert_eq!(toasts.len(), MAX_VISIBLE);
    }
}
