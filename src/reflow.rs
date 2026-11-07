//! Original focus reflow: a 6×6 centerpiece is carved out of a 12×8
//! block and remaining photos are reassigned to non-overlapping edge zones.
//! This does not copy Folia's AGPL layout or reflow tables.
use crate::layout::{tile_rect, Rect, Tile, BLOCK_COLUMNS, BLOCK_HEIGHT, BLOCK_WIDTH, GAP, PITCH, TILES_PER_BLOCK};

#[derive(Clone, Copy, Debug)]
struct Region {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Region {
    fn area(self) -> f32 { self.width * self.height }
    fn center(self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

fn into_world(region: Region, block_x: f32, block_y: f32) -> Rect {
    Rect {
        x: block_x + region.x * PITCH,
        y: block_y + region.y * PITCH,
        width: region.width * PITCH - GAP,
        height: region.height * PITCH - GAP,
    }
}

fn split_region(region: Region, count: usize) -> Vec<Region> {
    if count == 0 || region.area() <= 0.0 { return Vec::new() }
    let columns = ((count as f32 * region.width / region.height).sqrt().round() as usize)
        .clamp(1, count);
    let rows = count.div_ceil(columns);
    let mut parts = Vec::with_capacity(count);
    for row in 0..rows {
        let row_count = columns.min(count - row * columns);
        let row_height = region.height / rows as f32;
        let cell_width = region.width / row_count as f32;
        for column in 0..row_count {
            parts.push(Region {
                x: region.x + column as f32 * cell_width,
                y: region.y + row as f32 * row_height,
                width: cell_width,
                height: row_height,
            });
        }
    }
    parts
}

/// Reflow just the focus photo's block. It preserves photo identity, never duplicates
/// entries and never changes the positions of tiles in other blocks.
pub fn focus_reflow(slots: &[usize], focus: usize) -> Vec<Tile> {
    let Some(&focus_slot) = slots.get(focus) else { return Vec::new() };
    let block = focus_slot / TILES_PER_BLOCK;
    let first_slot = block * TILES_PER_BLOCK;
    let start = slots.partition_point(|slot| *slot < first_slot);
    let end = slots.partition_point(|slot| *slot < first_slot + TILES_PER_BLOCK);
    let block_x = (block % BLOCK_COLUMNS) as f32 * BLOCK_WIDTH;
    let block_y = (block / BLOCK_COLUMNS) as f32 * BLOCK_HEIGHT;
    let original = tile_rect(focus_slot);
    let local_x = (original.x - block_x) / PITCH;
    let local_y = (original.y - block_y) / PITCH;
    let center_x = local_x + (original.width + GAP) / PITCH / 2.0;
    let center_y = local_y + (original.height + GAP) / PITCH / 2.0;
    // Retain the focus tile near its previous location whenever possible.
    let fx = (center_x - 3.0).round().clamp(0.0, 6.0);
    let fy = (center_y - 3.0).round().clamp(0.0, 2.0);
    let focus_zone = Region { x: fx, y: fy, width: 6.0, height: 6.0 };
    let mut zones = vec![
        Region { x: 0.0, y: 0.0, width: 12.0, height: fy },
        Region { x: 0.0, y: fy + 6.0, width: 12.0, height: 2.0 - fy },
        Region { x: 0.0, y: fy, width: fx, height: 6.0 },
        Region { x: fx + 6.0, y: fy, width: 6.0 - fx, height: 6.0 },
    ];
    zones.retain(|region| region.area() > 0.0001);
    let mut counts = vec![0usize; zones.len()];
    let neighbor_count = end - start - 1;
    // Balance target slot areas instead of forcing all tiles into one skinny strip.
    for _ in 0..neighbor_count {
        let preferred = zones.iter().enumerate()
            .max_by(|(a, left), (b, right)| {
                (left.area() / (counts[*a] + 1) as f32)
                    .total_cmp(&(right.area() / (counts[*b] + 1) as f32))
            })
            .map(|(index, _)| index)
            .expect("the block always has spare area");
        counts[preferred] += 1;
    }
    let slots: Vec<Region> = zones.into_iter().zip(counts)
        .flat_map(|(region, count)| split_region(region, count))
        .collect();

    let others: Vec<usize> = (start..end).filter(|index| *index != focus).collect();
    let mut candidates = Vec::new();
    for (original_offset, index) in others.iter().enumerate() {
        let source = tile_rect(slots[*index]);
        let source_x = (source.x - block_x + source.width / 2.0) / PITCH;
        let source_y = (source.y - block_y + source.height / 2.0) / PITCH;
        for (slot, zone) in slots.iter().enumerate() {
            let (x, y) = zone.center();
            let distance = (x - source_x).powi(2) + (y - source_y).powi(2);
            candidates.push((distance, original_offset, slot));
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut assigned_photos = vec![false; others.len()];
    let mut assigned_slots = vec![false; slots.len()];
    let mut output = vec![Tile {
        index: focus,
        rect: into_world(focus_zone, block_x, block_y),
    }];
    for (_, offset, slot) in candidates {
        if assigned_photos[offset] || assigned_slots[slot] { continue }
        assigned_photos[offset] = true;
        assigned_slots[slot] = true;
        output.push(Tile {
            index: others[offset],
            rect: into_world(slots[slot], block_x, block_y),
        });
    }
    output.sort_by_key(|tile| tile.index);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_focus_produces_disjoint_tiles_inside_its_original_block() {
        for count in 1..=66 {
            for focus in 0..count {
                let slots: Vec<usize> = (0..count).collect();
                let targets = focus_reflow(&slots, focus);
                let block = focus / TILES_PER_BLOCK;
                let block_x = (block % BLOCK_COLUMNS) as f32 * BLOCK_WIDTH;
                let block_y = (block / BLOCK_COLUMNS) as f32 * BLOCK_HEIGHT;
                let expected = (count - block * TILES_PER_BLOCK).min(TILES_PER_BLOCK);
                assert_eq!(targets.len(), expected, "count={count} focus={focus}");
                for (i, tile) in targets.iter().enumerate() {
                    let r = tile.rect;
                    assert!(r.width > 0.0 && r.height > 0.0);
                    assert!(r.x >= block_x - 0.001 && r.y >= block_y - 0.001);
                    assert!(r.x + r.width <= block_x + BLOCK_WIDTH + 0.001);
                    assert!(r.y + r.height <= block_y + BLOCK_HEIGHT + 0.001);
                    for next in targets.iter().skip(i + 1) {
                        assert!(!r.intersects(&next.rect), "focus {focus} tiles {} and {} overlap", tile.index, next.index);
                        assert_ne!(tile.index, next.index);
                    }
                }
            }
        }
    }

    #[test]
    fn reflow_only_uses_occupied_slots_in_the_same_block() {
        let slots = [0, 3, 7, 12, 15, 39];
        let items = focus_reflow(&slots, 1);
        assert_eq!(items.len(), 3);
        assert_eq!(items.iter().map(|tile| slots[tile.index]).collect::<Vec<_>>(), vec![0, 3, 7]);
        for i in 0..items.len() {
            for other in items.iter().skip(i + 1) {
                assert!(!items[i].rect.intersects(&other.rect));
            }
        }
    }

    #[test]
    fn focus_uses_exactly_six_by_six_grid_cells() {
        let slots: Vec<usize> = (0..12).collect();
        let result = focus_reflow(&slots, 5);
        let focus = result.iter().find(|tile| tile.index == 5).unwrap().rect;
        assert!((focus.width - (6.0 * PITCH - GAP)).abs() < 0.001);
        assert!((focus.height - (6.0 * PITCH - GAP)).abs() < 0.001);
    }
}
