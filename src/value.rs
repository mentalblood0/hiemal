use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    io::{BufReader, BufWriter},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::Result;
use bytes::Bytes;
use dashu::{Decimal, Rational};
use parking_lot::RwLock;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Unexpected, Visitor},
};

use crate::{
    containers::{Object, Vector},
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    r#type::{Embedded, TypeKind},
};

pub fn serialize_rope<S>(rope: &ropey::Rope, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&rope.to_string())
}

pub fn deserialize_rope<'de, D>(deserializer: D) -> Result<ropey::Rope, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(ropey::Rope::from_str(&s))
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct Identified<T> {
    id: u64,
    pub content: T,
}

impl<T> Identified<T> {
    pub fn new(content: T) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            content,
        }
    }
}

impl<T> PartialEq for Identified<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T> Eq for Identified<T> {}

impl<T> Hash for Identified<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T> PartialOrd for Identified<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Identified<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

#[derive(Debug, PartialEq, PartialOrd, Hash, Eq, Ord)]
pub enum Runtime {
    CompiledRegex(Identified<regex::Regex>),
    BufferizedWriter(Identified<RwLock<BufWriter<std::fs::File>>>),
    BufferizedReader(Identified<RwLock<BufReader<std::fs::File>>>),
    HttpClient(Identified<ureq::Agent>),
    RandomNumberGenerator(Identified<RwLock<rand::rngs::ChaCha8Rng>>)
}

impl Serialize for Runtime {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Err(serde::ser::Error::custom(
            "Runtime contains runtime-only state and cannot be serialized",
        ))
    }
}

impl<'de> Deserialize<'de> for Runtime {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "Runtime contains runtime-only state and cannot be deserialized",
        ))
    }
}

impl Runtime {
    pub fn as_compiled_regex(&self) -> Option<&regex::Regex> {
        match self {
            Runtime::CompiledRegex(Identified { id: _, content }) => Some(content),
            _ => None,
        }
    }

    pub fn as_bufferized_writer(&self) -> Option<&RwLock<BufWriter<std::fs::File>>> {
        match self {
            Runtime::BufferizedWriter(Identified { id: _, content }) => Some(content),
            _ => None,
        }
    }

    pub fn as_bufferized_reader(&self) -> Option<&RwLock<BufReader<std::fs::File>>> {
        match self {
            Runtime::BufferizedReader(Identified { id: _, content }) => Some(content),
            _ => None,
        }
    }

    pub fn as_http_client(&self) -> Option<&ureq::Agent> {
        match self {
            Runtime::HttpClient(Identified { id: _, content }) => Some(content),
            _ => None,
        }
    }

    pub fn as_random_number_generator(&self) -> Option<&RwLock<rand::rngs::ChaCha8Rng>> {
        match self {
            Runtime::RandomNumberGenerator(Identified { id: _, content }) => Some(content),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Deserialize, Serialize, PartialEq, Debug, Clone, Eq, Hash, PartialOrd, Ord)]
#[serde(untagged)]
pub enum Value {
    Number(#[serde(deserialize_with = "deserialize_rational")] Rational),
    String(
        #[serde(
            deserialize_with = "deserialize_rope",
            serialize_with = "serialize_rope"
        )]
        ropey::Rope,
    ),
    Bool(bool),
    Tuple(Vector<Option<Value>>),
    Bytes {
        #[serde(
            serialize_with = "serialize_bytes",
            deserialize_with = "deserialize_bytes"
        )]
        bytes: Bytes,
    },
    Time {
        #[serde(with = "humantime_serde")]
        time: std::time::SystemTime,
    },
    Duration {
        #[serde(with = "humantime_serde")]
        duration: std::time::Duration,
    },
    Object(Object<String, Option<Value>>),
    Runtime(Arc<Runtime>),
}

impl Default for Value {
    fn default() -> Self {
        Value::Bool(false)
    }
}

pub fn serialize_bytes<S>(bytes: &Bytes, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if serializer.is_human_readable() {
        use std::fmt::Write;
        let mut string = String::with_capacity(bytes.len() * 3);
        for (byte_index, byte) in bytes.iter().enumerate() {
            if byte_index > 0 {
                string.push(' ');
            }
            write!(string, "{:02x}", byte).unwrap();
        }
        serializer.serialize_str(&string)
    } else {
        serializer.serialize_bytes(bytes.as_ref())
    }
}

fn parse_spaced_hex(string: &str) -> Result<Bytes, String> {
    if string.is_empty() {
        return Ok(Bytes::new());
    }
    let mut result = Vec::with_capacity((string.len() + 1) / 3);
    for (token_index, token) in string.split(' ').enumerate() {
        let bytes = token.as_bytes();
        if bytes.len() != 2 {
            return Err(format!(
                "bad token {:?} at index {}: invalid token length",
                token, token_index
            ));
        }
        let high = hex_value(bytes[0], 0)?;
        let low = hex_value(bytes[1], 1)?;
        result.push((high << 4) | low);
    }
    Ok(Bytes::from(result))
}

fn hex_value(byte: u8, index: usize) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(format!(
            "Invalid character {:?} at position {}",
            byte as char, index
        )),
    }
}

