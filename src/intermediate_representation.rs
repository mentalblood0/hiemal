use std::collections::BTreeMap;

use dashu::Rational;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    containers::{Map, Vector},
    program::Path,
    r#type::Type,
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
    Object(Map<String, Option<Value>>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntermediateRepresentation {
    pub root: Node,
    pub user_functions: Vec<UserFunction>,
    pub unique_constants_names_count: usize,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub struct Node {
    pub content: Content,
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub enum Condition {
    Type(Type),
    Value(Node),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub struct Case {
    pub condition: Condition,
    pub node: Node,
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub enum Content {
    Tuple(Vec<Node>),
    Scope {
        constants: Vec<ConstantDefinition>,
        compute: Box<Node>,
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
        from: Box<Node>,
        value_path_segments: Vec<ValuePathSegment>,
    },
    Match {
        r#match: Box<Node>,
        cases: Vec<Case>,
        match_constant_name_clustered_index_option: Option<usize>,
    },
    Map {
        map: Box<Node>,
        throughs: MapThroughs,
        map_constant_name_clustered_index: usize,
    },
    Object(BTreeMap<String, Node>),
    Value(Option<Value>),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub enum MapThroughs {
    Array(Box<Node>),
    Tuple {
        elements_nodes_indexes: Vec<usize>,
        nodes: Vec<Node>,
    },
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub enum EmbeddedFunction {
    Sum(Node),
    IsSorted(Node),
    StandardInput,
    ParseYaml(Node),
    KeyValuePairs(Node),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub struct UserFunction {
    pub external_constants_name_clustered_indices: Vec<usize>,
    pub node: Node,
    pub is_pure: bool,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub struct ConstantDefinition {
    pub name_clustered_index: usize,
    pub node: Node,
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub enum ValuePathSegment {
    ArrayIndex(usize),
    ObjectKey(String),
}
