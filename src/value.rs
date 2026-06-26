use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

use anyhow::Result;
use dashu::{Decimal, Rational};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Unexpected, Visitor},
};

use crate::{
    containers::{Map, Vector},
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
#[derive(Deserialize, Serialize, PartialEq, Debug, Clone, PartialOrd, Eq, Ord, Hash)]
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
    Object(Map<String, Option<Value>>),
}

impl Default for Value {
    fn default() -> Self {
        Value::Bool(false)
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
            } else {
                if let Ok(result) = Rational::from_str(s) {
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

    pub fn as_array_mut(&mut self) -> Option<&mut Vector<Option<Value>>> {
        match self {
            Value::Tuple(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Map<String, Option<Value>>> {
        match self {
            Value::Object(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Map<String, Option<Value>>> {
        match self {
            Value::Object(result) => Some(result),
            _ => None,
        }
    }

    pub fn r#type(value_option: &Option<Value>) -> Type {
        match value_option {
            Some(value) => match value {
                Value::Number(_) => Type::Number,
                Value::String(_) => Type::String,
                Value::Bool(_) => Type::Bool,
                Value::Tuple(array) => {
                    let elements_types = BTreeSet::from_iter(array.inner.iter().map(Value::r#type));
                    Type::Array(Box::new(match elements_types.len() {
                        0 => Type::Any,
                        1 => elements_types.into_iter().next().unwrap(),
                        _ => Type::Union(elements_types),
                    }))
                }
                Value::Object(object) => Type::Object(BTreeMap::from_iter(
                    object
                        .inner
                        .iter()
                        .map(|(key, value)| (key.clone(), Value::r#type(value))),
                )),
            },
            None => Type::Null,
        }
    }
}
