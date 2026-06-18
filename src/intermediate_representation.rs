use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{program::Path, value::Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntermediateRepresentation {
    pub root: Node,
    pub user_functions: Vec<UserFunction>,
    pub constants: Vec<Node>,
    pub unique_constants_names_count: usize,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub struct Node {
    pub path: Path,
    pub content: Content,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize)]
pub enum Content {
    Array(Vec<Node>),
    Scope {
        constants: Vec<ConstantDefinition>,
        compute: Box<Node>,
    },
    Branching(Box<Branching>),
    Constant(usize),
    EmbeddedFunctionCall(Box<EmbeddedFunction>),
    UserFunctionCall {
        arguments: Vec<ConstantDefinition>,
        body: usize,
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
