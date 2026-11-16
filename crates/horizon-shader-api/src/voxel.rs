//! Packed r32uint voxels and camera-centred toroidal section bookkeeping.

use std::collections::BTreeSet;

/// The size of one tracked chunk section along each axis, in blocks.
pub const SECTION_SIDE: i32 = 16;
/// The largest allowed side of a voxel window.
pub const MAX_SIDE: u32 = 256;
/// The largest allowed number of voxels in a voxel window.
pub const MAX_VOXELS: u64 = 256 * 128 * 256;
pub const VALIDITY_VECTORS: usize = 16;

/// Approximate geometry encoded in bits 0–3 of a voxel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Shape {
    Empty = 0,
    Full = 1,
    SlabBottom = 2,
    SlabTop = 3,
    StairsNorth = 4,
    StairsEast = 5,
    StairsSouth = 6,
    StairsWest = 7,
    Thin = 8,
    Pane = 9,
    StairsTopNorth = 10,
    StairsTopEast = 11,
    StairsTopSouth = 12,
    StairsTopWest = 13,
}

/// A voxel's packed representation. Tint is quantized from 8-bit to 6-bit RGB;
/// emission is clamped to the vanilla light-level range, 0–15.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Voxel {
    pub shape: Shape,
    pub translucent: bool,
    pub tint: [u8; 3],
    pub emission: u8,
}

impl Voxel {
    /// One `r32uint` texel: shape, translucency, emission and six-bit RGB.
    pub fn encode(self) -> u32 {
        if self.shape == Shape::Empty {
            return 0;
        }
        let quantize = |component: u8| (u32::from(component) * 63 + 127) / 255;
        self.shape as u32
            | (u32::from(u8::from(self.translucent)) << 4)
            | (u32::from(self.emission.min(15)) << 5)
            | (quantize(self.tint[0]) << 9)
            | (quantize(self.tint[1]) << 15)
            | (quantize(self.tint[2]) << 21)
    }
}

/// A camera-centred window of world sections and the sections needing upload.
/// World section coordinates map to stable toroidal slots as the window moves.
#[derive(Debug)]
pub struct Window {
    origin: [i32; 3],
    extent: [i32; 3],
    dirty: BTreeSet<[i32; 3]>,
}

