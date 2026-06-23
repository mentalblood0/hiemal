use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::value::Value;

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, Ord, Eq, Hash, PartialEq)]
pub enum Type {
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "string")]
    String,
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "null")]
    Null,
    #[serde(rename = "array")]
    Array(Box<Type>),
    #[serde(rename = "tuple")]
    Tuple(Vec<Type>),
    #[serde(rename = "object")]
    Object(BTreeMap<String, Type>),
    #[serde(rename = "union")]
    Union(BTreeSet<Type>),
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "literal")]
    Literal(Option<Value>),
    Unknown(usize),
}

impl From<BTreeSet<Type>> for Type {
    fn from(mut union_types: BTreeSet<Type>) -> Self {
        if union_types.contains(&Type::Any) {
            Type::Any
        } else {
            if union_types.contains(&Self::Literal(Some(Value::Bool(true))))
                && union_types.contains(&Self::Literal(Some(Value::Bool(false))))
            {
                union_types.remove(&Self::Literal(Some(Value::Bool(true))));
                union_types.remove(&Self::Literal(Some(Value::Bool(false))));
                union_types.insert(Self::Bool);
            }
            if union_types.contains(&Self::Literal(None)) {
                union_types.remove(&Self::Literal(None));
                union_types.insert(Self::Null);
            }
            match union_types.len() {
                0 => Self::Null,
                1 => union_types.into_iter().next().unwrap(),
                _ => Self::Union(union_types),
            }
        }
    }
}

impl Type {
    pub fn contains(&self, other: &Self) -> bool {
        if self == other {
            true
        } else {
            match (self, other) {
                (Self::Union(self_union_types), Self::Union(other_union_types)) => {
                    for other_union_type in other_union_types {
                        let mut found = false;
                        for self_union_type in self_union_types {
                            if self_union_type == other_union_type {
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            return false;
                        }
                    }
                    true
                }
                (Self::Union(self_union_types), other_type) => {
                    for self_union_type in self_union_types {
                        if self_union_type == other_type {
                            return true;
                        }
                    }
                    false
                }
                (self_type, Self::Union(other_union_types)) => {
                    other_union_types.len() == 1
                        && Some(self_type) == other_union_types.iter().next()
                }
                (Self::Any, _) => true,
                (_, Self::Any) => false,
                (Self::Literal(self_value), Self::Literal(other_value)) => {
                    self_value == other_value
                }
                (self_type, Self::Literal(other_value)) => {
                    self_type.contains(&Value::r#type(other_value))
                }
                _ => false,
            }
        }
    }

    pub fn strongest<'a>(&'a self, other: &'a Self) -> &'a Self {
        if self.contains(other) { other } else { self }
    }
}
