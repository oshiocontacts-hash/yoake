//! `horizon:mob_sync`: what a fully custom mob needs on a Horizon client beyond `AddEntity` (F4M-5b,
//! FR-F4-MOBS.md sections 3.5 and 3.9). Clientbound only; a stock client is sent nothing, and so is a
//! Horizon client when no custom type is loaded.
//!
//! A payload is one **frame**: a header that carries the server game time, then messages. Every
//! message is absolute (values, anchors and triggers say what is, not what changed since the last one),
//! so a frame that is applied twice or out of order leaves the same state as the in-order one.
//!
//! ```text
//! frame    := version u8 (=1)  game_time i64  count varint  message*count
//! message  := kind u8  body_len varint  body           (an unknown kind is skipped by its length)
//! kind 1 snapshot := entity varint  type_version u32  values  anchors  triggers  hitbox_mask u32
//! kind 2 delta    := entity varint  flags u8 (1 values, 2 anchors, 4 hitbox)
//!                    [values] [anchors] [hitbox_mask u32]       (each only when its flag is set)
//! kind 3 trigger  := entity varint  trigger_id u16  start_tick i64  speed u8
//! kind 4 seats    := entity varint  count u8 (<= 16)  (rider varint  seat u8)*count   (F4M-4d: which seat each
//!                    mob rider of the vehicle `entity` holds; absolute, an empty list clears)
//! values   := mask u32  value*popcount(mask)        (ascending slot order, at most 256 payload bytes, tags not counted)
//! value    := tag u8 + payload: 0 bool u8 | 1 i32 | 2 f32 | 3 string (len u8 <= 64, UTF-8) | 4 vec3 3*f32 | 5 enum u8
//! anchors  := count u8  (layer u8  state u8  tick i64  n u8  (time f32  speed f32)*n)*count
//! triggers := count u8  (trigger_id u16  start_tick i64  speed u8)*count
//! ```
//!
//! Numbers are big endian. `speed u8` of a trigger is the speed times 16 (16 is real time).
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};

/// The custom payload channel, clientbound only.
pub const CHANNEL: &str = "horizon:mob_sync";
/// The only frame version.
pub const VERSION: u8 = 1;
/// Bytes of tracked values in one message (the `values` list, mask excluded).
pub const MAX_VALUE_BYTES: usize = 256;
/// Bytes of a string value.
pub const MAX_STRING: usize = 64;
/// Tracked values of a type (one bit each in a mask).
pub const MAX_VALUES: usize = 32;
/// Controller layers of a type.
pub const MAX_LAYERS: usize = 16;
/// Animation slots of one controller state.
pub const MAX_SLOTS: usize = 8;
/// Recent triggers a mob keeps for a late joiner.
pub const TRIGGER_RING: usize = 4;
/// Messages in one frame at most.
pub const MAX_MESSAGES: usize = 4096;
/// Bytes of one frame the server builds at most (several frames go out when a tick has more).
pub const MAX_FRAME: usize = 16 * 1024;
/// Deltas per second a mob sends when none of its values asks for another `sync_rate`.
pub const DEFAULT_SYNC_RATE: u32 = 10;
/// Game ticks per second.
pub const TICKS_PER_SECOND: u32 = 20;
/// Seat claims in one `Seats` message at most.
pub const MAX_SEATS: usize = 16;

/// The trigger speed byte for a speed (real time is 16).
pub fn speed_byte(speed: f32) -> u8 {
    if speed.is_finite() {
        (speed * 16.0).round().clamp(0.0, 255.0) as u8
    } else {
        16
    }
}
/// The speed a trigger speed byte means.
pub fn speed_from_byte(b: u8) -> f32 {
    f32::from(b) / 16.0
}

/// One tracked value on the wire.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    I32(i32),
    F32(f32),
    Str(String),
    Vec3([f32; 3]),
    Enum(u8),
}

