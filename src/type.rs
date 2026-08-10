use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    sync::{Arc, LazyLock},
};

use anyhow::{Result, anyhow};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize, Serializer};

use crate::{
    intermediate_representation::RangeBound,
    intermediate_representation::ValuePathSegment,
    value::{deserialize_rope, serialize_rope},
};

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

static CONSTRUCTED_TYPES: LazyLock<[Type; 4]> = LazyLock::new(|| {
    [
        Type::Array(Box::new(Type::Union(BTreeSet::from_iter([
            Type::String,
            Type::Object(BTreeMap::from_iter([(
                "raw string".to_string().into(),
                Type::String,
            )])),
            Type::Constructed(Constructed::Or),
            Type::Constructed(Constructed::Repeat),
            Type::Constructed(Constructed::Group),
            Type::LiteralString("character".into()),
            Type::LiteralString("whitespace character".into()),
            Type::LiteralString("non-whitespace character".into()),
            Type::LiteralString("digit".into()),
            Type::LiteralString("non-digit".into()),
            Type::LiteralString("word character".into()),
            Type::LiteralString("non-word character".into()),
            Type::LiteralString("start of string".into()),
            Type::LiteralString("end of string".into()),
            Type::LiteralString("word boundary".into()),
            Type::LiteralString("non-word boundary".into()),
        ])))),
        Type::Object(BTreeMap::from_iter([(
            "or".to_string().into(),
            Type::Array(Box::new(Type::Constructed(Constructed::Regex))),
        )])),
        Type::Union(BTreeSet::from_iter([
            Type::Object(BTreeMap::from_iter([
                (
                    "repeat".to_string().into(),
                    Type::Constructed(Constructed::Regex),
                ),
                ("min".to_string().into(), Type::Number),
                ("max".to_string().into(), Type::Number),
            ])),
            Type::Object(BTreeMap::from_iter([
                (
                    "repeat".to_string().into(),
                    Type::Constructed(Constructed::Regex),
                ),
                ("min".to_string().into(), Type::Number),
            ])),
            Type::Object(BTreeMap::from_iter([
                (
                    "repeat".to_string().into(),
                    Type::Constructed(Constructed::Regex),
                ),
                ("max".to_string().into(), Type::Number),
            ])),
            Type::Object(BTreeMap::from_iter([
                (
                    "repeat".to_string().into(),
                    Type::Constructed(Constructed::Regex),
                ),
                ("exactly".to_string().into(), Type::Number),
            ])),
        ])),
        Type::Object(BTreeMap::from_iter([
            (
                "group".to_string().into(),
                Type::Constructed(Constructed::Regex),
            ),
            ("name".to_string().into(), Type::String),
        ])),
    ]
});

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Constructed {
    #[default]
    Regex,
    Or,
    Repeat,
    Group,
}