impl Window {
    /// Start with every section dirty. Sides must be positive multiples of 16
    /// within the per-side and total voxel caps.
    pub fn new(size: [u32; 3], camera_block: [i32; 3]) -> Result<Self, &'static str> {
        if size
            .iter()
            .any(|side| *side == 0 || *side > MAX_SIDE || *side % SECTION_SIDE as u32 != 0)
            || size.iter().map(|side| u64::from(*side)).product::<u64>() > MAX_VOXELS
        {
            return Err("voxel dimensions must be multiples of 16 within the size caps");
        }
        let extent = size.map(|side| side as i32 / SECTION_SIDE);
        let origin = Self::camera_origin(camera_block, extent);
        let mut window = Self {
            origin,
            extent,
            dirty: BTreeSet::new(),
        };
        window.dirty.extend(Self::sections(origin, extent));
        Ok(window)
    }

    /// Move the window; retained sections keep their slots and only newly
    /// exposed sections are added to the pending uploads.
    pub fn move_camera(&mut self, camera_block: [i32; 3]) {
        let extent = self.extent;
        let next = Self::camera_origin(camera_block, extent);
        if next == self.origin {
            return;
        }
        let previous = self.origin;
        self.origin = next;
        self.dirty
            .retain(|section| Self::contains(next, extent, *section));
        self.dirty.extend(
            Self::sections(next, extent)
                .filter(|section| !Self::contains(previous, extent, *section)),
        );
    }

    /// Mark a changed world section if it lies inside the current window.
    pub fn mark_dirty(&mut self, section: [i32; 3]) {
        if Self::contains(self.origin, self.extent, section) {
            self.dirty.insert(section);
        }
    }

    /// Invalidate all cells when the loaded world changes.
    pub fn invalidate(&mut self) {
        self.dirty.extend(Self::sections(self.origin, self.extent));
    }

    /// Drain the pending world section coordinates in deterministic order.
    pub fn take_dirty(&mut self) -> Vec<[i32; 3]> {
        std::mem::take(&mut self.dirty).into_iter().collect()
    }

    /// Take at most `budget` sections, nearest the camera's section first.
    pub fn take_dirty_nearest(&mut self, camera_block: [i32; 3], budget: usize) -> Vec<[i32; 3]> {
        let camera = camera_block.map(|coordinate| coordinate.div_euclid(SECTION_SIDE));
        let mut pending: Vec<_> = self.dirty.iter().copied().collect();
        pending.sort_unstable_by_key(|section| {
            let distance: i64 = (0..3)
                .map(|axis| (i64::from(section[axis]) - i64::from(camera[axis])).pow(2))
                .sum();
            (distance, *section)
        });
        pending.truncate(budget);
        for section in &pending {
            self.dirty.remove(section);
        }
        pending
    }

    /// Physical texture slot of the minimum block, without floating-point world coordinates.
    pub fn origin_slot(&self) -> [u32; 3] {
        std::array::from_fn(|axis| {
            self.origin[axis].rem_euclid(self.extent[axis]) as u32 * SECTION_SIDE as u32
        })
    }

    /// One bit per physical section slot (x + width * (y + height * z)).
    /// Pending sections are invalid, including slots evicted by camera movement.
    pub fn validity(&self) -> [[u32; 4]; VALIDITY_VECTORS] {
        let mut validity = [[0; 4]; VALIDITY_VECTORS];
        for section in Self::sections(self.origin, self.extent) {
            if self.dirty.contains(&section) {
                continue;
            }
            let slot = self.section_slot(section).unwrap();
            let index =
                slot[0] + self.extent[0] as u32 * (slot[1] + self.extent[1] as u32 * slot[2]);
            let word = index as usize / 32;
            validity[word / 4][word % 4] |= 1 << (index % 32);
        }
        validity
    }

    /// Minimum world block included in the window.
    pub fn origin(&self) -> [i32; 3] {
        self.origin.map(|coordinate| coordinate * SECTION_SIDE)
    }

    /// Return the physical section slot, or `None` outside the window.
    pub fn section_slot(&self, section: [i32; 3]) -> Option<[u32; 3]> {
        Self::contains(self.origin, self.extent, section)
            .then(|| std::array::from_fn(|axis| section[axis].rem_euclid(self.extent[axis]) as u32))
    }

    fn camera_origin(camera_block: [i32; 3], extent: [i32; 3]) -> [i32; 3] {
        std::array::from_fn(|axis| camera_block[axis].div_euclid(SECTION_SIDE) - extent[axis] / 2)
    }

    fn contains(origin: [i32; 3], extent: [i32; 3], section: [i32; 3]) -> bool {
        (0..3).all(|axis| {
            section[axis] >= origin[axis] && section[axis] < origin[axis] + extent[axis]
        })
    }

    fn sections(origin: [i32; 3], extent: [i32; 3]) -> impl Iterator<Item = [i32; 3]> {
        (origin[0]..origin[0] + extent[0]).flat_map(move |x| {
            (origin[1]..origin[1] + extent[1])
                .flat_map(move |y| (origin[2]..origin[2] + extent[2]).map(move |z| [x, y, z]))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Shape, Voxel, Window};

    #[test]
    fn uploads_are_bounded_nearest_first_and_evicted_slots_are_invalid() {
        let mut window = Window::new([32; 3], [30_000_001, 65, -30_000_001]).unwrap();
        assert_eq!(window.validity(), [[0; 4]; 16]);
        let first = window.take_dirty_nearest([30_000_001, 65, -30_000_001], 1);
        assert_eq!(first, [[1_875_000, 4, -1_875_001]]);
        assert_eq!(window.validity()[0][0].count_ones(), 1);
        assert_eq!(window.take_dirty_nearest([30_000_001, 65, -30_000_001], 3).len(), 3);
        assert_eq!(window.take_dirty().len(), 4);
        assert_eq!(window.validity()[0][0], 255);
        assert!(window.take_dirty_nearest([30_000_001, 65, -30_000_001], 8).is_empty());
        window.move_camera([30_000_017, 65, -30_000_001]);
        assert_eq!(window.validity()[0][0].count_ones(), 4);
        assert_eq!(window.origin_slot(), [0, 16, 0]);
        assert_eq!(window.take_dirty_nearest([30_000_017, 65, -30_000_001], 2).len(), 2);
        assert_eq!(window.validity()[0][0].count_ones(), 6);
        assert_eq!(window.take_dirty().len(), 2);
        window.invalidate();
        assert_eq!(window.validity(), [[0; 4]; 16]);
    }

    #[test]
    fn integer_slots_preserve_adjacent_blocks_far_from_origin() {
        for camera in [[30_000_001, 65, -30_000_001], [-30_000_001, -65, 30_000_001]] {
            let mut window = Window::new([128; 3], camera).unwrap();
            window.take_dirty();
            let origin = window.origin();
            let origin_slot = window.origin_slot();
            for position in [camera, camera.map(|coordinate| coordinate + 1)] {
                let addressed: [u32; 3] = std::array::from_fn(|axis| {
                    ((position[axis] - origin[axis]) as u32 + origin_slot[axis]) % 128
                });
                assert_eq!(addressed, position.map(|coordinate| coordinate.rem_euclid(128) as u32));
            }
        }
    }

    #[test]
    fn encodes_shapes_tint_translucency_and_emission() {
        let full = Voxel {
            shape: Shape::Full,
            translucent: false,
            tint: [255; 3],
            emission: 15,
        };
        assert_eq!(full.encode(), 0x07ff_ffe1);
        assert_eq!(
            Voxel {
                shape: Shape::StairsNorth,
                translucent: true,
                tint: [255, 0, 0],
                emission: 5,
            }
            .encode(),
            0x0000_7eb4
        );
        assert_eq!(
            Voxel {
                emission: 255,
                ..full
            }
            .encode(),
            full.encode()
        );
        assert_eq!(
            Voxel {
                shape: Shape::Empty,
                ..full
            }
            .encode(),
            0
        );
    }

    #[test]
    fn retains_slots_and_only_updates_entering_or_changed_sections() {
        let mut window = Window::new([32, 32, 32], [0, 0, 0]).unwrap();
        assert_eq!(window.take_dirty().len(), 8);
        let slot = window.section_slot([0, 0, 0]).unwrap();
        window.move_camera([15, 0, 0]);
        assert!(window.take_dirty().is_empty());
        window.move_camera([16, 0, 0]);
        assert_eq!(window.section_slot([0, 0, 0]), Some(slot));
        assert_eq!(window.section_slot([-1, 0, 0]), None);
        let entering = window.take_dirty();
        assert_eq!(entering.len(), 4);
        assert!(entering.iter().all(|section| section[0] == 1));
        window.mark_dirty([0, 0, 0]);
        window.mark_dirty([0, 0, 0]);
        window.mark_dirty([-1, 0, 0]);
        assert_eq!(window.take_dirty(), vec![[0, 0, 0]]);
    }

    #[test]
    fn handles_negative_coordinates_and_large_camera_jumps() {
        let mut window = Window::new([32, 32, 32], [0, 0, 0]).unwrap();
        assert_eq!(window.section_slot([-1, -1, -1]), Some([1, 1, 1]));
        window.move_camera([-1, 0, 0]);
        assert_eq!(window.take_dirty().len(), 8);
        window.move_camera([160, 0, 0]);
        assert_eq!(window.take_dirty().len(), 8);
    }

    #[test]
    fn reuses_evicted_slots_and_retains_pending_dirty_sections() {
        let mut window = Window::new([32, 32, 32], [0, 0, 0]).unwrap();
        window.take_dirty();
        let evicted_slot = window.section_slot([-1, 0, 0]).unwrap();
        window.mark_dirty([0, 0, 0]);
        window.mark_dirty([-1, -1, 0]);

        window.move_camera([16, 0, 0]);

        assert_eq!(window.section_slot([1, 0, 0]), Some(evicted_slot));
        let pending = window.take_dirty();
        assert_eq!(pending.len(), 5);
        assert!(pending.contains(&[0, 0, 0]));
        assert!(!pending.contains(&[-1, -1, 0]));
        assert_eq!(pending.iter().filter(|section| section[0] == 1).count(), 4);
    }

    #[test]
    fn rejects_invalid_sizes() {
        assert!(Window::new([0, 16, 16], [0; 3]).is_err());
        assert!(Window::new([17, 16, 16], [0; 3]).is_err());
        assert!(Window::new([272, 16, 16], [0; 3]).is_err());
        assert!(Window::new([256, 256, 256], [0; 3]).is_err());
        assert!(Window::new([256, 128, 256], [0; 3]).is_ok());
    }
}