impl Value {
    /// Bytes of this value's payload on the wire (the tag byte is not counted: the 256 byte budget
    /// of a mob is the schema's, which counts a bool as 1, an i32 as 4, a vec3 as 12, a string as 65).
    pub fn wire_len(&self) -> usize {
        match self {
            Value::Bool(_) | Value::Enum(_) => 1,
            Value::I32(_) | Value::F32(_) => 4,
            Value::Str(s) => 1 + s.len(),
            Value::Vec3(_) => 12,
        }
    }
    fn write(&self, w: &mut Writer) -> Result<()> {
        match self {
            Value::Bool(b) => {
                w.u8(0);
                w.bool(*b);
            }
            Value::I32(n) => {
                w.u8(1);
                w.i32(*n);
            }
            Value::F32(f) => {
                w.u8(2);
                w.f32(*f);
            }
            Value::Str(s) => {
                if s.len() > MAX_STRING {
                    return Err(Error::new("mob sync: a string value is over 64 bytes"));
                }
                w.u8(3);
                w.u8(s.len() as u8);
                w.bytes(s.as_bytes());
            }
            Value::Vec3(v) => {
                w.u8(4);
                v.iter().for_each(|f| w.f32(*f));
            }
            Value::Enum(i) => {
                w.u8(5);
                w.u8(*i);
            }
        }
        Ok(())
    }
    fn read(r: &mut Reader) -> Result<Value> {
        Ok(match r.u8()? {
            0 => Value::Bool(r.u8()? != 0),
            1 => Value::I32(r.i32()?),
            2 => Value::F32(r.f32()?),
            3 => {
                let len = usize::from(r.u8()?);
                if len > MAX_STRING {
                    return Err(Error::new("mob sync: a string value is over 64 bytes"));
                }
                let s = std::str::from_utf8(r.bytes(len)?)
                    .map_err(|e| Error::new(format!("mob sync: bad UTF-8: {e}")))?;
                Value::Str(s.to_owned())
            }
            4 => Value::Vec3([r.f32()?, r.f32()?, r.f32()?]),
            5 => Value::Enum(r.u8()?),
            tag => return Err(Error::new(format!("mob sync: unknown value tag {tag}"))),
        })
    }
}

/// The most payload bytes a value of a type's `kind` takes on the wire (`None` for an unknown kind).
pub fn max_value_len(kind: &str) -> Option<usize> {
    Some(match kind {
        "bool" | "enum" => 1,
        "i32" | "f32" => 4,
        "vec3" => 12,
        "string" => 1 + MAX_STRING,
        _ => return None,
    })
}

/// Tracked values by slot: bit `n` of `mask` is slot `n`, `values` are in ascending slot order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValueSet {
    pub mask: u32,
    pub values: Vec<Value>,
}

impl ValueSet {
    /// The slots and values in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Value)> {
        (0..MAX_VALUES)
            .filter(|n| self.mask & (1 << n) != 0)
            .zip(&self.values)
    }
    /// Bytes of the values on the wire.
    pub fn value_bytes(&self) -> usize {
        self.values.iter().map(Value::wire_len).sum()
    }
    fn check(&self) -> Result<()> {
        if self.mask.count_ones() as usize != self.values.len() {
            return Err(Error::new("mob sync: mask and values disagree"));
        }
        if self.value_bytes() > MAX_VALUE_BYTES {
            return Err(Error::new("mob sync: over 256 bytes of values"));
        }
        Ok(())
    }
    fn write(&self, w: &mut Writer) -> Result<()> {
        self.check()?;
        w.u32(self.mask);
        self.values.iter().try_for_each(|v| v.write(w))
    }
    fn read(r: &mut Reader) -> Result<ValueSet> {
        let mask = r.u32()?;
        let mut values = Vec::new();
        let mut bytes = 0;
        for _ in 0..mask.count_ones() {
            let start = r.position();
            values.push(Value::read(r)?);
            bytes += r.position() - start - 1;
            if bytes > MAX_VALUE_BYTES {
                return Err(Error::new("mob sync: over 256 bytes of values"));
            }
        }
        Ok(ValueSet { mask, values })
    }
}

/// The clocks of one controller layer's current state: every animation slot of it, anchored at
/// `tick`. A newer one replaces the layer's older one whole.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerAnchor {
    pub layer: u8,
    pub state: u8,
    /// Server game tick of the anchor.
    pub tick: i64,
    /// Animation time in seconds at `tick` and the speed after it, per animation slot.
    pub slots: Vec<(f32, f32)>,
}

