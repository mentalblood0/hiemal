use std::{collections::BTreeMap, str::FromStr};

use anyhow::Result;
use bytes::Bytes;
use dashu::{Decimal, Rational};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Unexpected, Visitor},
};
use std::fmt;

use crate::{
    containers::{Object, Vector},
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    r#type::Type,
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
    use serde::ser::SerializeMap;

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

    pub fn r#type(value_option: &Option<Value>) -> Type {
        match value_option {
            Some(value) => match value {
                Value::Number(_) => Type::Number,
                Value::String(string) => Type::LiteralString(string.clone()),
                Value::Bytes(_) => Type::Bytes,
                Value::Bool(true) => Type::LiteralTrue,
                Value::Bool(false) => Type::LiteralFalse,
                Value::Tuple(tuple) => Type::Tuple(
                    tuple
                        .iter()
                        .map(|element| Value::r#type(element))
                        .collect::<Vec<_>>()
                        .into(),
                ),
                Value::Object(object) => Type::Object(
                    BTreeMap::from_iter(
                        object
                            .iter()
                            .map(|(key, value)| (key.clone(), Value::r#type(value))),
                    )
                    .into(),
                ),
            },
            None => Type::Null,
        }
    }
}