impl Constructed {
    pub fn inner(&self) -> &Type {
        CONSTRUCTED_TYPES.get(self.clone() as u8 as usize).unwrap()
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
    #[serde(rename = "generic object")]
    GenericObject(Box<Type>),
    #[serde(rename = "union")]
    Union(BTreeSet<Type>),
    #[default]
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "literal true")]
    LiteralTrue,
    #[serde(rename = "literal false")]
    LiteralFalse,
    #[serde(rename = "literal string")]
    LiteralString(#[serde(deserialize_with = "deserialize_rope")] ropey::Rope),
    #[serde(rename = "constructed")]
    Constructed(Constructed),
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
    #[serde(rename = "generic object")]
    GenericObject(Box<Type>),
    #[serde(rename = "union")]
    Union(BTreeSet<Type>),
    #[default]
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "literal true")]
    LiteralTrue,
    #[serde(rename = "literal false")]
    LiteralFalse,
    #[serde(rename = "literal string")]
    LiteralString(
        #[serde(
            deserialize_with = "deserialize_rope",
            serialize_with = "serialize_rope"
        )]
        ropey::Rope,
    ),
    #[serde(rename = "constructed")]
    Constructed(Constructed),
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
                1 => union_types.into_iter().next().unwrap(),
                _ => {
                    if union_types
                        .iter()
                        .all(|union_type| !matches!(union_type, Type::Union(_)))
                    {
                        Type::Union(union_types)
                    } else {
                        let mut result_union_types = BTreeSet::new();
                        for union_type in union_types.into_iter() {
                            match union_type {
                                Type::Union(mut inner_union_types) => {
                                    result_union_types.append(&mut inner_union_types)
                                }
                                non_union_type => {
                                    result_union_types.insert(non_union_type);
                                }
                            }
                        }
                        Type::Union(result_union_types)
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum TypeAtResult {
    Single(Type),
    Multiple(BTreeSet<Type>),
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
            Type::GenericObject(value_type) => value_type.is_known(),
            Type::Union(union_types) => union_types.iter().all(|union_type| union_type.is_known()),
            Type::Constructed(constructed) => constructed.inner().is_known(),
            _ => true,
        }
    }

    pub fn is_concrete(&self) -> bool {
        match self {
            Type::Array(element_type) => element_type.is_concrete(),
            Type::Tuple(elements_types) => elements_types
                .iter()
                .all(|element_type| element_type.is_concrete()),
            Type::Object(inner_types) => inner_types
                .values()
                .all(|value_type| value_type.is_concrete()),
            Type::GenericObject(value_type) => value_type.is_concrete(),
            Type::Union(_) | Type::Any | Type::Unknown(_) => false,
            Type::Constructed(constructed) => constructed.inner().is_concrete(),
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
                (Type::Any, _) | (Type::String, Type::LiteralString(_)) => true,
                (Type::Constructed(self_constructed), Type::Constructed(other_constructed)) => {
                    self_constructed.inner().contains(other_constructed.inner())
                }
                (Type::Constructed(self_constructed), _) => {
                    self_constructed.inner().contains(other)
                }
                (_, Type::Constructed(other_constructed)) => {
                    self.contains(other_constructed.inner())
                }
                (Type::Union(self_union_types), Type::Union(other_union_types)) => {
                    if self_union_types.is_superset(other_union_types) {
                        true
                    } else {
                        for other_union_type in other_union_types {
                            let mut found_container = false;
                            for self_union_type in self_union_types {
                                if self_union_type.contains(other_union_type) {
                                    found_container = true;
                                    break;
                                }
                            }
                            if !found_container {
                                return false;
                            }
                        }
                        true
                    }
                }
                (Type::Union(self_union_types), other_type) => {
                    if self_union_types.contains(other_type) {
                        true
                    } else {
                        for self_union_type in self_union_types {
                            if self_union_type.contains(other_type) {
                                return true;
                            }
                        }
                        false
                    }
                }
                (self_type, Type::Union(other_union_types)) => other_union_types
                    .iter()
                    .all(|other_union_type| self_type.contains(other_union_type)),
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

                (Type::Object(self_inner_types), Type::Object(other_inner_types)) => {
                    for ((self_key, self_value_type), (other_key, other_value_type)) in
                        self_inner_types.iter().zip(other_inner_types.iter())
                    {
                        if self_key != other_key {
                            return false;
                        }
                        if !self_value_type.contains(other_value_type) {
                            return false;
                        }
                    }
                    true
                }
                (Type::Object(self_inner_types), Type::GenericObject(other_value_type)) => {
                    self_inner_types
                        .values()
                        .all(|self_value_type| self_value_type.contains(other_value_type))
                }
                (Type::GenericObject(self_value_type), Type::Object(other_inner_types)) => {
                    other_inner_types
                        .values()
                        .all(|other_value_type| self_value_type.contains(other_value_type))
                }
                (Type::GenericObject(self_value_type), Type::GenericObject(other_value_type)) => {
                    self_value_type.contains(other_value_type)
                }
                _ => false,
            }
        }
    }

    pub fn flatten(&self) -> Result<Type> {
        match self {
            Type::Constructed(constructed) => constructed.inner().flatten(),
            Type::Union(self_union_types) => {
                let mut result_union_types = BTreeSet::new();
                for self_union_type in self_union_types {
                    result_union_types.insert(self_union_type.flatten()?);
                }
                Ok(Type::from(result_union_types))
            }
            Type::Array(element_type) => match &**element_type {
                Type::Array(element_element_type) => Ok(Type::Array(element_element_type.clone())),
                Type::Tuple(element_elements_types) => Ok(Type::Array(Box::new(Type::Union(
                    BTreeSet::from_iter(element_elements_types.iter().cloned()),
                )))),
                Type::Union(element_union_types) => {
                    let mut result_element_union_types = BTreeSet::new();
                    for element_union_type in element_union_types {
                        match element_union_type {
                            Type::Array(element_union_element_type) => {
                                result_element_union_types
                                    .insert(*element_union_element_type.clone());
                            }
                            Type::Tuple(element_union_elements_types) => {
                                for element_union_element_type in element_union_elements_types {
                                    result_element_union_types
                                        .insert(element_union_element_type.clone());
                                }
                            }
                            non_sequence_type => {
                                return Err(anyhow!(
                                    "can not flatten {self:#?} because it may contain element of \
                                     type {non_sequence_type:#?}"
                                ));
                            }
                        }
                    }
                    Ok(Type::Array(Box::new(Type::from(
                        result_element_union_types,
                    ))))
                }
                non_sequence_type => Err(anyhow!(
                    "can not flatten {self:#?} because it contains element of type \
                     {non_sequence_type:#?}"
                )),
            },
            Type::Tuple(elements_types) => {
                if elements_types
                    .iter()
                    .all(|element_type| matches!(element_type, Type::Tuple(_)))
                {
                    let mut result_elements_types = Vec::new();
                    for element_type in elements_types {
                        match element_type {
                            Type::Tuple(element_elements_types) => {
                                for element_element_type in element_elements_types {
                                    result_elements_types.push(element_element_type.clone());
                                }
                            }
                            _ => panic!(),
                        }
                    }
                    Ok(Type::Tuple(result_elements_types))
                } else {
                    let mut result_elements_types = BTreeSet::new();
                    for element_type in elements_types {
                        match element_type {
                            Type::Array(element_element_type) => {
                                result_elements_types.insert(*element_element_type.clone());
                            }
                            Type::Tuple(element_elements_types) => {
                                for element_element_type in element_elements_types {
                                    result_elements_types.insert(element_element_type.clone());
                                }
                            }
                            Type::Union(element_union_types) => {
                                for element_union_type in element_union_types {
                                    match element_union_type {
                                        Type::Array(element_union_element_type) => {
                                            result_elements_types
                                                .insert(*element_union_element_type.clone());
                                        }
                                        Type::Tuple(element_union_elements_types) => {
                                            for element_union_element_type in
                                                element_union_elements_types
                                            {
                                                result_elements_types
                                                    .insert(element_union_element_type.clone());
                                            }
                                        }
                                        non_sequence_type => {
                                            return Err(anyhow!(
                                                "can not flatten {self:#?} because it may contain \
                                                 element of type {non_sequence_type:#?}"
                                            ));
                                        }
                                    }
                                }
                            }
                            non_sequence_type => {
                                return Err(anyhow!(
                                    "can not flatten {self:#?} because it contains element of \
                                     type {non_sequence_type:#?}"
                                ));
                            }
                        }
                    }
                    Ok(Type::Array(Box::new(Type::from(result_elements_types))))
                }
            }
            _ => Err(anyhow!("can not flatten {self:#?}")),
        }
    }

    pub fn at(self, at_segment: &ValuePathSegment) -> Result<TypeAtResult> {
        match (self, at_segment) {
            (Type::Constructed(constructed), _) => constructed.inner().clone().at(at_segment),
            (Type::Union(union_types), _) => {
                let mut result_types = BTreeSet::new();
                for union_type in union_types {
                    match union_type.at(at_segment)? {
                        TypeAtResult::Single(result_type) => {
                            result_types.insert(result_type);
                        }
                        TypeAtResult::Multiple(ref mut result_types_part) => {
                            result_types.append(result_types_part);
                        }
                    }
                }
                Ok(TypeAtResult::Multiple(result_types))
            }
            (Type::Array(element_type), ValuePathSegment::ArrayIndex(_)) => {
                Ok(TypeAtResult::Single(*element_type))
            }
            (Type::Tuple(mut elements_types), ValuePathSegment::ArrayIndex(tuple_index)) => {
                if *tuple_index >= elements_types.len() {
                    let elements_types_len = elements_types.len();
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?} because there is only {} \
                         elements",
                        Type::Tuple(elements_types),
                        elements_types_len
                    ));
                }
                Ok(TypeAtResult::Single(elements_types.remove(*tuple_index)))
            }
            (Type::Array(element_type), ValuePathSegment::ArrayRange { from, to }) => {
                if let (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) =
                    (&**from, &**to)
                    && from > to
                {
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?}",
                        Type::Array(element_type)
                    ));
                }
                if let (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) =
                    (&**from, &**to)
                {
                    Ok(TypeAtResult::Single(Type::Tuple(vec![
                        *element_type;
                        to - from
                    ])))
                } else {
                    Ok(TypeAtResult::Single(Type::Array(element_type)))
                }
            }
            (Type::Tuple(elements_types), ValuePathSegment::ArrayRange { from, to }) => {
                if let (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) =
                    (&**from, &**to)
                    && from > to
                {
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?}",
                        Type::Tuple(elements_types)
                    ));
                }
                if let RangeBound::Static(Some(from)) = &**from
                    && from >= &elements_types.len()
                {
                    let elements_types_len = elements_types.len();
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?} because {from} >= {}",
                        Type::Tuple(elements_types),
                        elements_types_len
                    ));
                }
                if let RangeBound::Static(Some(to)) = &**to
                    && to > &elements_types.len()
                {
                    let elements_types_len = elements_types.len();
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?} because {to} > {}",
                        Type::Tuple(elements_types),
                        elements_types_len
                    ));
                }
                match (&**from, &**to) {
                    (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) => {
                        Ok(TypeAtResult::Single(Type::Tuple(Vec::from_iter(
                            elements_types.into_iter().skip(*from).take(to - from),
                        ))))
                    }
                    (RangeBound::Static(Some(from)), RangeBound::Static(None)) => {
                        Ok(TypeAtResult::Single(Type::Tuple(Vec::from_iter(
                            elements_types.into_iter().skip(*from),
                        ))))
                    }
                    (RangeBound::Static(None), RangeBound::Static(Some(to))) => {
                        Ok(TypeAtResult::Single(Type::Tuple(Vec::from_iter(
                            elements_types.into_iter().take(*to),
                        ))))
                    }
                    (RangeBound::Static(Some(from)), RangeBound::Dynamic(_)) => {
                        Ok(TypeAtResult::Single(Type::Array(Box::new(Type::Union(
                            BTreeSet::from_iter(elements_types.into_iter().skip(*from)),
                        )))))
                    }
                    (RangeBound::Dynamic(_), RangeBound::Static(Some(to))) => {
                        Ok(TypeAtResult::Single(Type::Array(Box::new(Type::Union(
                            BTreeSet::from_iter(elements_types.into_iter().take(*to)),
                        )))))
                    }
                    _ => Ok(TypeAtResult::Single(Type::Array(Box::new(Type::Union(
                        BTreeSet::from_iter(elements_types),
                    ))))),
                }
            }
            (Type::Object(mut object_inner_types), ValuePathSegment::ObjectKey(object_key)) => {
                if let Some(inner_type) = object_inner_types.remove(object_key) {
                    Ok(TypeAtResult::Single(inner_type))
                } else {
                    Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?} because no key {object_key:?}",
                        Type::Object(object_inner_types)
                    ))
                }
            }
            (Type::GenericObject(object_value_type), ValuePathSegment::ObjectKey(_)) => {
                Ok(TypeAtResult::Single(*object_value_type))
            }
            (self_, _) => Err(anyhow!("can not get from {self_:#?} at {at_segment:?}",)),
        }
    }

    pub fn at_path(self, path: &[ValuePathSegment]) -> Result<TypeAtResult> {
        if let Some(first_path_segment) = path.first() {
            let self_at = self.at(first_path_segment)?;
            match self_at {
                TypeAtResult::Single(intermediate_result) => {
                    intermediate_result.at_path(&path[1..])
                }
                TypeAtResult::Multiple(intermediate_results) => {
                    let mut results = BTreeSet::new();
                    for intermediate_result in intermediate_results {
                        match intermediate_result.at_path(&path[1..])? {
                            TypeAtResult::Single(result) => {
                                results.insert(result);
                            }
                            TypeAtResult::Multiple(mut results_part) => {
                                results.append(&mut results_part);
                            }
                        }
                    }
                    Ok(TypeAtResult::Multiple(results))
                }
            }
        } else {
            Ok(TypeAtResult::Single(self))
        }
    }

    pub fn union_types(&'a self) -> Box<dyn Iterator<Item = &'a Type> + 'a> {
        match self {
            Type::Constructed(constructed) => constructed.inner().union_types(),
            Type::Union(union_types) => Box::new(union_types.iter()),
            _ => Box::new([self].into_iter()),
        }
    }

    pub fn union_types_len(&'a self) -> usize {
        match self {
            Type::Constructed(constructed) => constructed.inner().union_types_len(),
            Type::Union(union_types) => union_types.len(),
            _ => 1,
        }
    }

    pub fn intersection(&self, other: &Type) -> Option<Type> {
        if self == other {
            Some(self.clone())
        } else {
            match (self, other) {
                (Type::Any, other_type) | (other_type, Type::Any) => Some(other_type.clone()),
                (Type::Bool, Type::LiteralTrue) | (Type::LiteralTrue, Type::Bool) => {
                    Some(Type::LiteralTrue)
                }
                (Type::Bool, Type::LiteralFalse) | (Type::LiteralFalse, Type::Bool) => {
                    Some(Type::LiteralFalse)
                }
                (Type::String, Type::LiteralString(literal_string))
                | (Type::LiteralString(literal_string), Type::String) => {
                    Some(Type::LiteralString(literal_string.clone()))
                }
                (Type::Constructed(self_constructed), Type::Constructed(other_constructed)) => {
                    self_constructed
                        .inner()
                        .intersection(other_constructed.inner())
                }
                (Type::Constructed(self_constructed), _) => {
                    self_constructed.inner().intersection(other)
                }
                (_, Type::Constructed(other_constructed)) => {
                    self.intersection(other_constructed.inner())
                }
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
                (Type::Object(self_inner_types), Type::Object(other_inner_types)) => {
                    let mut result_inner_types = BTreeMap::new();
                    for ((self_key, self_value_type), (other_key, other_value_type)) in
                        self_inner_types.iter().zip(other_inner_types.iter())
                    {
                        if self_key != other_key {
                            return None;
                        }
                        if let Some(values_types_intersection) =
                            self_value_type.intersection(other_value_type)
                        {
                            result_inner_types.insert(self_key.clone(), values_types_intersection);
                        } else {
                            return None;
                        }
                    }
                    if result_inner_types.is_empty() {
                        None
                    } else {
                        Some(Type::Object(result_inner_types))
                    }
                }
                (Type::Object(self_inner_types), Type::GenericObject(other_value_type))
                | (Type::GenericObject(other_value_type), Type::Object(self_inner_types)) => {
                    let mut result_inner_types = BTreeMap::new();
                    for (self_key, self_value_type) in self_inner_types.iter() {
                        if let Some(values_types_intersection) =
                            self_value_type.intersection(other_value_type)
                        {
                            result_inner_types.insert(self_key.clone(), values_types_intersection);
                        }
                    }
                    Some(Type::Object(result_inner_types))
                }
                (Type::GenericObject(self_value_type), Type::GenericObject(other_value_type)) => {
                    self_value_type.intersection(other_value_type).map(
                        |values_types_intersection| {
                            Type::GenericObject(Box::new(values_types_intersection))
                        },
                    )
                }
                _ => None,
            }
        }
    }
}
