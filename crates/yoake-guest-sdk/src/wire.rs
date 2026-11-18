//! The guest side of the wire codec: the same bytes as `yoake_slice::codec` (the host side), `no_std`.
//! Generated stubs use it under the name `codec` and `CallError`, so the host and guest text match.
//!
//! Little-endian fixed-width numbers, one-byte tags for `bool`/`Option`/`Result`, 32-bit lengths
//! for strings and vectors, fixed arrays and tuples in field order, `usize`/`isize` as 64 bits,
//! `char` as a 32-bit scalar value. Generated code calls these helpers directly; it never builds a
//! pointer, so everything that can go wrong is a [`CallError`], not a trap.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Why a generated call did not run to the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallError {
    UnexpectedEnd,
    TrailingBytes,
    InvalidBool(u8),
    InvalidChar(u32),
    InvalidTag(&'static str, u32),
    InvalidUtf8,
    LengthOverflow,
    /// A value nests deeper than [`MAX_DECODE_DEPTH`].
    TooDeep,
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd => write!(f, "argument buffer ended early"),
            Self::TrailingBytes => write!(f, "argument buffer has trailing bytes"),
            Self::InvalidBool(tag) => write!(f, "invalid bool tag {tag}"),
            Self::InvalidChar(value) => write!(f, "{value:#x} is not a char"),
            Self::InvalidTag(kind, tag) => write!(f, "invalid {kind} tag {tag}"),
            Self::InvalidUtf8 => write!(f, "string is not UTF-8"),
            Self::LengthOverflow => write!(f, "length does not fit this host"),
            Self::TooDeep => write!(f, "value nests too deeply"),
        }
    }
}


pub trait Encode {
    fn encode(&self, out: &mut Vec<u8>);
}

pub trait Decode: Sized {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError>;
}

/// How deep decoded values may nest (`Box<Self>` chains, recursive structs and enums).
pub const MAX_DECODE_DEPTH: u32 = 64;

/// The most elements a decoder reserves up front; longer vectors grow as elements really arrive, so
/// a forged length (up to the argument cap) cannot make the host reserve hundreds of megabytes.
pub const MAX_PREALLOC: usize = 4096;

/// A checked reader over the argument bytes.
pub struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
    depth: u32,
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0, depth: 0 }
    }

    /// Fails unless every byte was read (a call with extra bytes is a malformed call).
    pub fn finish(&self) -> Result<(), CallError> {
        if self.position == self.bytes.len() { Ok(()) } else { Err(CallError::TrailingBytes) }
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub fn take(&mut self, len: usize) -> Result<&'a [u8], CallError> {
        let end = self.position.checked_add(len).ok_or(CallError::LengthOverflow)?;
        let bytes = self.bytes.get(self.position..end).ok_or(CallError::UnexpectedEnd)?;
        self.position = end;
        Ok(bytes)
    }

    /// A 32-bit length prefix.
    pub fn len(&mut self) -> Result<usize, CallError> {
        usize::try_from(u32::decode(self)?).map_err(|_| CallError::LengthOverflow)
    }

    /// A length-prefixed byte string.
    pub fn bytes(&mut self) -> Result<&'a [u8], CallError> {
        let len = self.len()?;
        self.take(len)
    }

    /// Enters one nesting level (generated decoders of named types and `Box` call this); pair with
    /// [`Decoder::leave`]. Fails past [`MAX_DECODE_DEPTH`].
    pub fn enter(&mut self) -> Result<(), CallError> {
        if self.depth >= MAX_DECODE_DEPTH {
            return Err(CallError::TooDeep);
        }
        self.depth += 1;
        Ok(())
    }

    pub fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// `Vec::with_capacity` for `len` elements, capped at [`MAX_PREALLOC`].
    pub fn vec_with<T>(len: usize) -> Vec<T> {
        Vec::with_capacity(len.min(MAX_PREALLOC))
    }

    /// A one-byte tag.
    pub fn tag(&mut self) -> Result<u8, CallError> {
        u8::decode(self)
    }

    /// A vector's length, bounded by the bytes that are left (one byte per element at least, so a
    /// forged length cannot make the decoder reserve gigabytes).
    pub fn vec_len(&mut self) -> Result<usize, CallError> {
        let len = self.len()?;
        if len > self.remaining() { Err(CallError::UnexpectedEnd) } else { Ok(len) }
    }
}

/// A 32-bit length prefix, the writer side of [`Decoder::len`].
pub fn enc_len(len: usize, out: &mut Vec<u8>) {
    (len as u32).encode(out);
}

pub fn encode_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    enc_len(bytes.len(), out);
    out.extend_from_slice(bytes);
}

macro_rules! integer_codec {
    ($($ty:ty),* $(,)?) => {$(
        impl Encode for $ty {
            fn encode(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
        }
        impl Decode for $ty {
            fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
                let bytes: [u8; core::mem::size_of::<$ty>()] =
                    input.take(core::mem::size_of::<$ty>())?.try_into().map_err(|_| CallError::UnexpectedEnd)?;
                Ok(<$ty>::from_le_bytes(bytes))
            }
        }
    )*};
}

integer_codec!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

impl Encode for bool {
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
}

impl Decode for bool {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        match u8::decode(input)? {
            0 => Ok(false),
            1 => Ok(true),
            tag => Err(CallError::InvalidBool(tag)),
        }
    }
}

impl Encode for char {
    fn encode(&self, out: &mut Vec<u8>) {
        u32::from(*self).encode(out);
    }
}

impl Decode for char {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        let value = u32::decode(input)?;
        char::from_u32(value).ok_or(CallError::InvalidChar(value))
    }
}

