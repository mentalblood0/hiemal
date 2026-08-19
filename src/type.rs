use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    sync::{Arc, LazyLock},
};

use anyhow::{Result, anyhow};
use enumset::{EnumSet, EnumSetType};
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

static CONSTRUCTED_TYPES: LazyLock<[Type; 5]> = LazyLock::new(|| {
    [
        TypeKind::Union(
            BTreeSet::from_iter([
                TypeKind::GenericLiteralString.into(),
                TypeKind::Array(Box::new(
                    TypeKind::Union(
                        BTreeSet::from_iter([
                            TypeKind::String.into(),
                            TypeKind::Object(
                                BTreeMap::from_iter([(
                                    "raw string".to_string().into(),
                                    TypeKind::String.into(),
                                )])
                                .into(),
                            )
                            .into(),
                            TypeKind::Constructed(Constructed::OneOf).into(),
                            TypeKind::Constructed(Constructed::CharacterExceptFromString).into(),
                            TypeKind::Constructed(Constructed::Repeat).into(),
                            TypeKind::Constructed(Constructed::Group).into(),
                            TypeKind::LiteralString("character".into()).into(),
                            TypeKind::LiteralString("whitespace character".into()).into(),
                            TypeKind::LiteralString("non-whitespace character".into()).into(),
                            TypeKind::LiteralString("digit".into()).into(),
                            TypeKind::LiteralString("non-digit".into()).into(),
                            TypeKind::LiteralString("word character".into()).into(),
                            TypeKind::LiteralString("non-word character".into()).into(),
                            TypeKind::LiteralString("start of string".into()).into(),
                            TypeKind::LiteralString("end of string".into()).into(),
                            TypeKind::LiteralString("word boundary".into()).into(),
                            TypeKind::LiteralString("non-word boundary".into()).into(),
                        ])
                        .into(),
                    )
                    .into(),
                ))
                .into(),
            ])
            .into(),
        )
        .into(),
        TypeKind::Object(
            BTreeMap::from_iter([(
                "one of".to_string().into(),
                TypeKind::Array(Box::new(TypeKind::Constructed(Constructed::Regex).into())).into(),
            )])
            .into(),
        )
        .into(),
        TypeKind::Object(
            BTreeMap::from_iter([(
                "character except from string".to_string().into(),
                TypeKind::String.into(),
            )])
            .into(),
        )
        .into(),
        TypeKind::Union(
            BTreeSet::from_iter([
                TypeKind::Object(
                    BTreeMap::from_iter([
                        (
                            "repeat".to_string().into(),
                            TypeKind::Constructed(Constructed::Regex).into(),
                        ),
                        ("min".to_string().into(), TypeKind::Number.into()),
                        ("max".to_string().into(), TypeKind::Number.into()),
                    ])
                    .into(),
                )
                .into(),
                TypeKind::Object(
                    BTreeMap::from_iter([
                        (
                            "repeat".to_string().into(),
                            TypeKind::Constructed(Constructed::Regex).into(),
                        ),
                        ("min".to_string().into(), TypeKind::Number.into()),
                    ])
                    .into(),
                )
                .into(),
                TypeKind::Object(
                    BTreeMap::from_iter([
                        (
                            "repeat".to_string().into(),
                            TypeKind::Constructed(Constructed::Regex).into(),
                        ),
                        ("max".to_string().into(), TypeKind::Number.into()),
                    ])
                    .into(),
                )
                .into(),
                TypeKind::Object(
                    BTreeMap::from_iter([
                        (
                            "repeat".to_string().into(),
                            TypeKind::Constructed(Constructed::Regex).into(),
                        ),
                        ("exactly".to_string().into(), TypeKind::Number.into()),
                    ])
                    .into(),
                )
                .into(),
            ])
            .into(),
        )
        .into(),
        TypeKind::Object(
            BTreeMap::from_iter([
                (
                    "group".to_string().into(),
                    TypeKind::Constructed(Constructed::Regex).into(),
                ),
                ("name".to_string().into(), TypeKind::String.into()),
            ])
            .into(),
        )
        .into(),
    ]
});

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Constructed {
    #[default]
    Regex,
    OneOf,
    CharacterExceptFromString,
    Repeat,
    Group,
}

impl Constructed {
    pub fn inner(&self) -> &Type {
        CONSTRUCTED_TYPES.get(self.clone() as u8 as usize).unwrap()
    }
}