impl LayerAnchor {
    fn write(&self, w: &mut Writer) -> Result<()> {
        if usize::from(self.layer) >= MAX_LAYERS || self.slots.len() > MAX_SLOTS {
            return Err(Error::new("mob sync: anchor layer or slot out of range"));
        }
        w.u8(self.layer);
        w.u8(self.state);
        w.i64(self.tick);
        w.u8(self.slots.len() as u8);
        for (time, speed) in &self.slots {
            w.f32(*time);
            w.f32(*speed);
        }
        Ok(())
    }
    fn read(r: &mut Reader) -> Result<LayerAnchor> {
        let layer = r.u8()?;
        if usize::from(layer) >= MAX_LAYERS {
            return Err(Error::new("mob sync: anchor layer out of range"));
        }
        let state = r.u8()?;
        let tick = r.i64()?;
        let n = usize::from(r.u8()?);
        if n > MAX_SLOTS {
            return Err(Error::new("mob sync: too many anchor slots"));
        }
        let mut slots = Vec::with_capacity(n);
        for _ in 0..n {
            slots.push((r.f32()?, r.f32()?));
        }
        Ok(LayerAnchor { layer, state, tick, slots })
    }
}

fn write_anchors(w: &mut Writer, anchors: &[LayerAnchor]) -> Result<()> {
    if anchors.len() > MAX_LAYERS {
        return Err(Error::new("mob sync: too many anchors"));
    }
    w.u8(anchors.len() as u8);
    anchors.iter().try_for_each(|a| a.write(w))
}

fn read_anchors(r: &mut Reader) -> Result<Vec<LayerAnchor>> {
    let n = usize::from(r.u8()?);
    if n > MAX_LAYERS {
        return Err(Error::new("mob sync: too many anchors"));
    }
    (0..n).map(|_| LayerAnchor::read(r)).collect()
}

/// A one-shot animation: which trigger (the global index over the controller layers' trigger lists
/// in file order), when it started and how fast it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub id: u16,
    pub start_tick: i64,
    /// The speed times 16 ([`speed_byte`]).
    pub speed: u8,
}

impl Trigger {
    fn write(&self, w: &mut Writer) {
        w.u16(self.id);
        w.i64(self.start_tick);
        w.u8(self.speed);
    }
    fn read(r: &mut Reader) -> Result<Trigger> {
        Ok(Trigger { id: r.u16()?, start_tick: r.i64()?, speed: r.u8()? })
    }
}

/// Everything about a mob, right after its `AddEntity`.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub entity: i32,
    /// Identifies the layout of the type's tracked values and controllers; a client whose type
    /// differs ignores the mob's data.
    pub type_version: u32,
    pub values: ValueSet,
    pub anchors: Vec<LayerAnchor>,
    /// The most recent triggers, oldest first (at most [`TRIGGER_RING`]).
    pub triggers: Vec<Trigger>,
    /// Bit `n` is set while hitbox part `n` is enabled.
    pub hitbox_mask: u32,
}

/// What changed about a mob since its last delta (merged over the ticks in between).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delta {
    pub entity: i32,
    pub values: Option<ValueSet>,
    pub anchors: Vec<LayerAnchor>,
    pub hitbox_mask: Option<u32>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.values.is_none() && self.anchors.is_empty() && self.hitbox_mask.is_none()
    }
}

/// One mob rider's seat on a custom vehicle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatEntry {
    /// The rider's entity id.
    pub rider: i32,
    /// The index of the seat in the vehicle type's `seats`.
    pub seat: u8,
}

/// Who sits where on the custom vehicle `entity` (F4M-4d); the whole list, so an empty one frees every seat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seats {
    pub entity: i32,
    pub seats: Vec<SeatEntry>,
}

/// A one-shot animation that started (sent at once, not merged).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerStart {
    pub entity: i32,
    pub trigger: Trigger,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Snapshot(Snapshot),
    Delta(Delta),
    Trigger(TriggerStart),
    /// The seats of a custom vehicle's mob riders (F4M-4d).
    Seats(Seats),
}

