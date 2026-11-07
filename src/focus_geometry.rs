//! Geometry for morphing a tile into the immersive photo inspector.
//! All coordinates are relative to the wall viewport, not the native window.
use crate::{camera::Camera, layout::Rect};

pub fn focus_rect(source: Rect, camera: Camera, width: f32, height: f32, progress: f32) -> Rect {
    let (x, y) = camera.world_to_screen(source.x, source.y);
    let source = Rect {
        x,
        y,
        width: source.width * camera.zoom,
        height: source.height * camera.zoom,
    };
    let margin = 24.0;
    let max_width = (width - margin * 2.0).max(1.0);
    let max_height = (height - margin * 2.0).max(1.0);
    let target_width = max_width.min(980.0);
    let target_height = max_height.min(740.0);
    let destination = Rect {
        x: (width - target_width) * 0.5,
        y: (height - target_height) * 0.5,
        width: target_width,
        height: target_height,
    };
    let t = progress.clamp(0.0, 1.0);
    Rect {
        x: source.x + (destination.x - source.x) * t,
        y: source.y + (destination.y - source.y) * t,
        width: source.width + (destination.width - source.width) * t,
        height: source.height + (destination.height - source.height) * t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn starts_at_the_exact_tile_position() {
        let camera = Camera { x: -13.0, y: 82.0, zoom: 1.3 };
        let tile = Rect { x: 125.0, y: 96.0, width: 212.0, height: 180.0 };
        let result = focus_rect(tile, camera, 1024.0, 768.0, 0.0);
        let expected = camera.world_to_screen(tile.x, tile.y);
        assert!((result.x - expected.0).abs() < 0.001);
        assert!((result.y - expected.1).abs() < 0.001);
        assert!((result.width - tile.width * camera.zoom).abs() < 0.001);
    }
    #[test]
    fn ends_centered_and_inside_viewport() {
        let r = focus_rect(Rect { x: 50.0, y: 80.0, width: 240.0, height: 170.0 },
            Camera::default(), 1440.0, 900.0, 1.0);
        assert_eq!((r.width, r.height), (980.0, 740.0));
        assert!((r.x + r.width * 0.5 - 720.0).abs() < 0.001);
        assert!((r.y + r.height * 0.5 - 450.0).abs() < 0.001);
    }
    #[test]
    fn progress_is_clamped_and_small_windows_do_not_panic() {
        let tile = Rect { x: 20.0, y: 30.0, width: 220.0, height: 310.0 };
        let camera = Camera::default();
        assert_eq!(focus_rect(tile, camera, 400.0, 280.0, -20.0),
                   focus_rect(tile, camera, 400.0, 280.0, 0.0));
        assert_eq!(focus_rect(tile, camera, 400.0, 280.0, 9.0),
                   focus_rect(tile, camera, 400.0, 280.0, 1.0));
        let r = focus_rect(tile, camera, 70.0, 60.0, 1.0);
        assert!(r.x >= 0.0 && r.y >= 0.0);
        assert!(r.x+r.width <= 70.0 && r.y+r.height <= 60.0);
    }
}
