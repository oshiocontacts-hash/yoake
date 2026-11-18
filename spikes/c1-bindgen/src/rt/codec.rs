use super::CallError;

pub trait Encode {
    fn encode(&self, out: &mut Vec<u8>);
}

pub trait Decode: Sized {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError>;
}

pub struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub fn finish(&self) -> Result<(), CallError> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(CallError::TrailingBytes)
        }
    }

    pub fn take(&mut self, len: usize) -> Result<&'a [u8], CallError> {
        let end = self.position.checked_add(len).ok_or(CallError::LengthOverflow)?;
        let bytes = self.bytes.get(self.position..end).ok_or(CallError::UnexpectedEnd)?;
        self.position = end;
        Ok(bytes)
    }

    fn len(&mut self) -> Result<usize, CallError> {
        usize::try_from(u32::decode(self)?).map_err(|_| CallError::LengthOverflow)
    }

    pub fn bytes(&mut self) -> Result<&'a [u8], CallError> {
        let len = self.len()?;
        self.take(len)
    }
}

macro_rules! integer_codec {
    ($($ty:ty),* $(,)?) => {$ (
        impl Encode for $ty {
            fn encode(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
        }
        impl Decode for $ty {
            fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
                let bytes: [u8; std::mem::size_of::<$ty>()] = input
                    .take(std::mem::size_of::<$ty>())?
                    .try_into()
                    .map_err(|_| CallError::UnexpectedEnd)?;
                Ok(<$ty>::from_le_bytes(bytes))
            }
        }
    )* };
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
        let text = std::str::from_utf8(bytes).map_err(|_| CallError::InvalidUtf8)?;
        Ok(text.to_owned())
    }
}

pub fn encode_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    (bytes.len() as u32).encode(out);
    out.extend_from_slice(bytes);
}

impl Encode for [u8] {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_bytes(self, out);
    }
}

impl<T: Encode + ?Sized> Encode for &T {
    fn encode(&self, out: &mut Vec<u8>) {
        (*self).encode(out);
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
        let items = (0..N)
            .map(|_| T::decode(input))
            .collect::<Result<Vec<_>, _>>()?;
        items.try_into().map_err(|_| CallError::UnexpectedEnd)
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        (self.len() as u32).encode(out);
        for item in self {
            item.encode(out);
        }
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        let len = input.len()?;
        (0..len).map(|_| T::decode(input)).collect()
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
        match u8::decode(input)? {
            0 => Ok(None),
            1 => Ok(Some(T::decode(input)?)),
            tag => Err(CallError::InvalidTag("Option", tag)),
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
        match u8::decode(input)? {
            0 => Ok(Ok(T::decode(input)?)),
            1 => Ok(Err(E::decode(input)?)),
            tag => Err(CallError::InvalidTag("Result", tag)),
        }
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

impl<A: Encode, B: Encode> Encode for (A, B) {
    fn encode(&self, out: &mut Vec<u8>) {
        self.0.encode(out);
        self.1.encode(out);
    }
}

impl<A: Decode, B: Decode> Decode for (A, B) {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        Ok((A::decode(input)?, B::decode(input)?))
    }
}

impl<A: Encode, B: Encode, C: Encode> Encode for (A, B, C) {
    fn encode(&self, out: &mut Vec<u8>) {
        self.0.encode(out);
        self.1.encode(out);
        self.2.encode(out);
    }
}

impl<A: Decode, B: Decode, C: Decode> Decode for (A, B, C) {
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CallError> {
        Ok((A::decode(input)?, B::decode(input)?, C::decode(input)?))
    }
}
