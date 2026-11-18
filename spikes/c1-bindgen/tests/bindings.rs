use c1_bindgen::rt::codec::{Decode, Decoder, Encode};
use c1_bindgen::rt::{
    BorrowMode, BorrowTable, CallError, Handle, HandleTable, PathHandle, ResolvePath, Root,
};

#[derive(Default)]
struct Game {
    value: u32,
}

impl Game {
    fn value(&self) -> u32 {
        self.value
    }

    fn affine(&mut self, value: u32, multiplier: u32, addend: u32) -> u32 {
        self.value = value * multiplier + addend;
        self.value
    }

    fn string_is(&self, expected: &str, actual: &str) -> bool {
        expected == actual
    }
}

impl ResolvePath<Game> for Game {
    fn resolve_path<'a>(&'a mut self, path: &PathHandle) -> Result<&'a mut Game, CallError> {
        if path.root == Root::Game && path.accessors.is_empty() {
            Ok(self)
        } else {
            Err(CallError::InvalidPath("mock only has a root Game".into()))
        }
    }
}

fn call_1(
    root: &mut Game,
    path: &PathHandle,
    args: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), CallError> {
    Decoder::new(args).finish()?;
    let target: &mut Game = root.resolve_path(path)?;
    let result = target.value();
    result.encode(out);
    Ok(())
}

fn call_2(
    root: &mut Game,
    path: &PathHandle,
    args: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), CallError> {
    let mut decoder = Decoder::new(args);
    let value = <u32 as Decode>::decode(&mut decoder)?;
    let multiplier = <u32 as Decode>::decode(&mut decoder)?;
    let addend = <u32 as Decode>::decode(&mut decoder)?;
    decoder.finish()?;
    let target: &mut Game = root.resolve_path(path)?;
    let result = target.affine(value, multiplier, addend);
    result.encode(out);
    Ok(())
}

fn call_3(
    root: &mut Game,
    path: &PathHandle,
    args: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), CallError> {
    let mut decoder = Decoder::new(args);
    let expected = <String as Decode>::decode(&mut decoder)?;
    let actual = <String as Decode>::decode(&mut decoder)?;
    decoder.finish()?;
    let target: &mut Game = root.resolve_path(path)?;
    let result = target.string_is(expected.as_str(), actual.as_str());
    result.encode(out);
    Ok(())
}

#[test]
fn hand_wired_bindings_call_mock_game() {
    let mut game = Game::default();
    let path = PathHandle::game();
    let mut out = Vec::new();
    call_1(&mut game, &path, &[], &mut out).unwrap();
    assert_eq!(u32::decode(&mut Decoder::new(&out)).unwrap(), 0);

    let mut args = Vec::new();
    (3u32, 4u32, 5u32).encode(&mut args);
    out.clear();
    call_2(&mut game, &path, &args, &mut out).unwrap();
    assert_eq!(game.value, 17);
    assert_eq!(u32::decode(&mut Decoder::new(&out)).unwrap(), 17);

    let mut args = Vec::new();
    "stone".encode(&mut args);
    "stone".encode(&mut args);
    out.clear();
    call_3(&mut game, &path, &args, &mut out).unwrap();
    assert!(bool::decode(&mut Decoder::new(&out)).unwrap());
}

#[test]
fn borrow_table_rejects_prefix_overlap_and_allows_shared() {
    let entities = PathHandle::game().field("entities");
    let entity = entities.clone().entity(7);
    let mut borrows = BorrowTable::default();
    let exclusive = borrows.acquire(&entity, BorrowMode::Exclusive).unwrap();
    assert_eq!(
        borrows.acquire(&entities, BorrowMode::Exclusive),
        Err(CallError::BorrowConflict)
    );
    borrows.release(exclusive).unwrap();

    let first = borrows.acquire(&entity, BorrowMode::Shared).unwrap();
    let second = borrows.acquire(&entities, BorrowMode::Shared).unwrap();
    assert_eq!(
        borrows.acquire(&entities, BorrowMode::Exclusive),
        Err(CallError::BorrowConflict)
    );
    borrows.release(first).unwrap();
    borrows.release(second).unwrap();
    borrows.acquire(&entities, BorrowMode::Exclusive).unwrap();
}

#[test]
fn stale_handle_is_rejected_after_slot_reuse() {
    let mut handles = HandleTable::default();
    let stale = handles.insert(PathHandle::game().field("old"));
    handles.remove(stale).unwrap();
    let replacement = handles.insert(PathHandle::game().field("new"));
    assert_eq!(stale.to_bits() & 0xffff_ffff, replacement.to_bits() & 0xffff_ffff);
    assert_eq!(handles.resolve(stale), Err(CallError::StaleHandle));
    assert_eq!(handles.resolve(replacement).unwrap().accessors.len(), 1);
    assert_ne!(stale, replacement);
    let _ = Handle::from_bits(stale.to_bits());
}

#[test]
fn decoder_rejects_trailing_bytes_and_bad_tags() {
    let mut trailing = Decoder::new(&[1, 0]);
    assert!(bool::decode(&mut trailing).unwrap());
    assert_eq!(trailing.finish(), Err(CallError::TrailingBytes));
    assert_eq!(
        bool::decode(&mut Decoder::new(&[2])),
        Err(CallError::InvalidBool(2))
    );
    assert_eq!(
        Option::<u32>::decode(&mut Decoder::new(&[3])),
        Err(CallError::InvalidTag("Option", 3))
    );
}
