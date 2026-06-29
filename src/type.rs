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
            let non_literal_union_types = union_types
                .iter()
                .filter(|r#type| {
                    if let Type::Literal(_) = r#type {
                        false
                    } else {
                        true
                    }
                })
                .cloned()
                .collect::<BTreeSet<_>>();
            union_types = union_types
                .into_iter()
                .filter(|r#type| {
                    if let Type::Literal(type_value) = r#type
                        && non_literal_union_types.contains(&Value::r#type(type_value))
                    {
                        false
                    } else {
                        true
                    }
                })
                .collect::<BTreeSet<_>>();
            match union_types.len() {
                0 => Type::Null,
                1 => union_types.into_iter().next().unwrap(),
                _ => Type::Union(union_types),
            }
        }
    }
}

impl<'a> Type {
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
                (Type::Literal(self_value), Type::Literal(other_value)) => {
                    self_value == other_value
                }
                (self_type, Type::Literal(other_value)) => {
                    self_type.contains(&Value::r#type(other_value))
                }
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

    pub fn unliteral(self) -> Type {
        match self {
            Type::Literal(type_value) => Value::r#type(&type_value),
            _ => self,
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
                    if union_types.contains(other_type) {
                        Some(other_type.clone())
                    } else {
                        None
                    }
                }
                (Type::Literal(self_type_value), Type::Literal(other_type_value)) => {
                    if self_type_value == other_type_value {
                        Some(self.clone())
                    } else {
                        None
                    }
                }
                (Type::Literal(type_value), other_type)
                | (other_type, Type::Literal(type_value)) => {
                    if &Value::r#type(type_value) == other_type {
                        Some(Type::Literal(type_value.clone()))
                    } else {
                        None
                    }
                }
                (Type::Array(self_array_element_type), Type::Array(other_array_element_type)) => {
                    if let Some(element_types_intersection) =
                        self_array_element_type.intersection(other_array_element_type)
                    {
                        Some(Type::Array(Box::new(element_types_intersection)))
                    } else {
                        None
                    }
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
