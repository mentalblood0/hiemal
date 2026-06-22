use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Hash)]
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
    #[serde(rename = "object")]
    Object(BTreeMap<String, Type>),
    #[serde(rename = "union")]
    Union(BTreeSet<Type>),
    #[serde(rename = "any")]
    Any,
    Unknown(usize),
}

impl From<BTreeSet<Type>> for Type {
    fn from(union_types: BTreeSet<Type>) -> Self {
        match union_types.len() {
            0 => Self::Null,
            1 => union_types.into_iter().next().unwrap(),
            _ => Self::Union(union_types),
        }
    }
}
