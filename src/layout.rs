//! Original, deterministic 12×8 geometric mosaic template.
//! Unlike an infinite repeating feed, every local photo has one finite position.
use crate::camera::Camera;

pub const PITCH: f32 = 98.0;
pub const GAP: f32 = 6.0;
pub const BLOCK_WIDTH: f32 = PITCH * 12.0;
pub const BLOCK_HEIGHT: f32 = PITCH * 8.0;
pub const TILES_PER_BLOCK: usize = 12;
pub const BLOCK_COLUMNS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width && self.x + self.width > other.x
            && self.y < other.y + other.height && self.y + self.height > other.y
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub index: usize,
    pub rect: Rect,
}

/// Fully packed layout: coordinates are (column, row, span columns, span rows).
/// Adjacent blocks alternate mirroring, creating variation without random jumping.
const TEMPLATE: [(u8, u8, u8, u8); TILES_PER_BLOCK] = [
    (0, 0, 4, 3), (4, 0, 3, 3), (7, 0, 5, 2),
    (7, 2, 2, 1), (9, 2, 3, 2), (0, 3, 2, 3),
    (2, 3, 5, 3), (7, 3, 2, 3), (9, 4, 3, 2),
    (0, 6, 3, 2), (3, 6, 4, 2), (7, 6, 5, 2),
];

pub fn tile_rect(index: usize) -> Rect {
    let block = index / TILES_PER_BLOCK;
    let column = block % BLOCK_COLUMNS;
    let row = block / BLOCK_COLUMNS;
    let (x, y, width, height) = TEMPLATE[index % TILES_PER_BLOCK];
    let x = if block % 2 == 1 { 12 - x - width } else { x };
    let y = if block % 3 == 2 { 8 - y - height } else { y };
    Rect {
        x: column as f32 * BLOCK_WIDTH + x as f32 * PITCH,
        y: row as f32 * BLOCK_HEIGHT + y as f32 * PITCH,
        width: width as f32 * PITCH - GAP,
        height: height as f32 * PITCH - GAP,
    }
}

/// Enumerates just blocks intersecting viewport + prefetch margin, never all N images.
pub fn visible_tiles(slots: &[usize], camera: Camera, width: f32, height: f32) -> Vec<Tile> {
    if slots.is_empty() || width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let pad = 320.0 / camera.zoom;
    let (left, top) = camera.screen_to_world(0.0, 0.0);
    let (right, bottom) = camera.screen_to_world(width, height);
    let min_col = (((left - pad) / BLOCK_WIDTH).floor() as isize).max(0) as usize;
    let max_col = (((right + pad) / BLOCK_WIDTH).ceil() as isize)
        .clamp(0, BLOCK_COLUMNS as isize) as usize;
    let min_row = (((top - pad) / BLOCK_HEIGHT).floor() as isize).max(0) as usize;
    // Sparse slots can extend far beyond the number of currently visible photos.
    let total_rows = (slots.last().copied().unwrap_or(0) / TILES_PER_BLOCK / BLOCK_COLUMNS) + 1;
    let max_row = (((bottom + pad) / BLOCK_HEIGHT).ceil() as isize)
        .clamp(0, total_rows as isize) as usize;
    let view = Rect {
        x: left - pad, y: top - pad,
        width: right - left + pad * 2.0,
        height: bottom - top + pad * 2.0,
    };
    let mut visible = Vec::new();
    for row in min_row..max_row {
        for col in min_col..max_col {
            let base = (row * BLOCK_COLUMNS + col) * TILES_PER_BLOCK;
            for offset in 0..TILES_PER_BLOCK {
                let slot = base + offset;
                // O(log N): even 50,000 images with thousands of missing slots
                // do not require a full image vector scan on each frame.
                let Ok(index) = slots.binary_search(&slot) else { continue };
                let rect = tile_rect(slot);
                if rect.intersects(&view) {
                    visible.push(Tile { index, rect });
                }
            }
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_exactly_fills_grid_without_overlap() {
        let mut occupied = [[false; 12]; 8];
        for &(x, y, w, h) in &TEMPLATE {
            for row in y..(y + h) {
                for col in x..(x + w) {
                    assert!(!occupied[row as usize][col as usize]);
                    occupied[row as usize][col as usize] = true;
                }
            }
        }
        assert!(occupied.iter().flatten().all(|v| *v));
    }

    #[test]
    fn each_photo_has_stable_non_overlapping_position() {
        let rects: Vec<Rect> = (0..100).map(tile_rect).collect();
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!a.intersects(b), "tiles {i} overlap");
            }
        }
    }

    #[test]
    fn deleting_one_photo_does_not_move_another_photo() {
        let original = [0, 1, 2, 3, 12, 13];
        let after_delete = [0, 2, 3, 12, 13];
        let visible_before = visible_tiles(&original, Camera::default(), 1680.0, 1000.0);
        let visible_after = visible_tiles(&after_delete, Camera::default(), 1680.0, 1000.0);
        for tile in &visible_after {
            let slot = after_delete[tile.index];
            let previous = visible_before.iter().find(|other| original[other.index] == slot).unwrap();
            assert_eq!(previous.rect, tile.rect);
        }
        assert!(visible_after.iter().all(|tile| after_delete[tile.index] != 1));
    }

    #[test]
    fn viewport_search_is_bounded_for_extremely_sparse_slots() {
        let slots: Vec<usize> = (0..50_000).map(|slot| slot * 29).collect();
        let camera = Camera { x: 0.0, y: -200_000.0, zoom: 0.8 };
        let visible = visible_tiles(&slots, camera, 1920.0, 1080.0);
        assert!(visible.len() < 220);
        assert!(visible.iter().all(|tile| slots[tile.index] < 1_450_000));
    }

    #[test]
    fn initial_five_photos_are_in_the_visible_canvas() {
        // Regression fixture for Windows v0.4.1: all five thumbnails were
        // decoded, but a collapsed GPUI canvas prevented them from painting.
        let slots = [0, 1, 2, 3, 4];
        let camera = Camera::default();
        let visible = visible_tiles(&slots, camera, 1440.0, 900.0);
        assert_eq!(visible.len(), 5);
        let first = visible.iter().find(|tile| tile.index == 0).unwrap();
        let (x, y) = camera.world_to_screen(first.rect.x, first.rect.y);
        assert!(x >= 0.0 && y >= 0.0);
        assert!(x + first.rect.width * camera.zoom > 0.0);
        assert!(y + first.rect.height * camera.zoom > 0.0);
    }

    #[test]
    fn viewport_work_is_bounded_for_huge_libraries() {
        let slots: Vec<usize> = (0..100_000).collect();
        let tiles = visible_tiles(&slots, Camera::default(), 1440.0, 900.0);
        assert!(!tiles.is_empty());
        assert!(tiles.len() < 200, "should cull invisible tiles");
    }
}
