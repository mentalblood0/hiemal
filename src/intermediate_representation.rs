use std::{collections::BTreeMap, sync::Arc};

use bytes::Bytes;
use dashu::Rational;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    containers::{self, Vector},
    program::Path,
    r#type::Type,
    value::{deserialize_bytes, serialize_bytes},
};

#[serde_as]
#[repr(u8)]
#[derive(Deserialize, Serialize, PartialEq, Debug, Clone, PartialOrd, Eq, Ord, Hash)]
pub enum Value {
    Number(#[serde_as(as = "DisplayFromStr")] Rational),
    String(
        #[serde(
            deserialize_with = "crate::value::deserialize_rope",
            serialize_with = "crate::value::serialize_rope"
        )]
        ropey::Rope,
    ),
    Bool(bool),
    Tuple(Vector<Option<Value>>),
    Bytes(
        #[serde(
            deserialize_with = "deserialize_bytes",
            serialize_with = "serialize_bytes"
        )]
        Bytes,
    ),
    Object(containers::Object<String, Option<Value>>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct IntermediateRepresentation {
    pub root: Arc<Node>,
    pub user_functions: Vec<UserFunction>,
    pub unique_constants_names_count: usize,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct Node {
    pub content: Content,
    pub r#type: Type,
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub enum Condition {
    Type(Type),
    Value(Arc<Node>),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct Case {
    pub condition: Condition,
    pub node: Arc<Node>,
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub enum Content {
    Tuple(Vec<Arc<Node>>),
    Scope {
        constants: Vec<ConstantDefinition>,
        compute: Arc<Node>,
    },
    Constant(usize),
    EmbeddedFunctionCall {
        path: Option<Path>,
        embedded_function: Box<EmbeddedFunction>,
    },
    UserFunctionCall {
        arguments: Vec<ConstantDefinition>,
        body: usize,
    },
    FromAt {
        from: Arc<Node>,
        value_path_segments: Vec<ValuePathSegment>,
        default: Arc<Node>,
    },
    Match {
        r#match: Arc<Node>,
        cases: Vec<Case>,
        match_constant_name_clustered_index_option: Option<usize>,
    },
    Map(Arc<Map>),
    Filter(Arc<Filter>),
    Fold {
        fold: Arc<Node>,
        fold_constant_name_clustered_index: usize,
        starting_with: Arc<Node>,
        accumulating_in_constant_name_clustered_index: usize,
        fold_concrete_type_and_throughs: Vec<(Type, Throughs)>,
    },
    Sequence(Arc<Sequence>),
    Object(BTreeMap<Arc<String>, Arc<Node>>),
    Value(Arc<Option<Value>>),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct Map {
    pub map: Arc<Node>,
    pub map_concrete_type_and_throughs: Vec<(Type, Throughs)>,
    pub map_constant_name_clustered_index: usize,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct Filter {
    pub filter: Arc<Node>,
    pub filter_concrete_type_and_throughs: Vec<(Type, Throughs)>,
    pub filter_constant_name_clustered_index: usize,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct Sequence {
    pub starting_with: Arc<Node>,
    pub current_constant_name_clustered_index: usize,
    pub next: Arc<Node>,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub enum Throughs {
    Array(Arc<Node>),
    Tuple {
        nodes_indexes: Vec<usize>,
        nodes: Vec<Arc<Node>>,
    },
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub enum EmbeddedFunction {
    Sum(Arc<Node>),
    IsSorted(Arc<Node>),
    StandardInput,
    ParseYaml(Arc<Node>),
    KeyValuePairs(Arc<Node>),
    Flatten(Arc<Node>),
    MatchGroups { string: Arc<Node>, regex: Arc<Node> },
    Concat(Arc<Node>),
    ReadBytesFromFile(Arc<Node>),
    StringFromBytes(Arc<Node>),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct UserFunction {
    pub external_constants_name_clustered_indices: Vec<usize>,
    pub node: Arc<Node>,
    pub is_pure: bool,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct ConstantDefinition {
    pub name_clustered_index: usize,
    pub node: Arc<Node>,
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum RangeBound {
    Static(Option<usize>),
    Dynamic(Arc<Node>),
}

impl Default for RangeBound {
    fn default() -> Self {
        Self::Static(None)
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub enum ValuePathSegment {
    ArrayIndex(usize),
    ObjectKey(String),
    ArrayRange {
        from: Box<RangeBound>,
        to: Box<RangeBound>,
    },
}
