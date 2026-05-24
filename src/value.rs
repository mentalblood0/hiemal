use anyhow::Result;
use dashu::{Decimal, Rational};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, Unexpected, Visitor},
};
use std::str::FromStr;

use crate::default_argument_name::DEFAULT_ARGUMENT_NAME;

pub type SmallMap<K, V> = small_map::FxSmallMap<32, K, V>;

#[derive(Deserialize, Serialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum Value {
    #[serde(deserialize_with = "deserialize_rational")]
    Number(Rational),
    String(String),
    Bool(bool),
    Null,
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

    pub fn as_string(&self) -> Option<&String> {
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
}
