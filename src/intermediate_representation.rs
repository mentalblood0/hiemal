use std::collections::BTreeMap;

use dashu::Rational;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    containers::{Map, Vector},
    program::Path,
};

#[serde_as]
#[repr(u8)]
#[derive(Deserialize, Serialize, PartialEq, Debug, Clone, PartialOrd, Eq, Ord, Default, Hash)]
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
    #[default]
    Null,
    Array(Vector<Value>),
    Object(Map<String, Value>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntermediateRepresentation {
    pub root: Node,
    pub user_functions: Vec<UserFunction>,
    pub constants: Vec<Node>,
    pub unique_constants_names_count: usize,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub struct Node {
    pub content: Content,
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub enum Content {
    Array(Vec<Node>),
    Scope {
        constants: Vec<ConstantDefinition>,
        compute: Box<Node>,
    },
    Branching(Box<Branching>),
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
    Object(BTreeMap<String, Node>),
    Value(Value),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub struct Branching {
    pub r#if: Node,
    pub then: Node,
    pub r#else: Node,
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub enum EmbeddedFunction {
    Sum(Node),
    IsSorted(Node),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub struct UserFunction {
    pub external_constants_name_clustered_indices: Vec<usize>,
    pub node: Node,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub struct ConstantDefinition {
    pub name_clustered_index: usize,
    pub index: usize,
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub enum ValuePathSegment {
    ArrayIndex(usize),
    ObjectKey(String),
}
