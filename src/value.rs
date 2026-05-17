use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use dashu::{Decimal, Rational};
use serde::{
    Deserializer,
    de::{self, Unexpected, Visitor},
};
use std::str::FromStr;
use url::Url;

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
            if s == "_" {
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
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct ExtendedDefinition {
    pub access: Rc<BTreeSet<String>>,
    pub compute: RcOrValue,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(untagged)]
pub enum Definition {
    Extended(ExtendedDefinition),
    Default(RcOrValue),
}

impl Definition {
    pub fn rc_or_value(&self) -> &RcOrValue {
        match self {
            Definition::Extended(extended_definition) => &extended_definition.compute,
            Definition::Default(default_definition) => default_definition,
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct With {
    #[serde(default)]
    pub definitions: BTreeMap<String, Definition>,
    #[serde(default)]
    pub constants: BTreeMap<String, RcOrValue>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct WithCompute {
    pub with: With,
    pub compute: RcOrValue,
}

fn default_alias() -> String {
    "_".to_string()
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Map {
    pub map: RcOrValue,
    #[serde(default = "default_alias")]
    pub as_alias: String,
    pub through: RcOrValue,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Filter {
    pub filter: RcOrValue,
    #[serde(default = "default_alias")]
    pub as_alias: String,
    pub through: RcOrValue,
}

fn default_current_value_alias() -> String {
    "current".to_string()
}

fn default_accumulator_value_alias() -> String {
    "accumulator".to_string()
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Fold {
    pub fold: RcOrValue,
    #[serde(default = "default_current_value_alias")]
    pub as_alias: String,
    pub starting_with: RcOrValue,
    #[serde(default = "default_accumulator_value_alias")]
    pub accumulating_in_alias: String,
    pub through: RcOrValue,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Branching {
    pub r#if: RcOrValue,
    pub then: RcOrValue,
    pub r#else: RcOrValue,
}

fn default_error_alias() -> String {
    "error".to_string()
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct TryOr {
    pub r#try: RcOrValue,
    pub or: RcOrValue,
    #[serde(default = "default_error_alias")]
    pub with_error_alias: String,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum AtSegment {
    ObjectKey(String),
    ArrayIndex(usize),
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct FromAt {
    pub from: RcOrValue,
    pub at: Vec<AtSegment>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum Value {
    #[serde(deserialize_with = "deserialize_rational")]
    Number(Rational),
    String(String),
    Bool(bool),
    Null,
    Array(Vec<RcOrValue>),
    With(Box<WithCompute>),
    Map(Box<Map>),
    Filter(Box<Filter>),
    Fold(Box<Fold>),
    Branching(Box<Branching>),
    TryOr(Box<TryOr>),
    FromAt(Box<FromAt>),
    Object(BTreeMap<String, RcOrValue>),
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Include {
    IncludeUrl(Url),
    IncludeFile(std::path::PathBuf),
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum ValueWithIncludes {
    Include(Include),
    Array(Vec<ValueWithIncludes>),
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

    pub fn as_array(&self) -> Option<&Vec<RcOrValue>> {
        match self {
            Value::Array(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, RcOrValue>> {
        match self {
            Value::Object(result) => Some(result),
            _ => None,
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
#[serde(untagged)]
pub enum RcOrValue {
    Rc(Rc<Value>),
    Value(Value),
}

impl Clone for RcOrValue {
    fn clone(&self) -> Self {
        match self {
            RcOrValue::Rc(rc) => RcOrValue::Rc(rc.clone()),
            RcOrValue::Value(value) => match value {
                Value::Number(_) | Value::String(_) | Value::Bool(_) | Value::Null => {
                    RcOrValue::Value(value.clone())
                }
                complex_value => RcOrValue::Rc(Rc::new(complex_value.clone())),
            },
        }
    }
}

impl PartialEq for RcOrValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (RcOrValue::Rc(a), RcOrValue::Rc(b)) => *a == *b,
            (RcOrValue::Value(a), RcOrValue::Value(b)) => a == b,
            (RcOrValue::Rc(rc_val), RcOrValue::Value(val)) => **rc_val == *val,
            (RcOrValue::Value(val), RcOrValue::Rc(rc_val)) => *val == **rc_val,
        }
    }
}

impl RcOrValue {
    pub fn borrow_rc_if_complex_otherwise_value(self) -> Self {
        match self {
            RcOrValue::Rc(rc) => RcOrValue::Rc(rc),
            RcOrValue::Value(value) => match value {
                Value::Number(_) | Value::String(_) | Value::Bool(_) | Value::Null => {
                    RcOrValue::Value(value)
                }
                complex_value => RcOrValue::Rc(Rc::new(complex_value)),
            },
        }
    }

    pub fn value(&self) -> &Value {
        match self {
            RcOrValue::Rc(rc) => rc,
            RcOrValue::Value(value) => value,
        }
    }

    pub fn rc(&self) -> Rc<Value> {
        match self {
            RcOrValue::Rc(rc) => rc.clone(),
            RcOrValue::Value(value) => Rc::new(value.clone()),
        }
    }

    pub fn as_number(&self) -> Option<Rational> {
        match self {
            RcOrValue::Rc(rc) => rc.as_number(),
            RcOrValue::Value(value) => value.as_number(),
        }
    }

    pub fn as_string(&self) -> Option<&String> {
        match self {
            RcOrValue::Rc(rc) => rc.as_string(),
            RcOrValue::Value(value) => value.as_string(),
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            RcOrValue::Rc(rc) => rc.as_bool(),
            RcOrValue::Value(value) => value.as_bool(),
        }
    }

    pub fn as_array(&self) -> Option<&Vec<RcOrValue>> {
        match self {
            RcOrValue::Rc(rc) => rc.as_array(),
            RcOrValue::Value(value) => value.as_array(),
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, RcOrValue>> {
        match self {
            RcOrValue::Rc(rc) => rc.as_object(),
            RcOrValue::Value(value) => value.as_object(),
        }
    }
}
