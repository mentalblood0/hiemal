use std::{collections::BTreeMap, sync::Arc};

use dashu::Rational;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    containers::{self, Vector},
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
    Object(containers::Object<String, Option<Value>>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct IntermediateRepresentation {
    pub root: Node,
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
    Value(Node),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct Case {
    pub condition: Condition,
    pub node: Node,
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub enum Content {
    Tuple(Vec<Arc<Node>>),
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
    Map(Arc<Map>),
    Filter(Arc<Filter>),
    Fold {
        fold: Box<Node>,
        fold_constant_name_clustered_index: usize,
        starting_with: Box<Node>,
        accumulating_in_constant_name_clustered_index: usize,
        throughs: Throughs,
    },
    Sequence(Arc<Sequence>),
    Object(BTreeMap<Arc<String>, Arc<Node>>),
    Value(Arc<Option<Value>>),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct Map {
    pub map: Box<Node>,
    pub throughs: Throughs,
    pub map_constant_name_clustered_index: usize,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct Filter {
    pub filter: Box<Node>,
    pub throughs: Throughs,
    pub filter_constant_name_clustered_index: usize,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct Sequence {
    pub starting_with: Box<Node>,
    pub current_constant_name_clustered_index: usize,
    pub next: Arc<Node>,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub enum Throughs {
    Array(Box<Node>),
    Tuple {
        nodes_indexes: Vec<usize>,
        nodes: Vec<Node>,
    },
}

#[repr(u8)]
#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub enum EmbeddedFunction {
    Sum(Node),
    IsSorted(Node),
    StandardInput,
    ParseYaml(Node),
    KeyValuePairs(Node),
    Flatten(Node),
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct UserFunction {
    pub external_constants_name_clustered_indices: Vec<usize>,
    pub node: Node,
    pub is_pure: bool,
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq, Serialize, Deserialize, Hash)]
pub struct ConstantDefinition {
    pub name_clustered_index: usize,
    pub node: Arc<Node>,
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
pub enum RangeBound {
    Static(Option<usize>),
    Dynamic(Node),
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
    ArrayRange((Box<RangeBound>, Box<RangeBound>)),
}