impl Encode for usize {
    fn encode(&self, out: &mut Vec<u8>) {
        (*self as u64).encode(out);
    }
}

impl Decode for usize {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        usize::try_from(u64::decode(input)?).map_err(|_| CallError::LengthOverflow)
    }
}

impl Encode for isize {
    fn encode(&self, out: &mut Vec<u8>) {
        (*self as i64).encode(out);
    }
}

impl Decode for isize {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        isize::try_from(i64::decode(input)?).map_err(|_| CallError::LengthOverflow)
    }
}

impl Encode for str {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_bytes(self.as_bytes(), out);
    }
}

impl Encode for String {
    fn encode(&self, out: &mut Vec<u8>) {
        self.as_str().encode(out);
    }
}

impl Decode for String {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        let bytes = input.bytes()?;
        core::str::from_utf8(bytes).map(str::to_owned).map_err(|_| CallError::InvalidUtf8)
    }
}

impl<T: Encode + ?Sized> Encode for &T {
    fn encode(&self, out: &mut Vec<u8>) {
        (**self).encode(out);
    }
}

impl Encode for () {
    fn encode(&self, _out: &mut Vec<u8>) {}
}

impl Decode for () {
    fn decode(_input: &mut Decoder<'_>) -> Result<Self, CallError> {
        Ok(())
    }
}

impl<T: Encode, const N: usize> Encode for [T; N] {
    fn encode(&self, out: &mut Vec<u8>) {
        for item in self {
            item.encode(out);
        }
    }
}

impl<T: Decode, const N: usize> Decode for [T; N] {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        let mut items = Decoder::vec_with(N.min(input.remaining()));
        for _ in 0..N {
            items.push(T::decode(input)?);
        }
        items.try_into().map_err(|_| CallError::UnexpectedEnd)
    }
}

impl<T: Encode> Encode for [T] {
    fn encode(&self, out: &mut Vec<u8>) {
        enc_len(self.len(), out);
        for item in self {
            item.encode(out);
        }
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        self.as_slice().encode(out);
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        let len = input.vec_len()?;
        let mut items = Decoder::vec_with(len);
        for _ in 0..len {
            items.push(T::decode(input)?);
        }
        Ok(items)
    }
}

impl<T: Encode> Encode for Option<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            None => 0u8.encode(out),
            Some(value) => {
                1u8.encode(out);
                value.encode(out);
            }
        }
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        match input.tag()? {
            0 => Ok(None),
            1 => Ok(Some(T::decode(input)?)),
            tag => Err(CallError::InvalidTag("Option", u32::from(tag))),
        }
    }
}

impl<T: Encode, E: Encode> Encode for Result<T, E> {
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Ok(value) => {
                0u8.encode(out);
                value.encode(out);
            }
            Err(error) => {
                1u8.encode(out);
                error.encode(out);
            }
        }
    }
}

impl<T: Decode, E: Decode> Decode for Result<T, E> {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        match input.tag()? {
            0 => Ok(Ok(T::decode(input)?)),
            1 => Ok(Err(E::decode(input)?)),
            tag => Err(CallError::InvalidTag("Result", u32::from(tag))),
        }
    }
}

impl<T: Encode + ?Sized> Encode for Box<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        (**self).encode(out);
    }
}

impl<T: Decode> Decode for Box<T> {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        input.enter()?;
        let value = T::decode(input);
        input.leave();
        Ok(Box::new(value?))
    }
}

macro_rules! tuple_codec {
    ($($name:ident $idx:tt),+) => {
        impl<$($name: Encode),+> Encode for ($($name,)+) {
            fn encode(&self, out: &mut Vec<u8>) {
                $(self.$idx.encode(out);)+
            }
        }
        impl<$($name: Decode),+> Decode for ($($name,)+) {
            fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
                Ok(($($name::decode(input)?,)+))
            }
        }
    };
}

tuple_codec!(A 0);
tuple_codec!(A 0, B 1);
tuple_codec!(A 0, B 1, C 2);
tuple_codec!(A 0, B 1, C 2, D 3);
tuple_codec!(A 0, B 1, C 2, D 3, E 4);
tuple_codec!(A 0, B 1, C 2, D 3, E 4, F 5);


#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn round<T: Encode + Decode + PartialEq + core::fmt::Debug>(value: T) {
        let mut bytes = Vec::new();
        value.encode(&mut bytes);
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(T::decode(&mut decoder).unwrap(), value);
        decoder.finish().unwrap();
    }

    #[test]
    fn the_guest_codec_round_trips_and_matches_the_host_bytes() {
        round(7u8);
        round(-9i64);
        round(2.5f64);
        round("hello".to_owned());
        round(vec![Some([1i32, 2, 3]), None]);
        round((1u8, "x".to_owned(), Err::<u8, _>(4u16)));
        // the layout the host codec writes: u32 length, then bytes
        let mut bytes = Vec::new();
        "ab".encode(&mut bytes);
        assert_eq!(bytes, [2, 0, 0, 0, b'a', b'b']);
        let mut bytes = Vec::new();
        1.5f32.encode(&mut bytes);
        assert_eq!(bytes, 1.5f32.to_le_bytes());
    }

    #[test]
    fn hostile_input_is_an_error() {
        assert_eq!(Vec::<u64>::decode(&mut Decoder::new(&[0, 0xca, 0x9a, 0x3b])), Err(CallError::UnexpectedEnd));
        assert_eq!(bool::decode(&mut Decoder::new(&[2])), Err(CallError::InvalidBool(2)));
        let deep: Vec<u8> = Vec::new();
        assert!(Box::<u8>::decode(&mut Decoder::new(&deep)).is_err());
    }
}
