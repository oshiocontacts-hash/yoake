use super::*;
use std::collections::HashSet;

fn frame(width: i32, height: i32) -> (HashSet<BlockPos>, HashSet<BlockPos>) {
    let mut frame = HashSet::new();
    let mut inside = HashSet::new();
    for w in 0..width {
        for h in 0..height {
            inside.insert(BlockPos::new(-w, 64 + h, 0));
        }
        frame.insert(BlockPos::new(-w, 63, 0));
        frame.insert(BlockPos::new(-w, 64 + height, 0));
    }
    for h in 0..height {
        frame.insert(BlockPos::new(1, 64 + h, 0));
        frame.insert(BlockPos::new(-width, 64 + h, 0));
    }
    (frame, inside)
}

#[test]
fn custom_frame_block_and_size_limits_use_the_shared_scanner() {
    let (frame, inside) = frame(3, 2);
    let size = PortalSize {
        min_width: 2,
        max_width: 4,
        min_height: 2,
        max_height: 4,
    };
    let shape = scan_shape(
        -64,
        BlockPos::new(0, 64, 0),
        true,
        size,
        |pos| frame.contains(&pos),
        |pos| inside.contains(&pos),
        |_| false,
    );
    assert!(shape.is_valid());
    assert_eq!((shape.width, shape.height), (3, 2));
    assert_eq!(shape.bottom_left, BlockPos::new(0, 64, 0));
    assert_eq!(shape.portal_blocks, 0);

    let mut wrong_size = size;
    wrong_size.min_height = 3;
    let rejected = scan_shape(
        -64,
        BlockPos::new(0, 64, 0),
        true,
        wrong_size,
        |pos| frame.contains(&pos),
        |pos| inside.contains(&pos),
        |_| false,
    );
    assert!(!rejected.is_valid());
}

#[test]
fn a_complete_custom_fill_is_counted_for_neighbour_removal() {
    let (frame, inside) = frame(2, 3);
    let shape = scan_shape(
        -64,
        BlockPos::new(0, 64, 0),
        true,
        PortalSize::default(),
        |pos| frame.contains(&pos),
        |pos| inside.contains(&pos),
        |pos| inside.contains(&pos),
    );
    assert!(shape.is_complete());
    assert_eq!(shape.portal_blocks, 6);
}

#[test]
fn item_fluid_and_fire_ignition_match_only_the_configured_cause() {
    let portal = PortalType::parse(
        "example:portal",
        &serde_json::json!({
            "frame":"example:frame",
            "ignite":{"item":"example:lighter","fluid":"minecraft:lava","fire":true},
            "target":"example:moon"
        }),
    )
    .unwrap();
    assert!(ignition_matches(&portal, Some("example:lighter"), None, false));
    assert!(ignition_matches(&portal, None, Some("minecraft:lava"), false));
    assert!(ignition_matches(&portal, None, None, true));
    assert!(!ignition_matches(&portal, Some("minecraft:flint_and_steel"), None, false));
    assert!(!ignition_matches(&portal, None, Some("minecraft:water"), false));
    assert!(!ignition_matches(&portal, None, None, false));
}
