//! A small stand-in for an engine module: the generator reads its rustdoc JSON, writes glue, and the
//! glue is compiled back into this very file (`--cfg glue`) and exercised by `main` below.
//! The cases are the ones MX3G1's classifier had to get right.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

#[cfg(glue)]
use yoake_slice::activation::{self, FieldTable};
#[cfg(glue)]
use yoake_slice::codec::{Decode, Encode};

// ---- plain data ----

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Idle,
    Run(u32),
    Walk { speed: f32, name: String },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meters(f64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker;

#[derive(Clone, Debug, PartialEq)]
struct Locked {
    secret: u32,
    label: Option<Arc<str>>,
}

impl Mode {
    pub fn is_idle(&self) -> bool {
        matches!(self, Mode::Idle)
    }
}

// ---- a type that is not data (an opaque field from std) ----

pub struct Opaque(std::cell::Cell<u8>);

impl Opaque {
    pub fn peek(&self) -> u8 {
        self.0.get()
    }
}

/// Frozen by config: no accessors, no bound methods.
pub struct Sealed {
    pub value: u32,
}

impl Sealed {
    pub fn peek(&self) -> u32 {
        self.value
    }
}

/// A recursive data type: the decoder must stop a deep chain.
pub struct Node {
    pub value: u8,
    pub next: Option<Box<Node>>,
}

pub fn node_sum(n: Node) -> u32 {
    let mut total = 0u32;
    let mut cur = Some(Box::new(n));
    while let Some(node) = cur {
        total += u32::from(node.value);
        cur = node.next;
    }
    total
}

/// A type that carries a lifetime: never a handle.
pub struct Borrowing<'a> {
    text: &'a str,
}

impl<'a> Borrowing<'a> {
    fn len(&self) -> usize {
        self.text.len()
    }
}

// ---- a handle type with every kind of field ----

pub struct Thing {
    pub count: i32,
    pub pos: Vec3,
    name: String,
    mode: Mode,
    tags: Vec<String>,
    scores: [u16; 3],
    opt: Option<Meters>,
    ratio: f32,
    flag: bool,
    big: usize,
    locked: Locked,
    table: HashMap<String, i64>,
    cell: Opaque,
}

impl Thing {
    pub fn new(n: i32) -> Thing {
        Thing {
            count: n,
            pos: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            name: "thing".to_string(),
            mode: Mode::Idle,
            tags: vec!["a".to_string()],
            scores: [1, 2, 3],
            opt: None,
            ratio: 0.5,
            flag: false,
            big: 7,
            locked: Locked { secret: 9, label: None },
            table: HashMap::new(),
            cell: Opaque(std::cell::Cell::new(4)),
        }
    }

    pub fn bump(&mut self, by: i32) -> i32 {
        self.count += by;
        self.count
    }

    fn private_helper(&self, scale: f64) -> Vec3 {
        Vec3 { x: self.pos.x * scale, y: self.pos.y * scale, z: self.pos.z * scale }
    }

    pub fn translate(&mut self, d: Vec3) -> bool {
        self.pos.x += d.x;
        self.pos.y += d.y;
        self.pos.z += d.z;
        d.x != 0.0 || d.y != 0.0 || d.z != 0.0
    }

    pub fn name_str(&self) -> &str {
        &self.name
    }

    pub fn score_slice(&self) -> &[u16] {
        &self.scores
    }

    pub fn set_mode(&mut self, mode: Mode) -> Mode {
        std::mem::replace(&mut self.mode, mode)
    }

    pub fn sum_into(&self, acc: &mut Vec<u64>, extra: &[u32]) -> usize {
        acc.push(u64::from(self.count as u32));
        acc.extend(extra.iter().map(|e| u64::from(*e)));
        acc.len()
    }

    pub fn first_tag(&self) -> Option<String> {
        self.tags.first().cloned()
    }

    pub fn lookup(&self, key: &str) -> Result<i64, String> {
        self.table.get(key).copied().ok_or_else(|| format!("no {key}"))
    }