#[derive(EnumSetType, Serialize, Deserialize, Debug, PartialOrd, Ord, Hash)]
#[enumset(serialize_repr = "list")]
pub enum Capability {
    #[serde(rename = "append to file")]
    AppendToFile,
    #[serde(rename = "create file")]
    CreateFile,
    #[serde(rename = "overwrite file")]
    OverwriteFile,
    #[serde(rename = "remove file")]
    RemoveFile,
    #[serde(rename = "read file")]
    ReadFile,
    #[serde(rename = "read standard input")]
    ReadStandardInput,
    #[serde(rename = "read network")]
    ReadNetwork,
    #[serde(rename = "write network")]
    WriteNetwork,
}

#[derive(Serialize, Deserialize, Debug, PartialOrd, Ord, Hash, PartialEq, Eq, Default, Clone)]
pub struct TypeProperties {
    pub capabilities: EnumSet<Capability>,
    pub is_computable: bool,
}

impl TypeProperties {
    pub fn unified(&self, another_type_properties: &TypeProperties) -> TypeProperties {
        Self {
            capabilities: self.capabilities | another_type_properties.capabilities,
            is_computable: self.is_computable && another_type_properties.is_computable,
        }
    }

    pub fn intersect(&mut self, another_type_properties: &TypeProperties) {
        self.capabilities |= another_type_properties.capabilities;
        self.is_computable = self.is_computable && another_type_properties.is_computable;
    }
}

impl<'a, I> From<I> for TypeProperties
where
    I: Iterator<Item = &'a TypeProperties>,
{
    fn from(type_properties_iterator: I) -> Self {
        let mut result = TypeProperties::default();
        for type_properties in type_properties_iterator {
            result = result.unified(type_properties);
        }
        result
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Default, Eq)]
#[serde(from = "TypeKind")]
#[serde(into = "TypeKind")]
pub struct Type {
    pub kind: TypeKind,
    pub properties: TypeProperties,
}

impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl PartialOrd for Type {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Type {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.kind.cmp(&other.kind)
    }
}

impl Hash for Type {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
    }
}

impl From<TypeKind> for Type {
    fn from(type_kind: TypeKind) -> Self {
        Self {
            kind: type_kind,
            properties: TypeProperties {
                capabilities: EnumSet::default(),
                is_computable: true,
            },
        }
    }
}

impl From<Type> for TypeKind {
    fn from(r#type: Type) -> Self {
        r#type.kind
    }
}

impl<'a, I> From<(TypeKind, I)> for Type
where
    I: Iterator<Item = &'a Type>,
{
    fn from(type_kind_and_inner_types: (TypeKind, I)) -> Self {
        Self {
            kind: type_kind_and_inner_types.0,
            properties: type_kind_and_inner_types
                .1
                .map(|r#type| &r#type.properties)
                .into(),
        }
    }
}

#[derive(Deserialize, Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeKind {
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
    Tuple(Arc<Vec<Type>>),
    #[serde(rename = "object")]
    Object(Arc<BTreeMap<Arc<String>, Type>>),
    #[serde(rename = "generic object")]
    GenericObject(Box<Type>),
    #[serde(rename = "union")]
    Union(Arc<BTreeSet<Type>>),
    #[default]
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "literal true")]
    LiteralTrue,
    #[serde(rename = "literal false")]
    LiteralFalse,
    #[serde(rename = "literal string")]
    LiteralString(#[serde(deserialize_with = "deserialize_rope")] ropey::Rope),
    #[serde(rename = "generic literal string")]
    GenericLiteralString,
    #[serde(rename = "bytes")]
    Bytes,
    #[serde(rename = "constructed")]
    Constructed(Constructed),
    #[serde(skip_deserializing)]
    Unknown(MaybeType),
}

#[repr(u8)]
#[derive(Serialize, Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KnownTypeKind {
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "string")]
    String,
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "null")]
    Null,
    #[serde(rename = "array")]
    Array(Box<TypeKind>),
    #[serde(rename = "tuple")]
    Tuple(Arc<Vec<Type>>),
    #[serde(rename = "object")]
    Object(Arc<BTreeMap<Arc<String>, Type>>),
    #[serde(rename = "generic object")]
    GenericObject(Box<Type>),
    #[serde(rename = "union")]
    Union(Arc<BTreeSet<Type>>),
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
    #[serde(rename = "generic literal string")]
    GenericLiteralString,
    #[serde(rename = "bytes")]
    Bytes,
    #[serde(rename = "constructed")]
    Constructed(Constructed),
}

impl Serialize for TypeKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            TypeKind::Unknown(maybe_type) => match &*maybe_type.lockable_internals.read() {
                Some(r#type) => r#type.serialize(serializer),
                None => Result::Err(serde::ser::Error::custom(
                    "unknown type have not been resolved",
                )),
            },
            known_type => unsafe {
                std::mem::transmute::<&TypeKind, &KnownTypeKind>(known_type).serialize(serializer)
            },
        }
    }
}