impl Message {
    pub fn entity(&self) -> i32 {
        match self {
            Message::Snapshot(m) => m.entity,
            Message::Delta(m) => m.entity,
            Message::Trigger(m) => m.entity,
            Message::Seats(m) => m.entity,
        }
    }
}

const SNAPSHOT: u8 = 1;
const DELTA: u8 = 2;
const TRIGGER: u8 = 3;
const SEATS: u8 = 4;
const HAS_VALUES: u8 = 1;
const HAS_ANCHORS: u8 = 2;
const HAS_HITBOX: u8 = 4;

impl Message {
    fn write(&self, w: &mut Writer) -> Result<()> {
        let mut body = Writer::new();
        let kind = match self {
            Message::Snapshot(m) => {
                body.varint(m.entity);
                body.u32(m.type_version);
                m.values.write(&mut body)?;
                write_anchors(&mut body, &m.anchors)?;
                if m.triggers.len() > TRIGGER_RING * 2 {
                    return Err(Error::new("mob sync: too many triggers"));
                }
                body.u8(m.triggers.len() as u8);
                m.triggers.iter().for_each(|t| t.write(&mut body));
                body.u32(m.hitbox_mask);
                SNAPSHOT
            }
            Message::Delta(m) => {
                body.varint(m.entity);
                let flags = u8::from(m.values.is_some()) * HAS_VALUES
                    | u8::from(!m.anchors.is_empty()) * HAS_ANCHORS
                    | u8::from(m.hitbox_mask.is_some()) * HAS_HITBOX;
                body.u8(flags);
                if let Some(v) = &m.values {
                    v.write(&mut body)?;
                }
                if !m.anchors.is_empty() {
                    write_anchors(&mut body, &m.anchors)?;
                }
                if let Some(h) = m.hitbox_mask {
                    body.u32(h);
                }
                DELTA
            }
            Message::Trigger(m) => {
                body.varint(m.entity);
                m.trigger.write(&mut body);
                TRIGGER
            }
            Message::Seats(m) => {
                if m.seats.len() > MAX_SEATS {
                    return Err(Error::new("mob sync: too many seats"));
                }
                body.varint(m.entity);
                body.u8(m.seats.len() as u8);
                for s in &m.seats {
                    body.varint(s.rider);
                    body.u8(s.seat);
                }
                SEATS
            }
        };
        w.u8(kind);
        w.varint(body.data.len() as i32);
        w.bytes(&body.data);
        Ok(())
    }

    fn read(kind: u8, body: &[u8]) -> Result<Option<Message>> {
        let mut r = Reader::new(body);
        let m = match kind {
            SNAPSHOT => {
                let entity = r.varint()?;
                let type_version = r.u32()?;
                let values = ValueSet::read(&mut r)?;
                let anchors = read_anchors(&mut r)?;
                let n = usize::from(r.u8()?);
                if n > TRIGGER_RING * 2 {
                    return Err(Error::new("mob sync: too many triggers"));
                }
                let triggers = (0..n).map(|_| Trigger::read(&mut r)).collect::<Result<_>>()?;
                let hitbox_mask = r.u32()?;
                Message::Snapshot(Snapshot { entity, type_version, values, anchors, triggers, hitbox_mask })
            }
            DELTA => {
                let entity = r.varint()?;
                let flags = r.u8()?;
                let values = if flags & HAS_VALUES != 0 { Some(ValueSet::read(&mut r)?) } else { None };
                let anchors = if flags & HAS_ANCHORS != 0 { read_anchors(&mut r)? } else { Vec::new() };
                let hitbox_mask = if flags & HAS_HITBOX != 0 { Some(r.u32()?) } else { None };
                Message::Delta(Delta { entity, values, anchors, hitbox_mask })
            }
            TRIGGER => Message::Trigger(TriggerStart { entity: r.varint()?, trigger: Trigger::read(&mut r)? }),
            SEATS => {
                let entity = r.varint()?;
                let n = usize::from(r.u8()?);
                if n > MAX_SEATS {
                    return Err(Error::new("mob sync: too many seats"));
                }
                let seats = (0..n)
                    .map(|_| Ok(SeatEntry { rider: r.varint()?, seat: r.u8()? }))
                    .collect::<Result<_>>()?;
                Message::Seats(Seats { entity, seats })
            }
            // A kind of a later version: skipped by its length.
            _ => return Ok(None),
        };
        if r.remaining() != 0 {
            return Err(Error::new("mob sync: bytes left after a message"));
        }
        Ok(Some(m))
    }
}

