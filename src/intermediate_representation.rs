use std::collections::BTreeMap;

use crate::{program::Path, value::Value};

#[derive(Debug, Clone)]
pub struct IntermediateRepresentation {
    pub root: Node,
    pub user_functions: Vec<(Vec<usize>, Node)>,
    pub constants: Vec<Node>,
    pub unique_constants_names_count: usize,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub path: Path,
    pub content: Content,
}

#[derive(Debug, Clone)]
pub enum Content {
    Array(Vec<Node>),
    Scope {
        constants: Vec<(usize, usize)>,
        compute: Box<Node>,
    },
    Branching(Box<Branching>),
    Constant(usize),
    EmbeddedFunctionCall(Box<EmbeddedFunction>),
    UserFunctionCall {
        arguments: Vec<(usize, usize)>,
        body: usize,
    },
    Object(BTreeMap<String, Node>),
    Value(Value),
}

#[derive(Debug, Clone)]
pub struct Branching {
    pub r#if: Node,
    pub then: Node,
    pub r#else: Node,
}

#[derive(Debug, Clone)]
pub enum EmbeddedFunction {
    Sum(Node),
    IsSorted(Node),
}
