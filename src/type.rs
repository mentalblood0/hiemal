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
    fn from(mut union_types: BTreeSet<Type>) -> Type {
        if union_types.contains(&Type::Any) {
            Type::Any
        } else {
            if union_types.contains(&Type::Literal(Some(Value::Bool(true))))
                && union_types.contains(&Type::Literal(Some(Value::Bool(false))))
            {
                union_types.remove(&Type::Literal(Some(Value::Bool(true))));
                union_types.remove(&Type::Literal(Some(Value::Bool(false))));
                union_types.insert(Type::Bool);
            }
            if union_types.contains(&Type::Literal(None)) {
                union_types.remove(&Type::Literal(None));
                union_types.insert(Type::Null);
            }
            match union_types.len() {
                0 => Type::Null,
                1 => union_types.into_iter().next().unwrap(),
                _ => Type::Union(union_types),
            }
        }
    }
}

impl Type {
    pub fn contains(&self, other: &Type) -> bool {
        if self == other {
            true
        } else {
            match (self, other) {
                (Type::Union(self_union_types), Type::Union(other_union_types)) => {
                    self_union_types.is_superset(other_union_types)
                }
                (Type::Union(self_union_types), other_type) => {
                    self_union_types.contains(other_type)
                }
                (self_type, Type::Union(other_union_types)) => {
                    other_union_types.len() == 1
                        && Some(self_type) == other_union_types.iter().next()
                }
                (Type::Any, _) => true,
                (_, Type::Any) => false,
                (Type::Literal(self_value), Type::Literal(other_value)) => {
                    self_value == other_value
                }
                (self_type, Type::Literal(other_value)) => {
                    self_type.contains(&Value::r#type(other_value))
                }
                _ => false,
            }
        }
    }

    pub fn intersection(&self, other: &Type) -> Option<Type> {
        if self == other {
            Some(self.clone())
        } else {
            match (self, other) {
                // {literal: {bool: true}}, number
                // bool, string
                (Type::Union(self_union_types), Type::Union(other_union_types)) => {
                    let result = self_union_types
                        .intersection(other_union_types)
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    if result.is_empty() {
                        None
                    } else {
                        Some(Type::from(result))
                    }
                }
                (Type::Union(self_union_types), other_type) => {
                    if self_union_types.contains(other_type) {
                        Some(other_type.clone())
                    } else {
                        None
                    }
                }
                (self_type, Type::Union(other_union_types)) => {
                    if other_union_types.len() == 1
                        && Some(self_type) == other_union_types.iter().next()
                    {
                        Some(self_type.clone())
                    } else {
                        None
                    }
                }
                (Type::Any, other_type) => Some(other_type.clone()),
                (self_type, Type::Any) => Some(self_type.clone()),
                (Type::Literal(Value::Bool(_)), other_type) => self.clone()
                _ => None,
            }
        }
    }
}