impl From<BTreeSet<Type>> for Type {
    fn from(mut union_types: BTreeSet<Type>) -> Type {
        if union_types.contains(&TypeKind::Any.into()) {
            (TypeKind::Any, union_types.iter()).into()
        } else {
            if union_types
                .iter()
                .any(|union_type| matches!(union_type.kind, TypeKind::Union(_)))
            {
                union_types = union_types
                    .into_iter()
                    .flat_map(|union_type| match &union_type.kind {
                        TypeKind::Union(inner_union_types) => {
                            Box::new((**inner_union_types).clone().into_iter())
                                as Box<dyn Iterator<Item = Type>>
                        }
                        _ => {
                            Box::new(std::iter::once(union_type)) as Box<dyn Iterator<Item = Type>>
                        }
                    })
                    .collect();
            };
            if let (Some(literal_true_union_type), Some(literal_false_union_type)) = (
                union_types.get(&TypeKind::LiteralTrue.into()).cloned(),
                union_types.get(&TypeKind::LiteralFalse.into()).cloned(),
            ) {
                union_types.remove(&TypeKind::LiteralTrue.into());
                union_types.remove(&TypeKind::LiteralFalse.into());
                union_types.insert(
                    (
                        TypeKind::Bool,
                        [literal_true_union_type, literal_false_union_type].iter(),
                    )
                        .into(),
                );
            }
            match union_types.len() {
                1 => union_types.into_iter().next().unwrap(),
                _ => {
                    let properties = union_types.iter().map(|r#type| &r#type.properties).into();
                    Type {
                        kind: TypeKind::Union(union_types.into()),
                        properties,
                    }
                }
            }
        }
    }
}

impl From<Vec<Type>> for Type {
    fn from(tuple_types: Vec<Type>) -> Type {
        let properties = tuple_types.iter().map(|r#type| &r#type.properties).into();
        Type {
            kind: TypeKind::Tuple(tuple_types.into()),
            properties,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TypeAtResult {
    Single(Type),
    Multiple(BTreeSet<Type>),
}

impl TypeKind {
    pub fn is_known(&self) -> bool {
        match self {
            TypeKind::Unknown(maybe_type) => maybe_type.lockable_internals.read().is_some(),
            TypeKind::Array(element_type) => element_type.kind.is_known(),
            TypeKind::Tuple(elements_types) => elements_types
                .iter()
                .all(|element_type| element_type.kind.is_known()),
            TypeKind::Object(inner_types) => inner_types
                .values()
                .all(|value_type| value_type.kind.is_known()),
            TypeKind::GenericObject(value_type) => value_type.kind.is_known(),
            TypeKind::Union(union_types) => union_types
                .iter()
                .all(|union_type| union_type.kind.is_known()),
            TypeKind::Constructed(constructed) => constructed.inner().kind.is_known(),
            _ => true,
        }
    }

    pub fn is_concrete(&self) -> bool {
        match self {
            TypeKind::Array(element_type) => element_type.kind.is_concrete(),
            TypeKind::Tuple(elements_types) => elements_types
                .iter()
                .all(|element_type| element_type.kind.is_concrete()),
            TypeKind::Object(inner_types) => inner_types
                .values()
                .all(|value_type| value_type.kind.is_concrete()),
            TypeKind::GenericObject(value_type) => value_type.kind.is_concrete(),
            TypeKind::Union(_) | TypeKind::Any | TypeKind::Unknown(_) => false,
            TypeKind::Constructed(constructed) => constructed.inner().kind.is_concrete(),
            _ => true,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<Arc<String>, Type>> {
        match self {
            TypeKind::Object(result) => Some(result),
            _ => None,
        }
    }

    pub fn contains(&self, other: &TypeKind) -> bool {
        if self == other {
            true
        } else {
            match (self, other) {
                (TypeKind::Any, _)
                | (TypeKind::String, TypeKind::LiteralString(_))
                | (TypeKind::String, TypeKind::GenericLiteralString)
                | (TypeKind::LiteralString(_), TypeKind::GenericLiteralString) => true,
                (
                    TypeKind::Constructed(self_constructed),
                    TypeKind::Constructed(other_constructed),
                ) => self_constructed
                    .inner()
                    .kind
                    .contains(&other_constructed.inner().kind),
                (TypeKind::Constructed(self_constructed), _) => {
                    self_constructed.inner().kind.contains(other)
                }
                (_, TypeKind::Constructed(other_constructed)) => {
                    self.contains(&other_constructed.inner().kind)
                }
                (TypeKind::Union(self_union_types), TypeKind::Union(other_union_types)) => {
                    if self_union_types.is_superset(other_union_types) {
                        true
                    } else {
                        for other_union_type in other_union_types.as_ref() {
                            let mut found_container = false;
                            for self_union_type in self_union_types.as_ref() {
                                if self_union_type.kind.contains(&other_union_type.kind) {
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
                (TypeKind::Union(self_union_types), other_type_kind) => {
                    if self_union_types.contains(&other_type_kind.clone().into()) {
                        true
                    } else {
                        for self_union_type in self_union_types.as_ref() {
                            if self_union_type.kind.contains(other_type_kind) {
                                return true;
                            }
                        }
                        false
                    }
                }
                (self_type, TypeKind::Union(other_union_types)) => other_union_types
                    .iter()
                    .all(|other_union_type| self_type.contains(&other_union_type.kind)),
                (TypeKind::Bool, TypeKind::LiteralTrue | TypeKind::LiteralFalse) => true,
                (
                    TypeKind::Tuple(self_tuple_elements_types),
                    TypeKind::Tuple(other_tuple_elements_types),
                ) => {
                    if self_tuple_elements_types.len() != other_tuple_elements_types.len() {
                        return false;
                    }
                    for element_index in 0..self_tuple_elements_types.len() {
                        if !self_tuple_elements_types[element_index]
                            .kind
                            .contains(&other_tuple_elements_types[element_index].kind)
                        {
                            return false;
                        }
                    }
                    true
                }
                (
                    TypeKind::Array(self_array_element_type),
                    TypeKind::Tuple(other_tuple_elements_types),
                ) => {
                    for other_tuple_element_type in other_tuple_elements_types.as_ref() {
                        if !self_array_element_type
                            .kind
                            .contains(&other_tuple_element_type.kind)
                        {
                            return false;
                        }
                    }
                    true
                }

                (TypeKind::Object(self_inner_types), TypeKind::Object(other_inner_types)) => {
                    for ((self_key, self_value_type), (other_key, other_value_type)) in
                        self_inner_types.iter().zip(other_inner_types.iter())
                    {
                        if self_key != other_key {
                            return false;
                        }
                        if !self_value_type.kind.contains(&other_value_type.kind) {
                            return false;
                        }
                    }
                    true
                }
                (TypeKind::Object(self_inner_types), TypeKind::GenericObject(other_value_type)) => {
                    self_inner_types.values().all(|self_value_type| {
                        self_value_type.kind.contains(&other_value_type.kind)
                    })
                }
                (TypeKind::GenericObject(self_value_type), TypeKind::Object(other_inner_types)) => {
                    other_inner_types.values().all(|other_value_type| {
                        self_value_type.kind.contains(&other_value_type.kind)
                    })
                }
                (
                    TypeKind::GenericObject(self_value_type),
                    TypeKind::GenericObject(other_value_type),
                ) => self_value_type.kind.contains(&other_value_type.kind),
                _ => false,
            }
        }
    }

    pub fn intersection(&self, other: &TypeKind) -> Option<TypeKind> {
        if self == other {
            Some(self.clone())
        } else {
            match (self, other) {
                (TypeKind::Any, other_type) | (other_type, TypeKind::Any) => {
                    Some(other_type.clone())
                }
                (TypeKind::Bool, TypeKind::LiteralTrue)
                | (TypeKind::LiteralTrue, TypeKind::Bool) => Some(TypeKind::LiteralTrue),
                (TypeKind::Bool, TypeKind::LiteralFalse)
                | (TypeKind::LiteralFalse, TypeKind::Bool) => Some(TypeKind::LiteralFalse),
                (TypeKind::String, TypeKind::LiteralString(literal_string))
                | (TypeKind::LiteralString(literal_string), TypeKind::String) => {
                    Some(TypeKind::LiteralString(literal_string.clone()))
                }
                (TypeKind::String, TypeKind::GenericLiteralString)
                | (TypeKind::GenericLiteralString, TypeKind::String) => {
                    Some(TypeKind::GenericLiteralString)
                }
                (TypeKind::GenericLiteralString, TypeKind::LiteralString(literal_string))
                | (TypeKind::LiteralString(literal_string), TypeKind::GenericLiteralString) => {
                    Some(TypeKind::LiteralString(literal_string.clone()))
                }
                (
                    TypeKind::Constructed(self_constructed),
                    TypeKind::Constructed(other_constructed),
                ) => self_constructed
                    .inner()
                    .kind
                    .intersection(&other_constructed.inner().kind),
                (TypeKind::Constructed(self_constructed), _) => {
                    self_constructed.inner().kind.intersection(other)
                }
                (_, TypeKind::Constructed(other_constructed)) => {
                    self.intersection(&other_constructed.inner().kind)
                }
                (TypeKind::Union(self_union_types), TypeKind::Union(other_union_types)) => {
                    let result = self_union_types
                        .intersection(other_union_types)
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    if result.is_empty() {
                        None
                    } else {
                        Some(TypeKind::Union(result.into()))
                    }
                }
                (TypeKind::Union(union_types), other_type)
                | (other_type, TypeKind::Union(union_types)) => {
                    for union_type in union_types.iter() {
                        if let Some(result) = union_type.kind.intersection(other_type) {
                            return Some(result);
                        }
                    }
                    None
                }
                (
                    TypeKind::Array(self_array_element_type),
                    TypeKind::Array(other_array_element_type),
                ) => self_array_element_type
                    .kind
                    .intersection(&other_array_element_type.kind)
                    .map(|element_types_intersection| {
                        TypeKind::Array(Box::new(element_types_intersection.into()))
                    }),
                (
                    TypeKind::Tuple(self_tuple_elements_types),
                    TypeKind::Tuple(other_tuple_elements_types),
                ) => {
                    if self_tuple_elements_types.len() != other_tuple_elements_types.len() {
                        None
                    } else {
                        let mut result_tuple_types =
                            Vec::with_capacity(self_tuple_elements_types.len());
                        for element_index in 0..self_tuple_elements_types.len() {
                            if let Some(elements_types_intersection) = self_tuple_elements_types
                                [element_index]
                                .kind
                                .intersection(&other_tuple_elements_types[element_index].kind)
                            {
                                result_tuple_types.push(elements_types_intersection.into());
                            } else {
                                return None;
                            }
                        }
                        Some(TypeKind::Tuple(result_tuple_types.into()))
                    }
                }
                (TypeKind::Array(array_element_type), TypeKind::Tuple(tuple_elements_types))
                | (TypeKind::Tuple(tuple_elements_types), TypeKind::Array(array_element_type)) => {
                    let mut result_tuple_elements_types =
                        Vec::with_capacity(tuple_elements_types.len());
                    for tuple_element_type in tuple_elements_types.iter() {
                        if let Some(result_tuple_element_type) = tuple_element_type
                            .kind
                            .intersection(&array_element_type.kind)
                        {
                            result_tuple_elements_types.push(result_tuple_element_type)
                        } else {
                            return None;
                        }
                    }
                    Some(TypeKind::Tuple(tuple_elements_types.clone()))
                }
                (TypeKind::Object(self_inner_types), TypeKind::Object(other_inner_types)) => {
                    let mut result_inner_types = BTreeMap::new();
                    for ((self_key, self_value_type), (other_key, other_value_type)) in
                        self_inner_types.iter().zip(other_inner_types.iter())
                    {
                        if self_key != other_key {
                            return None;
                        }
                        if let Some(values_types_intersection) =
                            self_value_type.kind.intersection(&other_value_type.kind)
                        {
                            result_inner_types
                                .insert(self_key.clone(), values_types_intersection.into());
                        } else {
                            return None;
                        }
                    }
                    if result_inner_types.is_empty() {
                        None
                    } else {
                        Some(TypeKind::Object(result_inner_types.into()))
                    }
                }
                (TypeKind::Object(self_inner_types), TypeKind::GenericObject(other_value_type))
                | (TypeKind::GenericObject(other_value_type), TypeKind::Object(self_inner_types)) =>
                {
                    let mut result_inner_types = BTreeMap::new();
                    for (self_key, self_value_type) in self_inner_types.iter() {
                        if let Some(values_types_intersection) =
                            self_value_type.kind.intersection(&other_value_type.kind)
                        {
                            result_inner_types
                                .insert(self_key.clone(), values_types_intersection.into());
                        }
                    }
                    Some(TypeKind::Object(result_inner_types.into()))
                }
                (
                    TypeKind::GenericObject(self_value_type),
                    TypeKind::GenericObject(other_value_type),
                ) => self_value_type
                    .kind
                    .intersection(&other_value_type.kind)
                    .map(|values_types_intersection| {
                        TypeKind::GenericObject(Box::new(values_types_intersection.into()))
                    }),
                _ => None,
            }
        }
    }
}

impl<'a> Type {
    pub fn with_kind(&self, kind: TypeKind) -> Self {
        Self {
            kind,
            properties: self.properties.clone(),
        }
    }

    pub fn with_unified_properties_from(mut self, another_type: &Type) -> Self {
        self.properties = self.properties.unified(&another_type.properties);
        self
    }

    pub fn flatten(&self) -> Result<Type> {
        match &self.kind {
            TypeKind::Constructed(constructed) => constructed.inner().flatten(),
            TypeKind::Union(self_union_types) => {
                let mut result_union_types = BTreeSet::new();
                for self_union_type in self_union_types.as_ref() {
                    result_union_types.insert(self_union_type.flatten()?);
                }
                let result_union_types_arc = Arc::new(result_union_types);
                Ok(self.with_kind(TypeKind::Union(result_union_types_arc.clone())))
            }
            TypeKind::Array(element_type) => match &element_type.kind {
                TypeKind::Array(element_element_type) => {
                    Ok(self.with_kind(TypeKind::Array(element_element_type.clone())))
                }
                TypeKind::Tuple(element_elements_types) => {
                    Ok(self.with_kind(TypeKind::Array(Box::new(
                        (
                            TypeKind::Union(
                                BTreeSet::from_iter(element_elements_types.iter().cloned()).into(),
                            ),
                            element_elements_types.iter(),
                        )
                            .into(),
                    ))))
                }
                TypeKind::Union(element_union_types) => {
                    let mut result_element_union_types = BTreeSet::new();
                    for element_union_type in element_union_types.as_ref() {
                        match &element_union_type.kind {
                            TypeKind::Array(element_union_element_type) => {
                                result_element_union_types
                                    .insert(*element_union_element_type.clone());
                            }
                            TypeKind::Tuple(element_union_elements_types) => {
                                for element_union_element_type in
                                    element_union_elements_types.as_ref()
                                {
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
                    Ok(self.with_kind(TypeKind::Array(Box::new(Type::from(
                        result_element_union_types,
                    )))))
                }
                non_sequence_type => Err(anyhow!(
                    "can not flatten {self:#?} because it contains element of type \
                     {non_sequence_type:#?}"
                )),
            },
            TypeKind::Tuple(elements_types) => {
                if elements_types
                    .iter()
                    .all(|element_type| matches!(element_type.kind, TypeKind::Tuple(_)))
                {
                    let mut result_elements_types = Vec::new();
                    for element_type in elements_types.as_ref() {
                        match &element_type.kind {
                            TypeKind::Tuple(element_elements_types) => {
                                for element_element_type in element_elements_types.as_ref() {
                                    result_elements_types.push(element_element_type.clone());
                                }
                            }
                            _ => panic!(),
                        }
                    }
                    let result_elements_types_arc = Arc::new(result_elements_types);
                    Ok(self.with_kind(TypeKind::Tuple(result_elements_types_arc.clone())))
                } else {
                    let mut result_elements_types = BTreeSet::new();
                    for element_type in elements_types.as_ref() {
                        match &element_type.kind {
                            TypeKind::Array(element_element_type) => {
                                result_elements_types.insert(*element_element_type.clone());
                            }
                            TypeKind::Tuple(element_elements_types) => {
                                for element_element_type in element_elements_types.as_ref() {
                                    result_elements_types.insert(element_element_type.clone());
                                }
                            }
                            TypeKind::Union(element_union_types) => {
                                for element_union_type in element_union_types.as_ref() {
                                    match &element_union_type.kind {
                                        TypeKind::Array(element_union_element_type) => {
                                            result_elements_types
                                                .insert(*element_union_element_type.clone());
                                        }
                                        TypeKind::Tuple(element_union_elements_types) => {
                                            for element_union_element_type in
                                                element_union_elements_types.as_ref()
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
                    Ok(
                        self.with_kind(TypeKind::Array(Box::new(Type::from(
                            result_elements_types,
                        )))),
                    )
                }
            }
            _ => Err(anyhow!("can not flatten {self:#?}")),
        }
    }

    pub fn at(&self, at_segment: &ValuePathSegment) -> Result<(TypeAtResult, bool)> {
        match (&self.kind, at_segment) {
            (TypeKind::Constructed(constructed), _) => constructed.inner().clone().at(at_segment),
            (TypeKind::Union(union_types), _) => {
                let mut result_types = BTreeSet::new();
                let mut runtime_error_is_possible = false;
                for union_type in union_types.as_ref() {
                    let (mut union_type_at_result, union_type_runtime_error_is_possible) =
                        union_type.at(at_segment)?;
                    runtime_error_is_possible |= union_type_runtime_error_is_possible;
                    match union_type_at_result {
                        TypeAtResult::Single(result_type) => {
                            result_types.insert(result_type);
                        }
                        TypeAtResult::Multiple(ref mut result_types_part) => {
                            result_types.append(result_types_part);
                        }
                    }
                }
                Ok((
                    TypeAtResult::Multiple(result_types),
                    runtime_error_is_possible,
                ))
            }
            (TypeKind::Array(element_type), ValuePathSegment::ArrayIndex(_)) => {
                Ok((TypeAtResult::Single(*element_type.clone()), true))
            }
            (TypeKind::Tuple(elements_types), ValuePathSegment::ArrayIndex(tuple_index)) => {
                if *tuple_index >= elements_types.len() {
                    let elements_types_len = elements_types.len();
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?} because there is only {} \
                         elements",
                        TypeKind::Tuple(elements_types.clone()),
                        elements_types_len
                    ));
                }
                Ok((
                    TypeAtResult::Single(elements_types.get(*tuple_index).unwrap().clone()),
                    false,
                ))
            }
            (
                TypeKind::LiteralString(self_literal_string),
                ValuePathSegment::ArrayRange { from, to },
            ) => {
                if let (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) =
                    (&**from, &**to)
                    && from > to
                {
                    return Err(anyhow!("can not get from string at {at_segment:?}",));
                }
                if let (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) =
                    (&**from, &**to)
                {
                    Ok((
                        TypeAtResult::Single(
                            self.with_kind(TypeKind::LiteralString(
                                self_literal_string
                                    .get_slice(from..to)
                                    .unwrap_or_else(|| "".into())
                                    .into(),
                            )),
                        ),
                        false,
                    ))
                } else {
                    Ok((
                        TypeAtResult::Single(self.with_kind(TypeKind::String)),
                        false,
                    ))
                }
            }
            (TypeKind::String, ValuePathSegment::ArrayRange { from, to }) => {
                if let (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) =
                    (&**from, &**to)
                    && from > to
                {
                    return Err(anyhow!("can not get from string at {at_segment:?}",));
                }
                Ok((TypeAtResult::Single(self.clone()), false))
            }
            (TypeKind::Array(element_type), ValuePathSegment::ArrayRange { from, to }) => {
                if let (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) =
                    (&**from, &**to)
                    && from > to
                {
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?}",
                        TypeKind::Array(element_type.clone())
                    ));
                }
                if let (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) =
                    (&**from, &**to)
                {
                    Ok((
                        TypeAtResult::Single(
                            (
                                TypeKind::Tuple(vec![*element_type.clone(); to - from].into()),
                                std::iter::once(&**element_type),
                            )
                                .into(),
                        ),
                        false,
                    ))
                } else {
                    Ok((
                        TypeAtResult::Single(
                            (
                                TypeKind::Array(element_type.clone()),
                                std::iter::once(&**element_type),
                            )
                                .into(),
                        ),
                        false,
                    ))
                }
            }
            (TypeKind::Tuple(elements_types), ValuePathSegment::ArrayRange { from, to }) => {
                if let (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) =
                    (&**from, &**to)
                    && from > to
                {
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?}",
                        TypeKind::Tuple(elements_types.clone())
                    ));
                }
                if let RangeBound::Static(Some(from)) = &**from
                    && from >= &elements_types.len()
                {
                    let elements_types_len = elements_types.len();
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?} because {from} >= {}",
                        TypeKind::Tuple(elements_types.clone()),
                        elements_types_len
                    ));
                }
                if let RangeBound::Static(Some(to)) = &**to
                    && to > &elements_types.len()
                {
                    let elements_types_len = elements_types.len();
                    return Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?} because {to} > {}",
                        TypeKind::Tuple(elements_types.clone()),
                        elements_types_len
                    ));
                }
                match (&**from, &**to) {
                    (RangeBound::Static(Some(from)), RangeBound::Static(Some(to))) => Ok((
                        TypeAtResult::Single(
                            (
                                TypeKind::Tuple(
                                    Vec::from_iter(
                                        elements_types.iter().skip(*from).take(to - from).cloned(),
                                    )
                                    .into(),
                                ),
                                elements_types.iter().skip(*from).take(to - from),
                            )
                                .into(),
                        ),
                        false,
                    )),
                    (RangeBound::Static(Some(from)), RangeBound::Static(None)) => Ok((
                        TypeAtResult::Single(
                            (
                                TypeKind::Tuple(
                                    Vec::from_iter(elements_types.iter().skip(*from).cloned())
                                        .into(),
                                ),
                                elements_types.iter().skip(*from),
                            )
                                .into(),
                        ),
                        false,
                    )),
                    (RangeBound::Static(None), RangeBound::Static(Some(to))) => Ok((
                        TypeAtResult::Single(
                            (
                                TypeKind::Tuple(
                                    Vec::from_iter(elements_types.iter().take(*to).cloned()).into(),
                                ),
                                elements_types.iter().take(*to),
                            )
                                .into(),
                        ),
                        false,
                    )),
                    (RangeBound::Static(Some(from)), RangeBound::Dynamic(_)) => Ok((
                        TypeAtResult::Single(
                            (
                                TypeKind::Array(Box::new(
                                    (
                                        TypeKind::Union(
                                            BTreeSet::from_iter(
                                                elements_types.iter().skip(*from).cloned(),
                                            )
                                            .into(),
                                        ),
                                        elements_types.iter().skip(*from),
                                    )
                                        .into(),
                                )),
                                elements_types.iter().skip(*from),
                            )
                                .into(),
                        ),
                        false,
                    )),
                    (RangeBound::Dynamic(_), RangeBound::Static(Some(to))) => Ok((
                        TypeAtResult::Single(
                            (
                                TypeKind::Array(Box::new(
                                    (
                                        TypeKind::Union(
                                            BTreeSet::from_iter(
                                                elements_types.iter().take(*to).cloned(),
                                            )
                                            .into(),
                                        ),
                                        elements_types.iter().take(*to),
                                    )
                                        .into(),
                                )),
                                elements_types.iter().take(*to),
                            )
                                .into(),
                        ),
                        false,
                    )),
                    _ => Ok((
                        TypeAtResult::Single(
                            (
                                TypeKind::Array(Box::new(
                                    (
                                        TypeKind::Union(
                                            BTreeSet::from_iter(elements_types.iter().cloned())
                                                .into(),
                                        ),
                                        elements_types.iter(),
                                    )
                                        .into(),
                                )),
                                elements_types.iter(),
                            )
                                .into(),
                        ),
                        false,
                    )),
                }
            }
            (TypeKind::Object(object_inner_types), ValuePathSegment::ObjectKey(object_key)) => {
                if let Some(inner_type) = object_inner_types.get(object_key) {
                    Ok((TypeAtResult::Single(inner_type.clone()), false))
                } else {
                    Err(anyhow!(
                        "can not get from {:#?} at {at_segment:?} because no key {object_key:?}",
                        TypeKind::Object(object_inner_types.clone())
                    ))
                }
            }
            (TypeKind::GenericObject(object_value_type), ValuePathSegment::ObjectKey(_)) => {
                Ok((TypeAtResult::Single(*object_value_type.clone()), true))
            }
            (self_, _) => Err(anyhow!("can not get from {self_:#?} at {at_segment:?}",)),
        }
    }

    pub fn at_path(self, path: &[ValuePathSegment]) -> Result<(TypeAtResult, bool)> {
        if let Some(first_path_segment) = path.first() {
            let (self_at, mut runtime_error_is_possible) = self.at(first_path_segment)?;
            match self_at {
                TypeAtResult::Single(intermediate_result) => {
                    let (
                        intermediate_result_type_at_result,
                        intermediate_result_runtime_error_is_possible,
                    ) = intermediate_result.at_path(&path[1..])?;
                    Ok((
                        intermediate_result_type_at_result,
                        runtime_error_is_possible || intermediate_result_runtime_error_is_possible,
                    ))
                }
                TypeAtResult::Multiple(intermediate_results) => {
                    let mut results = BTreeSet::new();
                    for intermediate_result in intermediate_results {
                        let (
                            intermediate_result_type_at_result,
                            intermediate_result_runtime_error_is_possible,
                        ) = intermediate_result.at_path(&path[1..])?;
                        runtime_error_is_possible |= intermediate_result_runtime_error_is_possible;
                        match intermediate_result_type_at_result {
                            TypeAtResult::Single(result) => {
                                results.insert(result);
                            }
                            TypeAtResult::Multiple(mut results_part) => {
                                results.append(&mut results_part);
                            }
                        }
                    }
                    Ok((TypeAtResult::Multiple(results), runtime_error_is_possible))
                }
            }
        } else {
            Ok((TypeAtResult::Single(self), false))
        }
    }

    pub fn union_types(&'a self) -> Box<dyn Iterator<Item = &'a Type> + 'a> {
        match &self.kind {
            TypeKind::Constructed(constructed) => constructed.inner().union_types(),
            TypeKind::Union(union_types) => Box::new(union_types.iter()),
            _ => Box::new([self].into_iter()),
        }
    }

    pub fn union_types_len(&'a self) -> usize {
        match &self.kind {
            TypeKind::Constructed(constructed) => constructed.inner().union_types_len(),
            TypeKind::Union(union_types) => union_types.len(),
            _ => 1,
        }
    }
}
