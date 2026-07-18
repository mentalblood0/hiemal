use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize, Serializer};

#[derive(Debug, Clone)]
pub struct MaybeType {
    pub lockable_internals: Arc<RwLock<Option<Type>>>,
}

impl Default for MaybeType {
    fn default() -> Self {
        Self {
            lockable_internals: Arc::new(RwLock::new(None)),
        }
    }
}

impl PartialEq for MaybeType {
    fn eq(&self, other: &Self) -> bool {
        *self.lockable_internals.read() == *other.lockable_internals.read()
    }
}

impl Eq for MaybeType {}

impl PartialOrd for MaybeType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MaybeType {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.lockable_internals
            .read()
            .cmp(&*other.lockable_internals.read())
    }
}

impl Hash for MaybeType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.lockable_internals.read().hash(state);
    }
}

#[repr(u8)]
#[derive(Deserialize, Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    Object(BTreeMap<Arc<String>, Type>),
    #[serde(rename = "union")]
    Union(BTreeSet<Type>),
    #[default]
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "literal true")]
    LiteralTrue,
    #[serde(rename = "literal false")]
    LiteralFalse,
    #[serde(skip_deserializing)]
    Unknown(MaybeType),
}

#[repr(u8)]
#[derive(Serialize, Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KnownType {
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
    Object(BTreeMap<Arc<String>, Type>),
    #[serde(rename = "union")]
    Union(BTreeSet<Type>),
    #[default]
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "literal true")]
    LiteralTrue,
    #[serde(rename = "literal false")]
    LiteralFalse,
}

impl Serialize for Type {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Type::Unknown(maybe_type) => match &*maybe_type.lockable_internals.read() {
                Some(r#type) => r#type.serialize(serializer),
                None => Result::Err(serde::ser::Error::custom(
                    "unknown type have not been resolved",
                )),
            },
            known_type => unsafe {
                std::mem::transmute::<&Type, &KnownType>(known_type).serialize(serializer)
            },
        }
    }
}

impl From<BTreeSet<Type>> for Type {
    fn from(mut union_types: BTreeSet<Type>) -> Type {
        if union_types.contains(&Type::Any) {
            Type::Any
        } else {
            if union_types.contains(&Type::LiteralTrue) && union_types.contains(&Type::LiteralFalse)
            {
                union_types.remove(&Type::LiteralTrue);
                union_types.remove(&Type::LiteralFalse);
                union_types.insert(Type::Bool);
            }
            match union_types.len() {
                0 => Type::Null,
                1 => union_types.into_iter().next().unwrap(),
                _ => Type::Union(union_types),
            }
        }
    }
}

impl<'a> Type {
    pub fn is_known(&self) -> bool {
        match self {
            Type::Unknown(_) => false,
            Type::Array(element_type) => element_type.is_known(),
            Type::Tuple(elements_types) => elements_types
                .iter()
                .all(|element_type| element_type.is_known()),
            Type::Object(inner_types) => {
                inner_types.values().all(|value_type| value_type.is_known())
            }
            Type::Union(union_types) => union_types.iter().all(|union_type| union_type.is_known()),
            _ => true,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<Arc<String>, Type>> {
        match self {
            Type::Object(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_tuple_mut(&'a mut self) -> Option<&'a mut Vec<Type>> {
        match self {
            Type::Tuple(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_union_mut(&'a mut self) -> Option<&'a mut BTreeSet<Type>> {
        match self {
            Type::Union(result) => Some(result),
            _ => None,
        }
    }

    pub fn contains(&self, other: &Type) -> bool {
        if self == other {
            true
        } else {
            match (self, other) {
                (Type::Any, _) => true,
                (_, Type::Any) => false,
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
                (Type::Bool, Type::LiteralTrue | Type::LiteralFalse) => true,
                (
                    Type::Tuple(self_tuple_elements_types),
                    Type::Tuple(other_tuple_elements_types),
                ) => {
                    if self_tuple_elements_types.len() != other_tuple_elements_types.len() {
                        return false;
                    }
                    for element_index in 0..self_tuple_elements_types.len() {
                        if !self_tuple_elements_types[element_index]
                            .contains(&other_tuple_elements_types[element_index])
                        {
                            return false;
                        }
                    }
                    true
                }
                (Type::Array(self_array_element_type), Type::Tuple(other_tuple_elements_types)) => {
                    for other_tuple_element_type in other_tuple_elements_types {
                        if !self_array_element_type.contains(other_tuple_element_type) {
                            return false;
                        }
                    }
                    true
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
                (Type::Any, other_type) | (other_type, Type::Any) => Some(other_type.clone()),
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
                (Type::Union(union_types), other_type) | (other_type, Type::Union(union_types)) => {
                    for union_type in union_types {
                        if let Some(result) = union_type.intersection(other_type) {
                            return Some(result);
                        }
                    }
                    None
                }
                (Type::Bool, Type::LiteralTrue) | (Type::LiteralTrue, Type::Bool) => {
                    Some(Type::LiteralTrue)
                }
                (Type::Bool, Type::LiteralFalse) | (Type::LiteralFalse, Type::Bool) => {
                    Some(Type::LiteralFalse)
                }
                (Type::Array(self_array_element_type), Type::Array(other_array_element_type)) => {
                    self_array_element_type
                        .intersection(other_array_element_type)
                        .map(|element_types_intersection| {
                            Type::Array(Box::new(element_types_intersection))
                        })
                }
                (
                    Type::Tuple(self_tuple_elements_types),
                    Type::Tuple(other_tuple_elements_types),
                ) => {
                    if self_tuple_elements_types.len() != other_tuple_elements_types.len() {
                        None
                    } else {
                        let mut result_tuple_types =
                            Vec::with_capacity(self_tuple_elements_types.len());
                        for element_index in 0..self_tuple_elements_types.len() {
                            if let Some(elements_types_intersection) = self_tuple_elements_types
                                [element_index]
                                .intersection(&other_tuple_elements_types[element_index])
                            {
                                result_tuple_types.push(elements_types_intersection);
                            } else {
                                return None;
                            }
                        }
                        Some(Type::Tuple(result_tuple_types))
                    }
                }
                (Type::Array(array_element_type), Type::Tuple(tuple_elements_types))
                | (Type::Tuple(tuple_elements_types), Type::Array(array_element_type)) => {
                    let mut result_tuple_elements_types =
                        Vec::with_capacity(tuple_elements_types.len());
                    for tuple_element_type in tuple_elements_types {
                        if let Some(result_tuple_element_type) =
                            tuple_element_type.intersection(array_element_type)
                        {
                            result_tuple_elements_types.push(result_tuple_element_type)
                        } else {
                            return None;
                        }
                    }
                    Some(Type::Tuple(tuple_elements_types.clone()))
                }
                _ => None,
            }
        }
    }

    pub fn weakest_from_union(&self) -> Type {
        match self {
            Type::Union(union_types) => {
                let mut result = union_types.iter().next().unwrap().clone();
                for union_type in union_types.iter().skip(1) {
                    if union_type.contains(&result) {
                        result = union_type.clone();
                    }
                }
                result
            }
            r#type => r#type.clone(),
        }
    }
}
