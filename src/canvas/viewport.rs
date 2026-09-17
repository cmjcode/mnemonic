//! 2D infinite viewport with coordinate transforms (Screen <-> World) and zoom/pan.

use egui::{Color32, Painter, Pos2, Rect, Vec2};

/// 2D Infinite Viewport transformation state.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Viewport {
    /// World-space position currently centered at screen center [x, y].
    pub pan: [f32; 2],
    /// Zoom scale factor, where 1.0 is 100%. Clamped between 0.15 and 4.0.
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            pan: [0.0, 0.0],
            zoom: 1.0,
        }
    }
}

impl Viewport {
    pub const MIN_ZOOM: f32 = 0.15;
    pub const MAX_ZOOM: f32 = 4.0;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn pan_vec(&self) -> Vec2 {
        Vec2::new(self.pan[0], self.pan[1])
    }

    pub fn set_pan_vec(&mut self, v: Vec2) {
        self.pan = [v.x, v.y];
    }

    pub fn add_pan_vec(&mut self, delta: Vec2) {
        self.pan[0] += delta.x;
        self.pan[1] += delta.y;
    }

    /// Zoom centered at a specific screen position (e.g. mouse cursor).
    pub fn zoom_at(&mut self, factor: f32, screen_center: Pos2, screen_origin: Pos2) {
        let old_zoom = self.zoom;
        let new_zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        if (new_zoom - old_zoom).abs() < 1e-5 {
            return;
        }

        let world_at_mouse = self.screen_to_world(screen_center, screen_origin);
        self.zoom = new_zoom;
        let new_screen_at_mouse = self.world_to_screen(world_at_mouse, screen_origin);
        let screen_delta = screen_center - new_screen_at_mouse;
        self.add_pan_vec(screen_delta / self.zoom);
    }

    /// Zoom and pan so `world_rect` fills a screen area of `screen_size`, with a margin.
    pub fn fit_rect(&mut self, world_rect: Rect, screen_size: Vec2) {
        if !world_rect.is_positive() || screen_size.x <= 0.0 || screen_size.y <= 0.0 {
            return;
        }
        const MARGIN: f32 = 0.9;
        let zoom = (screen_size.x / world_rect.width())
            .min(screen_size.y / world_rect.height())
            * MARGIN;
        self.zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let center = world_rect.center();
        self.pan = [
            screen_size.x / (2.0 * self.zoom) - center.x,
            screen_size.y / (2.0 * self.zoom) - center.y,
        ];
    }

    /// Convert screen coordinates (pixels on window) to world coordinates.
    pub fn screen_to_world(&self, screen_pos: Pos2, screen_origin: Pos2) -> Pos2 {
        let relative_screen = screen_pos - screen_origin;
        Pos2::new(
            (relative_screen.x / self.zoom) - self.pan[0],
            (relative_screen.y / self.zoom) - self.pan[1],
        )
    }

    /// Convert world coordinates to screen coordinates.
    pub fn world_to_screen(&self, world_pos: Pos2, screen_origin: Pos2) -> Pos2 {
        Pos2::new(
            screen_origin.x + (world_pos.x + self.pan[0]) * self.zoom,
            screen_origin.y + (world_pos.y + self.pan[1]) * self.zoom,
        )
    }

    /// Convert a world-space bounding rect to screen space.
    pub fn world_rect_to_screen(&self, world_rect: Rect, screen_origin: Pos2) -> Rect {
        let min = self.world_to_screen(world_rect.min, screen_origin);
        let max = self.world_to_screen(world_rect.max, screen_origin);
        Rect::from_min_max(min, max)
    }

    /// Convert a screen-space bounding rect to world space.
    pub fn screen_rect_to_world(&self, screen_rect: Rect, screen_origin: Pos2) -> Rect {
        let min = self.screen_to_world(screen_rect.min, screen_origin);
        let max = self.screen_to_world(screen_rect.max, screen_origin);
        Rect::from_min_max(min, max)
    }

    /// Render infinite background dot matrix or subtle grid.
    pub fn draw_grid(&self, painter: &Painter, screen_rect: Rect, is_dark: bool) {
        let dot_color = if is_dark {
            Color32::from_white_alpha(35)
        } else {
            Color32::from_black_alpha(25)
        };

        // Grid cell step in world coordinates, doubled while zoomed out so
        // dots stay at least MIN_SCREEN_STEP apart — otherwise a zoomed-out
        // view paints tens of thousands of dots every frame.
        const MIN_SCREEN_STEP: f32 = 16.0;
        let mut step_world = 32.0;
        while step_world * self.zoom < MIN_SCREEN_STEP {
            step_world *= 2.0;
        }

        let origin = screen_rect.min;
        let top_left_world = self.screen_to_world(screen_rect.min, origin);
        let bottom_right_world = self.screen_to_world(screen_rect.max, origin);

        let start_x = (top_left_world.x / step_world).floor() * step_world;
        let start_y = (top_left_world.y / step_world).floor() * step_world;

        let mut x = start_x;
        while x <= bottom_right_world.x {
            let mut y = start_y;
            while y <= bottom_right_world.y {
                let screen_pt = self.world_to_screen(Pos2::new(x, y), origin);
                if screen_rect.contains(screen_pt) {
                    painter.circle_filled(screen_pt, 1.2 * self.zoom.clamp(0.6, 1.4), dot_color);
                }
                y += step_world;
            }
            x += step_world;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_to_world_round_trip() {
        let mut vp = Viewport::new();
        vp.pan = [120.0, -80.0];
        vp.zoom = 1.5;

        let origin = Pos2::new(50.0, 50.0);
        let original_world = Pos2::new(300.0, 450.0);

        let screen = vp.world_to_screen(original_world, origin);
        let restored_world = vp.screen_to_world(screen, origin);

        assert!((original_world.x - restored_world.x).abs() < 1e-4);
        assert!((original_world.y - restored_world.y).abs() < 1e-4);
    }

    #[test]
    fn zoom_clamps_within_bounds() {
        let mut vp = Viewport::new();
        let origin = Pos2::ZERO;
        let center = Pos2::new(100.0, 100.0);

        vp.zoom_at(0.01, center, origin);
        assert!(vp.zoom >= Viewport::MIN_ZOOM);

        vp.zoom_at(100.0, center, origin);
        assert!(vp.zoom <= Viewport::MAX_ZOOM);
    }
}