pub fn deserialize_bytes<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
where
    D: Deserializer<'de>,
{
    struct SpacedHexVisitor;

    impl<'de> de::Visitor<'de> for SpacedHexVisitor {
        type Value = Bytes;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(r#"a space-separated hex byte string (e.g. "d4 47 b1 ea") or raw bytes"#)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            parse_spaced_hex(v).map_err(E::custom)
        }

        fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
            Ok(Bytes::copy_from_slice(v))
        }

        fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
            Ok(Bytes::from(v))
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(byte) = seq.next_element::<u8>()? {
                bytes.push(byte);
            }
            Ok(Bytes::from(bytes))
        }
    }

    if deserializer.is_human_readable() {
        deserializer.deserialize_str(SpacedHexVisitor)
    } else {
        deserializer.deserialize_bytes(SpacedHexVisitor)
    }
}

pub fn deserialize_rational<'de, D>(deserializer: D) -> Result<Rational, D::Error>
where
    D: Deserializer<'de>,
{
    struct RationalVisitor;

    impl<'de> Visitor<'de> for RationalVisitor {
        type Value = Rational;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str(
                "an integer, a float, a fraction string like \"1/2\", or a decimal string like \
                 \"1.5\"",
            )
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Rational, E> {
            Ok(Rational::from(v))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Rational, E> {
            Ok(Rational::from(v))
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Rational, E> {
            Rational::simplest_from_f64(v)
                .ok_or_else(|| de::Error::invalid_value(Unexpected::Float(v), &self))
        }

        fn visit_f32<E: de::Error>(self, v: f32) -> Result<Rational, E> {
            Rational::simplest_from_f32(v)
                .ok_or_else(|| de::Error::invalid_value(Unexpected::Float(v as f64), &self))
        }

        fn visit_str<E: de::Error>(self, s: &str) -> Result<Rational, E> {
            if s == DEFAULT_ARGUMENT_NAME {
                Err(de::Error::invalid_value(Unexpected::Str(s), &self))
            } else if let Ok(result) = Rational::from_str(s) {
                Ok(result)
            } else if let Ok(result_real) = Decimal::from_str(s) {
                if let Ok(result) = Rational::try_from(result_real) {
                    Ok(result)
                } else {
                    Err(de::Error::invalid_value(Unexpected::Str(s), &self))
                }
            } else {
                Err(de::Error::invalid_value(Unexpected::Str(s), &self))
            }
        }

        fn visit_borrowed_str<E: de::Error>(self, s: &'de str) -> Result<Rational, E> {
            self.visit_str(s)
        }

        fn visit_string<E: de::Error>(self, s: String) -> Result<Rational, E> {
            self.visit_str(&s)
        }
    }

    deserializer.deserialize_any(RationalVisitor)
}

impl Value {
    pub fn as_number(&self) -> Option<Rational> {
        match self {
            Value::Number(result) => Some(result.clone()),
            _ => None,
        }
    }

    pub fn as_time(&self) -> Option<std::time::SystemTime> {
        match self {
            Value::Time { time: result } => Some(*result),
            _ => None,
        }
    }

    pub fn as_duration(&self) -> Option<std::time::Duration> {
        match self {
            Value::Duration { duration: result } => Some(*result),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&ropey::Rope> {
        match self {
            Value::String(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&Bytes> {
        match self {
            Value::Bytes { bytes: result } => Some(result),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(result) => Some(*result),
            _ => None,
        }
    }

    pub fn as_tuple(&self) -> Option<&Vector<Option<Value>>> {
        match self {
            Value::Tuple(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_runtime(&self) -> Option<&Runtime> {
        match self {
            Value::Runtime(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Object<String, Option<Value>>> {
        match self {
            Value::Object(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Object<String, Option<Value>>> {
        match self {
            Value::Object(result) => Some(result),
            _ => None,
        }
    }

    pub fn type_kind(value_option: &Option<Value>) -> TypeKind {
        match value_option {
            Some(value) => match value {
                Value::Number(_) => TypeKind::Number,
                Value::String(string) => TypeKind::LiteralString(string.clone()),
                Value::Bytes { bytes: _ } => TypeKind::Bytes,
                Value::Time { time: _ } => TypeKind::Time,
                Value::Duration { duration: _ } => TypeKind::Duration,
                Value::Bool(true) => TypeKind::LiteralTrue,
                Value::Bool(false) => TypeKind::LiteralFalse,
                Value::Tuple(tuple) => TypeKind::Tuple(
                    tuple
                        .iter()
                        .map(|element| Value::type_kind(element).into())
                        .collect::<Vec<_>>()
                        .into(),
                ),
                Value::Object(object) => TypeKind::Object(
                    BTreeMap::from_iter(
                        object
                            .iter()
                            .map(|(key, value)| (key.clone(), Value::type_kind(value).into())),
                    )
                    .into(),
                ),
                Value::Runtime(runtime) => match runtime.as_ref() {
                    Runtime::CompiledRegex(_) => TypeKind::Embedded(Embedded::CompiledRegex),
                    Runtime::BufferizedWriter(_) => TypeKind::Embedded(Embedded::BufferizedWriter),
                    Runtime::BufferizedReader(_) => TypeKind::Embedded(Embedded::BufferizedReader),
                    Runtime::HttpClient(_) => TypeKind::Embedded(Embedded::HttpClient),
                    Runtime::RandomNumberGenerator(_) => TypeKind::Embedded(Embedded::RandomNumberGenerator),
                },
            },
            None => TypeKind::Null,
        }
    }
}
