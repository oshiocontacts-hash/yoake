pub struct Game<'a> {
    block_items: &'a [&'a str],
    tags_ready: bool,
}

impl Game<'_> {
    pub fn native_item_is_block(&self, item: &str) -> bool {
        self.block_items.contains(&item)
    }

    pub fn native_client_item_in_tag(&self, tag: &str, item: &str) -> bool {
        self.tags_ready && tag == "minecraft:mineable/pickaxe" && item == "minecraft:stone"
    }

    pub fn native_client_item_tags_ready(&self) -> bool {
        self.tags_ready
    }
}

fn midpoint(bounding_box: [[f64; 3]; 2]) -> [f64; 3] {
    std::array::from_fn(|axis| bounding_box[0][axis] * 0.5 + bounding_box[1][axis] * 0.5)
}

include!(concat!(env!("OUT_DIR"), "/host_glue.rs"));

fn decoded<T: Decode>(bytes: &[u8]) -> T {
    let mut decoder = Decoder::new(bytes);
    let value = T::decode(&mut decoder).unwrap();
    decoder.finish().unwrap();
    value
}

#[test]
fn generated_category_a_bindings_compile_and_call() {
    let block_items = ["minecraft:stone"];
    let mut game = Game {
        block_items: &block_items,
        tags_ready: true,
    };
    let path = PathHandle::game();
    let mut out = Vec::new();

    let mut args = Vec::new();
    "minecraft:mineable/pickaxe".encode(&mut args);
    "minecraft:stone".encode(&mut args);
    call_1(&mut game, &path, &args, &mut out).unwrap();
    assert!(decoded::<bool>(&out));

    out.clear();
    call_2(&mut game, &path, &[], &mut out).unwrap();
    assert!(decoded::<bool>(&out));

    out.clear();
    let mut args = Vec::new();
    "minecraft:stone".encode(&mut args);
    call_3(&mut game, &path, &args, &mut out).unwrap();
    assert!(decoded::<bool>(&out));

    out.clear();
    let mut args = Vec::new();
    [[0.0f64, 2.0, 4.0], [4.0, 8.0, 12.0]].encode(&mut args);
    call_4(&mut game, &path, &args, &mut out).unwrap();
    assert_eq!(decoded::<[f64; 3]>(&out), [2.0, 5.0, 8.0]);
}
