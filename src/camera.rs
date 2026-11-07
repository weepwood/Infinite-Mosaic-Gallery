//! Pure screen/world-space transforms. Screen coordinates are local to the wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { x: 18.0, y: 18.0, zoom: 0.84 }
    }
}

impl Camera {
    pub const MIN_ZOOM: f32 = 0.25;
    pub const MAX_ZOOM: f32 = 2.8;

    pub fn world_to_screen(self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.zoom, self.y + y * self.zoom)
    }

    pub fn screen_to_world(self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.x) / self.zoom, (y - self.y) / self.zoom)
    }

    pub fn translated(self, dx: f32, dy: f32) -> Self {
        Self { x: self.x + dx, y: self.y + dy, ..self }
    }

    /// Keep the image point under the pointer fixed when changing scale.
    pub fn zoom_at(self, x: f32, y: f32, factor: f32) -> Self {
        let next_zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let (world_x, world_y) = self.screen_to_world(x, y);
        Self {
            x: x - world_x * next_zoom,
            y: y - world_y * next_zoom,
            zoom: next_zoom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_anchor_is_stationary() {
        let camera = Camera::default();
        let anchor = (421.0, 173.0);
        let before = camera.screen_to_world(anchor.0, anchor.1);
        let after = camera.zoom_at(anchor.0, anchor.1, 1.7);
        let recovered = after.world_to_screen(before.0, before.1);
        assert!((recovered.0 - anchor.0).abs() < 0.001);
        assert!((recovered.1 - anchor.1).abs() < 0.001);
    }

    #[test]
    fn zoom_is_bounded() {
        let camera = Camera::default();
        assert_eq!(camera.zoom_at(0.0, 0.0, 0.0001).zoom, Camera::MIN_ZOOM);
        assert_eq!(camera.zoom_at(0.0, 0.0, 900.0).zoom, Camera::MAX_ZOOM);
    }
}