    pub fn into_count(self) -> i32 {
        self.count
    }

    // handle-required: touches a type that is not data
    pub fn opaque(&self) -> &Opaque {
        &self.cell
    }

    pub fn with_opaque(&self, other: &Opaque) -> bool {
        self.cell.0.get() == other.0.get()
    }

    // returns &mut: needs a child handle
    pub fn name_mut(&mut self) -> &mut String {
        &mut self.name
    }

    // excluded: generics, closures, unsafe, dyn, associated types
    pub fn apply<F: Fn(i32) -> i32>(&mut self, f: F) {
        self.count = f(self.count);
    }

    pub fn each(&self, f: impl Fn(u16)) {
        for s in self.scores {
            f(s);
        }
    }

    pub unsafe fn raw(&self) -> i32 {
        self.count
    }

    pub fn dynamic(&self, f: &dyn Fn(i32) -> bool) -> bool {
        f(self.count)
    }
}

pub trait Tick {
    fn tick(&mut self, n: u32) -> u32;
}

impl Tick for Thing {
    fn tick(&mut self, n: u32) -> u32 {
        self.big += n as usize;
        self.big as u32
    }
}

impl<T: Clone> Tick for Vec<T> {
    fn tick(&mut self, n: u32) -> u32 {
        n
    }
}

// ---- free functions ----

pub fn midpoint(bounding_box: [[f64; 3]; 2]) -> [f64; 3] {
    std::array::from_fn(|axis| bounding_box[0][axis] * 0.5 + bounding_box[1][axis] * 0.5)
}

fn private_free(m: Mode, n: Option<u8>) -> (Mode, u8) {
    (m, n.unwrap_or(1))
}

pub fn generic_free<T>(x: T) -> T {
    x
}

pub fn pass_marker(m: Marker, k: Meters) -> (Marker, Meters) {
    (m, k)
}

/// Excluded: a `dyn` hides inside a std type that has no codec; the generator must look at its arguments.
pub fn cb_cell(c: &std::cell::RefCell<Box<dyn Fn()>>) {
    let _ = c;
}

/// Handle-required: a std type with no codec and nothing wrong inside it.
pub fn plain_cell(c: &std::cell::RefCell<u8>) -> u8 {
    *c.borrow()
}

/// `Option<&T>` arguments are decoded as `Option<T>` and lent back with `as_deref` / `as_ref`.
pub fn opt_args(s: Option<&str>, m: Option<&Meters>) -> u8 {
    s.map_or(0, |s| s.len() as u8) + m.map_or(0, |m| m.0 as u8)
}

/// A map alias that only changes the hasher.
pub type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<std::collections::hash_map::DefaultHasher>>;

pub fn fast(m: FastMap<String, u8>) -> usize {
    m.len() + usize::from(m.values().copied().sum::<u8>())
}

pub fn takes_char(c: char, s: &str, big: isize) -> String {
    format!("{c}{s}{big}")
}

// ---- the Game root ----

pub struct Game<'a> {
    value: u32,
    names: &'a [&'a str],
    pub things: Vec<Thing>,
}

impl Game<'_> {
    pub fn value(&self) -> u32 {
        self.value
    }

    pub(crate) fn affine(&mut self, value: u32, multiplier: u32, addend: u32) -> u32 {
        self.value = value * multiplier + addend;
        self.value
    }

    fn string_is(&self, expected: &str, actual: &str) -> bool {
        expected == actual
    }

    fn name_count(&self) -> usize {
        self.names.len()
    }

    fn scale(&mut self, v: &mut Vec<f64>, by: f64) {
        for x in v.iter_mut() {
            *x *= by;
        }
    }

    fn thing_count(&self) -> usize {
        self.things.len()
    }
}

// ---- the program that exercises the glue ----

#[cfg(glue)]
include!("glue/glue.rs");