/// A payload: the server game time the data is from and the messages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub game_time: i64,
    pub messages: Vec<Message>,
    /// Decoding only: messages that were skipped (a kind of a later version or a bad body).
    pub skipped: usize,
}

impl Frame {
    pub fn new(game_time: i64) -> Self {
        Frame { game_time, messages: Vec::new(), skipped: 0 }
    }
}

/// A frame's payload; an error for a message that does not fit its limits.
pub fn encode(frame: &Frame) -> Result<Vec<u8>> {
    if frame.messages.len() > MAX_MESSAGES {
        return Err(Error::new("mob sync: too many messages"));
    }
    let mut w = Writer::new();
    w.u8(VERSION);
    w.i64(frame.game_time);
    w.varint(frame.messages.len() as i32);
    for m in &frame.messages {
        m.write(&mut w)?;
    }
    Ok(w.data)
}

/// The size of one message on the wire (used to fill frames).
pub fn message_len(m: &Message) -> Result<usize> {
    let mut w = Writer::new();
    m.write(&mut w)?;
    Ok(w.data.len())
}

/// A frame from a payload. An unknown version or a header that does not parse is an error; a
/// message with a bad body or of an unknown kind is skipped and counted in `skipped`.
pub fn decode(data: &[u8]) -> Result<Frame> {
    let mut r = Reader::new(data);
    let version = r.u8()?;
    if version != VERSION {
        return Err(Error::new(format!("mob sync: unknown version {version}")));
    }
    let game_time = r.i64()?;
    let count = r.count(MAX_MESSAGES)?;
    let mut frame = Frame::new(game_time);
    for _ in 0..count {
        let kind = r.u8()?;
        let len = r.count(r.remaining())?;
        let body = r.bytes(len)?;
        match Message::read(kind, body) {
            Ok(Some(m)) => frame.messages.push(m),
            Ok(None) | Err(_) => frame.skipped += 1,
        }
    }
    if r.remaining() != 0 {
        return Err(Error::new("mob sync: bytes left after the frame"));
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> ValueSet {
        ValueSet {
            mask: 0b1_0111_1001,
            values: vec![
                Value::Bool(true),
                Value::I32(-77),
                Value::F32(1.5),
                Value::Str("héllo".into()),
                Value::Vec3([1.0, -2.0, 3.5]),
                Value::Enum(2),
            ],
        }
    }
    fn anchors() -> Vec<LayerAnchor> {
        vec![
            LayerAnchor { layer: 0, state: 3, tick: 1_000_000_007, slots: vec![(1.25, 2.0), (0.0, 0.5)] },
            LayerAnchor { layer: 2, state: 0, tick: -5, slots: vec![] },
        ]
    }
    fn trigger(id: u16, tick: i64) -> Trigger {
        Trigger { id, start_tick: tick, speed: speed_byte(1.0) }
    }

    fn round(frame: &Frame) -> Frame {
        decode(&encode(frame).unwrap()).unwrap()
    }

    #[test]
    fn every_message_and_value_type_round_trips() {
        let mut frame = Frame::new(123_456_789_012);
        frame.messages = vec![
            Message::Snapshot(Snapshot {
                entity: 41,
                type_version: 0xDEAD_BEEF,
                values: values(),
                anchors: anchors(),
                triggers: vec![trigger(1, 100), trigger(0, 120)],
                hitbox_mask: 0b101,
            }),
            Message::Delta(Delta { entity: 300, values: Some(values()), anchors: anchors(), hitbox_mask: Some(7) }),
            Message::Delta(Delta { entity: 1, values: None, anchors: vec![], hitbox_mask: Some(0) }),
            Message::Delta(Delta { entity: 2, values: Some(ValueSet::default()), anchors: vec![], hitbox_mask: None }),
            Message::Trigger(TriggerStart { entity: -3, trigger: trigger(65_535, i64::MAX) }),
        ];
        assert_eq!(round(&frame), frame);
        // An empty frame is just the header.
        assert_eq!(encode(&Frame::new(5)).unwrap().len(), 1 + 8 + 1);
    }

    #[test]
    fn the_byte_layout_of_a_trigger_message_is_the_documented_one() {
        let mut f = Frame::new(0x0102_0304_0506_0708);
        f.messages = vec![Message::Trigger(TriggerStart { entity: 5, trigger: Trigger { id: 0x0A0B, start_tick: 9, speed: 16 } })];
        assert_eq!(
            encode(&f).unwrap(),
            [
                1, // version
                1, 2, 3, 4, 5, 6, 7, 8, // game time
                1, // one message
                3, // kind trigger
                1 + 2 + 8 + 1, // body length
                5, // entity
                0x0A, 0x0B, // trigger id
                0, 0, 0, 0, 0, 0, 0, 9, // start tick
                16, // speed
            ]
        );
    }

    #[test]
    fn a_value_set_is_limited_to_256_bytes_and_the_mask_must_match() {
        // 3 strings of 64 bytes = 195 payload bytes fit; 4 (260) do not.
        let s = |n: usize| Value::Str("x".repeat(n));
        let three = ValueSet { mask: 0b111, values: vec![s(64), s(64), s(64)] };
        assert_eq!(three.value_bytes(), 195);
        let mut f = Frame::new(1);
        f.messages = vec![Message::Delta(Delta { entity: 1, values: Some(three), ..Default::default() })];
        assert_eq!(round(&f), f);
        let four = ValueSet { mask: 0b1111, values: vec![s(64), s(64), s(64), s(64)] };
        f.messages = vec![Message::Delta(Delta { entity: 1, values: Some(four), ..Default::default() })];
        assert!(encode(&f).is_err());
        // A string past 64 bytes, and a mask that disagrees with the values.
        let long = ValueSet { mask: 1, values: vec![s(65)] };
        f.messages = vec![Message::Delta(Delta { entity: 1, values: Some(long), ..Default::default() })];
        assert!(encode(&f).is_err());
        let off = ValueSet { mask: 0b11, values: vec![Value::Bool(true)] };
        f.messages = vec![Message::Delta(Delta { entity: 1, values: Some(off), ..Default::default() })];
        assert!(encode(&f).is_err());
        // The schema's own sizes: 65 bytes for the longest string, 12 for a vec3.
        assert_eq!(max_value_len("string"), Some(65));
        assert_eq!(max_value_len("vec3"), Some(12));
        assert_eq!(max_value_len("thing"), None);
    }

    #[test]
    fn a_bad_message_is_skipped_and_the_rest_of_the_frame_is_kept() {
        let good = Message::Trigger(TriggerStart { entity: 1, trigger: trigger(2, 3) });
        // A trigger whose body is too short, then a good one.
        let mut w = Writer::new();
        w.u8(VERSION);
        w.i64(9);
        w.varint(2);
        w.u8(TRIGGER);
        w.varint(2);
        w.bytes(&[1, 0]);
        good.write(&mut w).unwrap();
        let back = decode(&w.data).unwrap();
        assert_eq!(back.messages, vec![good.clone()]);
        assert_eq!(back.skipped, 1);
        // A kind of a later version is skipped by its length.
        let mut w = Writer::new();
        w.u8(VERSION);
        w.i64(4);
        w.varint(2);
        w.u8(99);
        w.varint(3);
        w.bytes(&[1, 2, 3]);
        Message::Trigger(TriggerStart { entity: 1, trigger: trigger(2, 3) }).write(&mut w).unwrap();
        let back = decode(&w.data).unwrap();
        assert_eq!((back.messages.len(), back.skipped), (1, 1));
    }

    #[test]
    fn garbage_is_an_error_never_a_panic() {
        assert!(decode(&[]).is_err());
        assert!(decode(&[2]).is_err(), "unknown version");
        let mut f = Frame::new(1);
        f.messages = vec![
            Message::Snapshot(Snapshot {
                entity: 1,
                type_version: 1,
                values: values(),
                anchors: anchors(),
                triggers: vec![trigger(1, 1)],
                hitbox_mask: 0,
            }),
            Message::Trigger(TriggerStart { entity: 1, trigger: trigger(1, 1) }),
        ];
        let bytes = encode(&f).unwrap();
        // Every truncation and every single-byte change decodes to something or an error.
        for n in 0..bytes.len() {
            let _ = decode(&bytes[..n]);
        }
        for i in 0..bytes.len() {
            for b in [0u8, 1, 0x7F, 0x80, 0xFF] {
                let mut x = bytes.clone();
                x[i] = b;
                let _ = decode(&x);
            }
        }
        // A message count past the limit.
        let mut w = Writer::new();
        w.u8(VERSION);
        w.i64(0);
        w.varint(MAX_MESSAGES as i32 + 1);
        assert!(decode(&w.data).is_err());
    }

    #[test]
    fn speed_bytes() {
        assert_eq!(speed_byte(1.0), 16);
        assert_eq!(speed_from_byte(16), 1.0);
        assert_eq!(speed_byte(-3.0), 0);
        assert_eq!(speed_byte(1000.0), 255);
        assert_eq!(speed_byte(f32::NAN), 16);
        assert_eq!(speed_byte(0.5), 8);
    }

    /// The numbers a design review wants: what a typical mob costs per message.
    #[test]
    fn typical_messages_are_small() {
        let one_layer = vec![LayerAnchor { layer: 0, state: 1, tick: 100, slots: vec![(0.5, 1.0)] }];
        let delta = Message::Delta(Delta { entity: 200, values: None, anchors: one_layer, hitbox_mask: None });
        assert_eq!(message_len(&delta).unwrap(), 25);
        let trigger = Message::Trigger(TriggerStart { entity: 200, trigger: trigger(0, 5) });
        assert_eq!(message_len(&trigger).unwrap(), 2 + 2 + 2 + 8 + 1);
    }

    /// F4M-4d: a `Seats` message round trips, has the documented bytes, may be empty, and is limited to
    /// `MAX_SEATS` entries (a bad body is skipped, the frame survives).
    #[test]
    fn a_seats_message_round_trips_and_is_bounded() {
        let seats = Seats {
            entity: 300,
            seats: vec![SeatEntry { rider: 7, seat: 0 }, SeatEntry { rider: 70_000, seat: 3 }],
        };
        let mut f = Frame::new(77);
        f.messages = vec![Message::Seats(seats.clone()), Message::Seats(Seats { entity: 1, seats: vec![] })];
        assert_eq!(round(&f), f);
        assert_eq!(Message::Seats(seats.clone()).entity(), 300);
        // kind 4, body = entity varint(300) = 2 bytes, count, (rider varint, seat) * 2.
        let mut w = Writer::new();
        Message::Seats(Seats { entity: 5, seats: vec![SeatEntry { rider: 9, seat: 2 }] }).write(&mut w).unwrap();
        assert_eq!(w.data, [4, 4, 5, 1, 9, 2]);
        // 16 fit, 17 do not.
        let many = |n: usize| Seats { entity: 1, seats: (0..n).map(|i| SeatEntry { rider: i as i32, seat: i as u8 }).collect() };
        f.messages = vec![Message::Seats(many(MAX_SEATS))];
        assert_eq!(round(&f), f);
        f.messages = vec![Message::Seats(many(MAX_SEATS + 1))];
        assert!(encode(&f).is_err());
        // A body that claims more seats than the limit, or has bytes left, is skipped like any bad body.
        let mut w = Writer::new();
        w.u8(VERSION);
        w.i64(1);
        w.varint(2);
        w.u8(SEATS);
        w.varint(3);
        w.bytes(&[1, 99, 0]);
        w.u8(SEATS);
        w.varint(5);
        w.bytes(&[1, 0, 1, 2, 3]);
        let back = decode(&w.data).unwrap();
        assert_eq!((back.messages.len(), back.skipped), (0, 2));
    }
}
