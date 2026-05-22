use std::collections::{BTreeMap, VecDeque};

use dashu::{Decimal, Rational};
use serde::{
    Deserialize, Deserializer,
    de::{self, Unexpected, Visitor},
};
use std::str::FromStr;
use url::Url;

use crate::default_argument_name::DEFAULT_ARGUMENT_NAME;

pub type SmallMap<K, V> = small_map::FxSmallMap<32, K, V>;

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

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct Constant {
    pub constant: String,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct With {
    #[serde(default)]
    pub functions: rpds::RedBlackTreeMapSync<String, Value>,
    #[serde(default)]
    pub constants: rpds::RedBlackTreeMapSync<String, Value>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct WithCompute {
    pub with: With,
    pub compute: Value,
}

fn default_alias() -> String {
    DEFAULT_ARGUMENT_NAME.to_string()
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct Map {
    pub map: Value,
    #[serde(default = "default_alias")]
    pub r#as: String,
    pub through: Value,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct Filter {
    pub filter: Value,
    #[serde(default = "default_alias")]
    pub r#as: String,
    pub through: Value,
}

fn default_current_value_alias() -> String {
    "current".to_string()
}

fn default_accumulator_value_alias() -> String {
    "accumulator".to_string()
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct Fold {
    pub fold: Value,
    #[serde(default = "default_current_value_alias")]
    pub r#as: String,
    #[serde(rename = "starting with")]
    pub starting_with: Value,
    #[serde(
        rename = "accumulating in",
        default = "default_accumulator_value_alias"
    )]
    pub accumulating_in: String,
    pub through: Value,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct Branching {
    pub r#if: Value,
    pub then: Value,
    pub r#else: Value,
}

fn default_error_alias() -> String {
    "error".to_string()
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct TryOr {
    pub r#try: Value,
    pub or: Value,
    #[serde(rename = "with error", default = "default_error_alias")]
    pub with_error: String,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum AtSegment {
    ObjectKey(String),
    ArrayIndex(usize),
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct FromAt {
    pub from: Value,
    pub at: rpds::ListSync<AtSegment>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum Value {
    #[serde(deserialize_with = "deserialize_rational")]
    Number(Rational),
    String(String),
    Bool(bool),
    Null,
    Array(rpds::VectorSync<Value>),
    Constant(Constant),
    With(Box<WithCompute>),
    Map(Box<Map>),
    Filter(Box<Filter>),
    Fold(Box<Fold>),
    Branching(Box<Branching>),
    TryOr(Box<TryOr>),
    FromAt(Box<FromAt>),
    Object(rpds::RedBlackTreeMapSync<String, Value>),
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum IncludeFrom {
    Url(Url),
    File(std::path::PathBuf),
}

#[derive(PartialEq, Debug, Clone)]
pub struct IncludeFromAt {
    pub from: IncludeFrom,
    pub at: Vec<AtSegment>,
}

impl<'de> Deserialize<'de> for IncludeFromAt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut values = VecDeque::deserialize(deserializer)?;
        let from = if let Some(first_value) = values.pop_front() {
            serde_json::from_value(first_value).map_err(serde::de::Error::custom)?
        } else {
            return Err(serde::de::Error::invalid_length(
                0,
                &"at least one element (url or file path)",
            ));
        };
        let at: Vec<AtSegment> = values
            .into_iter()
            .map(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(IncludeFromAt { from, at })
    }
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub struct Include {
    #[serde(deserialize_with = "IncludeFromAt::deserialize")]
    pub include: IncludeFromAt,
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum ValueWithIncludes {
    Array(Vec<ValueWithIncludes>),
    Include(Include),
    Object(BTreeMap<String, ValueWithIncludes>),
    Other(serde_json::Value),
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

    pub fn as_array(&self) -> Option<&rpds::VectorSync<Value>> {
        match self {
            Value::Array(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&rpds::RedBlackTreeMapSync<String, Value>> {
        match self {
            Value::Object(result) => Some(result),
            _ => None,
        }
    }
}