#[cfg(glue)]
fn main() {
    use yoake_slice::codec::{CallError, Decoder};

    let id = |path: &str| -> u32 {
        yoake_glue::BINDINGS.iter().find(|(_, p)| *p == path).unwrap_or_else(|| panic!("no binding {path}")).0
    };
    let absent = |path: &str| assert!(!yoake_glue::BINDINGS.iter().any(|(_, p)| *p == path), "{path} must not be bound");
    fn bytes(f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut v = Vec::new();
        f(&mut v);
        v
    }
    fn decode<T: Decode>(b: &[u8]) -> T {
        let mut d = Decoder::new(b);
        let v = T::decode(&mut d).unwrap();
        d.finish().unwrap();
        v
    }

    // the Game root
    let names = ["a", "b"];
    let mut game = Game { value: 3, names: &names, things: vec![Thing::new(1)] };
    let mut out = Vec::new();
    yoake_glue::call_game(&mut game, id("mini::Game::value"), &[], &mut out).unwrap();
    assert_eq!(decode::<u32>(&out), 3);
    out.clear();
    let args = bytes(|b| {
        2u32.encode(b);
        5u32.encode(b);
        1u32.encode(b);
    });
    yoake_glue::call_game(&mut game, id("mini::Game::affine"), &args, &mut out).unwrap();
    assert_eq!(decode::<u32>(&out), 11);
    out.clear();
    let args = bytes(|b| {
        "x".encode(b);
        "x".encode(b);
    });
    yoake_glue::call_game(&mut game, id("mini::Game::string_is"), &args, &mut out).unwrap();
    assert!(decode::<bool>(&out));
    out.clear();
    yoake_glue::call_game(&mut game, id("mini::Game::name_count"), &[], &mut out).unwrap();
    assert_eq!(decode::<usize>(&out), 2);
    // &mut Vec argument: written back after the call
    out.clear();
    let args = bytes(|b| {
        vec![1.0f64, 2.0].encode(b);
        3.0f64.encode(b);
    });
    yoake_glue::call_game(&mut game, id("mini::Game::scale"), &args, &mut out).unwrap();
    assert_eq!(decode::<Vec<f64>>(&out), vec![3.0, 6.0]);
    // a call with a trailing byte, a short buffer and an unknown id fail cleanly and leave `out` alone
    out.clear();
    let args = bytes(|b| {
        "x".encode(b);
        "x".encode(b);
        b.push(0);
    });
    assert_eq!(yoake_glue::call_game(&mut game, id("mini::Game::string_is"), &args, &mut out), Err(CallError::TrailingBytes));
    assert_eq!(yoake_glue::call_game(&mut game, id("mini::Game::affine"), &[1, 0], &mut out), Err(CallError::UnexpectedEnd));
    assert_eq!(yoake_glue::call_game(&mut game, 99_999, &[], &mut out), Err(CallError::UnknownBinding(99_999)));
    assert!(out.is_empty());

    // free functions and associated functions
    let args = bytes(|b| [[0.0f64, 2.0, 4.0], [4.0, 8.0, 12.0]].encode(b));
    yoake_glue::call(0, id("mini::midpoint"), &args, &mut out).unwrap();
    assert_eq!(decode::<[f64; 3]>(&out), [2.0, 5.0, 8.0]);
    out.clear();
    let args = bytes(|b| {
        'q'.encode(b);
        "-".encode(b);
        (-5isize).encode(b);
    });
    yoake_glue::call(0, id("mini::takes_char"), &args, &mut out).unwrap();
    assert_eq!(decode::<String>(&out), "q--5");
    out.clear();
    // Option<&str> and Option<&Meters>, and a map alias with another hasher
    let args = bytes(|b| {
        Some("abc".to_string()).encode(b);
        Some(2.0f64).encode(b);
    });
    yoake_glue::call(0, id("mini::opt_args"), &args, &mut out).unwrap();
    assert_eq!(decode::<u8>(&out), 5);
    out.clear();
    let args = bytes(|b| {
        2u32.encode(b);
        "a".encode(b);
        3u8.encode(b);
        "b".encode(b);
        4u8.encode(b);
    });
    yoake_glue::call(0, id("mini::fast"), &args, &mut out).unwrap();
    assert_eq!(decode::<usize>(&out), 9);
    out.clear();
    // a recursive type: ten levels decode, a thousand are refused (depth cap), not overflowed
    let chain = |n: usize| bytes(|b| {
        for i in 0..n {
            (i as u8).encode(b);
            1u8.encode(b);
        }
        7u8.encode(b);
        0u8.encode(b);
    });
    yoake_glue::call(0, id("mini::node_sum"), &chain(10), &mut out).unwrap();
    assert_eq!(decode::<u32>(&out), (0..10u32).sum::<u32>() + 7);
    out.clear();
    assert_eq!(yoake_glue::call(0, id("mini::node_sum"), &chain(1000), &mut out), Err(CallError::TooDeep));
    assert!(out.is_empty());
    // a huge claimed vector length is refused without reserving it
    let args = bytes(|b| {
        0x3b9a_ca00u32.encode(b);
    });
    assert_eq!(yoake_glue::call(0, id("mini::fast"), &args, &mut out), Err(CallError::UnexpectedEnd));
    // a unit struct (no bytes) and a tuple struct (one f64)
    let args = bytes(|b| 1.25f64.encode(b));
    yoake_glue::call(0, id("mini::pass_marker"), &args, &mut out).unwrap();
    assert_eq!(out, args);
    out.clear();
    // an enum variant with named fields, an Option, a tuple result
    let args = bytes(|b| {
        2u32.encode(b);
        1.5f32.encode(b);
        "fast".encode(b);
        Some(8u8).encode(b);
    });
    yoake_glue::call(0, id("mini::private_free"), &args, &mut out).unwrap();
    let mut d = Decoder::new(&out);
    assert_eq!(<u32 as Decode>::decode(&mut d).unwrap(), 2);
    assert_eq!(<f32 as Decode>::decode(&mut d).unwrap(), 1.5);
    assert_eq!(<String as Decode>::decode(&mut d).unwrap(), "fast");
    assert_eq!(<u8 as Decode>::decode(&mut d).unwrap(), 8);
    d.finish().unwrap();
    out.clear();
    // an enum tag out of range is an error
    let args = bytes(|b| {
        9u32.encode(b);
        None::<u8>.encode(b);
    });
    assert!(matches!(yoake_glue::call(0, id("mini::private_free"), &args, &mut out), Err(CallError::InvalidTag(..))));
    assert!(out.is_empty());

    // what must not be bound, and why (the classifier decisions)
    for path in [
        "mini::Thing::new",         // returns a non-data value (Thing holds a Cell)
        "mini::Thing::opaque",      // returns a reference to a non-data type
        "mini::Thing::with_opaque", // takes a non-data reference
        "mini::Thing::name_mut",    // returns &mut: child handle
        "mini::Thing::apply",       // generic
        "mini::Thing::each",        // closure argument
        "mini::Thing::raw",         // unsafe
        "mini::Thing::dynamic",     // dyn
        "mini::Thing::into_count",  // by-value self of a non-data type
        "mini::generic_free",       // generic
        "mini::cb_cell",            // a dyn inside a std type
        "mini::plain_cell",         // a std type with no codec
        "mini::Borrowing::len",     // the receiver type carries a lifetime
        "mini::Thing.cell#get",     // a field of a non-data type
        "mini::Sealed::peek",       // frozen type
        "mini::Sealed.value#get",   // frozen type: no accessors at all
    ] {
        absent(path);
    }
    // a method of a handle type that is not data is bound (Opaque has no data fields at all)
    id("mini::Opaque::peek");
    // impl-level generics (`impl<T: Clone> Tick for Vec<T>`) are not bound
    assert!(!yoake_glue::BINDINGS.iter().any(|(_, p)| p.contains("Vec<")));

    // handle receivers: a method, a trait method, field accessors, wrong-type and stale handles
    let mut thing = Thing::new(10);
    let mut id_of_handle = 0;
    activation::scope(&mut thing, &yoake_glue::FIELDS_crate__Thing, &mut |_| 0, |h| {
        id_of_handle = h;
        let mut out = Vec::new();
        yoake_glue::call(h, id("mini::Thing::bump"), &bytes(|b| 5i32.encode(b)), &mut out).unwrap();
        assert_eq!(decode::<i32>(&out), 15);
        out.clear();
        // the trait method
        yoake_glue::call(h, id("mini::<Thing as Tick>::tick"), &bytes(|b| 3u32.encode(b)), &mut out).unwrap();
        assert_eq!(decode::<u32>(&out), 10);
        out.clear();
        // Vec3 is plain data: a struct argument and a struct result
        let pos = Vec3 { x: 1.0, y: 2.0, z: 3.0 };
        let args = bytes(|b| {
            pos.x.encode(b);
            pos.y.encode(b);
            pos.z.encode(b);
        });
        yoake_glue::call(h, id("mini::Thing.pos#set"), &args, &mut out).unwrap();
        yoake_glue::call(h, id("mini::Thing::private_helper"), &bytes(|b| 2.0f64.encode(b)), &mut out).unwrap();
        assert_eq!(decode::<(f64, f64, f64)>(&out), (2.0, 4.0, 6.0));
        out.clear();
        yoake_glue::call(h, id("mini::Thing::translate"), &args, &mut out).unwrap();
        assert!(decode::<bool>(&out));
        out.clear();
        // &str and &[u16] results are copied out
        yoake_glue::call(h, id("mini::Thing::name_str"), &[], &mut out).unwrap();
        assert_eq!(decode::<String>(&out), "thing");
        out.clear();
        yoake_glue::call(h, id("mini::Thing::score_slice"), &[], &mut out).unwrap();
        assert_eq!(decode::<Vec<u16>>(&out), vec![1, 2, 3]);
        out.clear();
        // an enum argument and result
        let args = bytes(|b| {
            2u32.encode(b);
            1.5f32.encode(b);
            "w".encode(b);
        });
        yoake_glue::call(h, id("mini::Thing::set_mode"), &args, &mut out).unwrap();
        assert_eq!(out, 0u32.to_le_bytes());
        out.clear();
        // &mut Vec and &[u32] arguments, usize result, then the written-back Vec
        let args = bytes(|b| {
            vec![1u64].encode(b);
            vec![7u32, 8].encode(b);
        });
        yoake_glue::call(h, id("mini::Thing::sum_into"), &args, &mut out).unwrap();
        let mut d = Decoder::new(&out);
        assert_eq!(<usize as Decode>::decode(&mut d).unwrap(), 4);
        assert_eq!(<Vec<u64> as Decode>::decode(&mut d).unwrap(), vec![1, 15, 7, 8]);
        d.finish().unwrap();
        out.clear();
        // Option<String> and Result<i64, String>
        yoake_glue::call(h, id("mini::Thing::first_tag"), &[], &mut out).unwrap();
        assert_eq!(decode::<Option<String>>(&out), Some("a".to_string()));
        out.clear();
        yoake_glue::call(h, id("mini::Thing::lookup"), &bytes(|b| "k".encode(b)), &mut out).unwrap();
        assert_eq!(decode::<Result<i64, String>>(&out), Err("no k".to_string()));
        out.clear();
        // getters and setters of private fields of every shape
        yoake_glue::call(h, id("mini::Thing.name#set"), &bytes(|b| "renamed".encode(b)), &mut out).unwrap();
        yoake_glue::call(h, id("mini::Thing.name#get"), &[], &mut out).unwrap();
        assert_eq!(decode::<String>(&out), "renamed");
        out.clear();
        yoake_glue::call(h, id("mini::Thing.scores#set"), &bytes(|b| [9u16, 8, 7].encode(b)), &mut out).unwrap();
        yoake_glue::call(h, id("mini::Thing.scores#get"), &[], &mut out).unwrap();
        assert_eq!(decode::<[u16; 3]>(&out), [9, 8, 7]);
        out.clear();
        // a private struct with an Arc<str> field
        let locked_arg = bytes(|b| {
            77u32.encode(b);
            Some("lbl".to_string()).encode(b);
        });
        yoake_glue::call(h, id("mini::Thing.locked#set"), &locked_arg, &mut out).unwrap();
        yoake_glue::call(h, id("mini::Thing.locked#get"), &[], &mut out).unwrap();
        assert_eq!(out, locked_arg);
        out.clear();
        // a tuple struct with a private field inside an Option
        yoake_glue::call(h, id("mini::Thing.opt#set"), &bytes(|b| Some(2.5f64).encode(b)), &mut out).unwrap();
        yoake_glue::call(h, id("mini::Thing.opt#get"), &[], &mut out).unwrap();
        assert_eq!(decode::<Option<f64>>(&out), Some(2.5));
        out.clear();
        yoake_glue::call(h, id("mini::Thing.big#set"), &bytes(|b| 40u64.encode(b)), &mut out).unwrap();
        // a map
        let table = bytes(|b| {
            1u32.encode(b);
            "k".encode(b);
            5i64.encode(b);
        });
        yoake_glue::call(h, id("mini::Thing.table#set"), &table, &mut out).unwrap();
        yoake_glue::call(h, id("mini::Thing::lookup"), &bytes(|b| "k".encode(b)), &mut out).unwrap();
        assert_eq!(decode::<Result<i64, String>>(&out), Ok(5));
        out.clear();
        // frozen field: the setter id is reserved and refused, the FieldTable refuses it too
        let count_set = id("mini::Thing.count#set");
        assert_eq!(yoake_glue::call(h, count_set, &bytes(|b| 1i32.encode(b)), &mut out), Err(CallError::ReadOnly(count_set)));
        assert_eq!(activation::set(h, 0, 1), Err(yoake_slice::activation::Status::BadField));
        let tags_set = id("mini::Thing.tags#set");
        assert_eq!(yoake_glue::call(h, tags_set, &bytes(|b| Vec::<String>::new().encode(b)), &mut out), Err(CallError::ReadOnly(tags_set)));
        assert_eq!(decode::<i32>(&{
            yoake_glue::call(h, id("mini::Thing.count#get"), &[], &mut out).unwrap();
            let v = out.clone();
            out.clear();
            v
        }), 15, "a read-only field can still be read");
        // the scalar FieldTable of the same fields
        assert_eq!(activation::get(h, 0), Ok(15));
        assert_eq!(activation::get(h, 9), Ok(40));
        activation::set(h, 8, 1).unwrap();
        activation::set(h, 7, i64::from(0.25f32.to_bits())).unwrap();
        // a binding of another handle type through this handle is refused, nothing runs
        assert_eq!(
            yoake_glue::call(h, id("mini::Mode::is_idle"), &[], &mut out),
            Err(CallError::Handle(yoake_slice::activation::Status::WrongType))
        );
        // a forged argument index
        assert_eq!(
            yoake_glue::call(h | 3, id("mini::Thing::bump"), &bytes(|b| 1i32.encode(b)), &mut out),
            Err(CallError::Handle(yoake_slice::activation::Status::NoSuchArg))
        );
        assert!(out.is_empty());
    });
    assert_eq!(thing.count, 15);
    assert_eq!(thing.name, "renamed");
    assert_eq!(thing.scores, [9, 8, 7]);
    assert_eq!(thing.big, 40);
    assert!(thing.flag);
    assert_eq!(thing.ratio, 0.25);
    assert_eq!(thing.pos, Vec3 { x: 2.0, y: 4.0, z: 6.0 });
    assert_eq!(thing.opt, Some(Meters(2.5)));
    assert_eq!(thing.locked, Locked { secret: 77, label: Some(Arc::from("lbl")) });
    // after the scope the handle is stale
    let mut out = Vec::new();
    assert_eq!(
        yoake_glue::call(id_of_handle, id("mini::Thing::bump"), &bytes(|b| 1i32.encode(b)), &mut out),
        Err(CallError::Handle(yoake_slice::activation::Status::Stale))
    );
    println!("mini_engine: all glue checks passed");
}
