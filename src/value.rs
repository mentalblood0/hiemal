use std::{
    collections::BTreeMap,
    fmt,
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
    ser::SerializeMap,
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
}

#[repr(u8)]
#[derive(Deserialize, Serialize, PartialEq, Debug, Clone, Eq, Hash, PartialOrd, Ord)]
#[serde(untagged)]
pub enum Value {
    #[serde(deserialize_with = "deserialize_rational")]
    Number(Rational),
    #[serde(
        deserialize_with = "deserialize_rope",
        serialize_with = "serialize_rope"
    )]
    String(ropey::Rope),
    Bool(bool),
    Tuple(Vector<Option<Value>>),
    Bytes(
        #[serde(
            deserialize_with = "deserialize_bytes",
            serialize_with = "serialize_bytes"
        )]
        Bytes,
    ),
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
    let hex_string = hex::encode(bytes);
    let mut map = serializer.serialize_map(Some(1))?;
    map.serialize_entry("bytes", &hex_string)?;
    map.end()
}

pub fn deserialize_bytes<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{Error, MapAccess, Visitor};

    struct BytesVisitor;

    impl<'de> Visitor<'de> for BytesVisitor {
        type Value = Bytes;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map with a 'bytes' key containing a hex string")
        }

        fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut hex_string: Option<String> = None;
            while let Some(key) = access.next_key::<String>()? {
                if key == "bytes" {
                    if hex_string.is_some() {
                        return Err(M::Error::duplicate_field("bytes"));
                    }
                    hex_string = Some(access.next_value()?);
                } else {
                    let _: serde::de::IgnoredAny = access.next_value()?;
                }
            }
            let hex_string = hex_string.ok_or_else(|| M::Error::missing_field("bytes"))?;
            let bytes = hex::decode(&hex_string)
                .map_err(|_| M::Error::custom("invalid hex string"))?
                .into();
            Ok(bytes)
        }
    }

    deserializer.deserialize_map(BytesVisitor)
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

    pub fn as_string(&self) -> Option<&ropey::Rope> {
        match self {
            Value::String(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&Bytes> {
        match self {
            Value::Bytes(result) => Some(result),
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
                Value::Bytes(_) => TypeKind::Bytes,
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
                },
            },
            None => TypeKind::Null,
        }
    }
}
