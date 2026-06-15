use std::{collections::BTreeMap, sync::Arc};

use crate::{containers::Vector, value::Value};

#[derive(serde::Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord)]
#[serde(untagged)]
pub enum Program {
    Array(Arc<Vec<Program>>),
    Clause(Clause),
    EmbeddedFunction(Box<EmbeddedFunction>),
    Object(Arc<BTreeMap<String, Program>>),
    Value(Value),
}

impl Default for Program {
    fn default() -> Self {
        Self::Value(Value::Null)
    }
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub enum Clause {
    #[serde(rename = "scope")]
    Scope(Scope),
    #[serde(rename = "branching")]
    Branching {
        r#if: Box<Program>,
        then: Box<Program>,
        r#else: Box<Program>,
    },
    #[serde(rename = "constant")]
    Constant(String),
    #[serde(rename = "_")]
    DefaultArgument,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct Scope {
    #[serde(default)]
    pub functions: Arc<BTreeMap<String, Program>>,
    #[serde(default)]
    pub constants: Arc<BTreeMap<String, Program>>,
    pub compute: Box<Program>,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub enum EmbeddedFunction {
    #[serde(rename = "sum")]
    Sum(Program),
    #[serde(rename = "is sorted")]
    IsSorted(Program),
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Default)]
pub enum PathSegment {
    ArrayIndex(usize),
    #[default]
    Scope,
    Compute,
    Functions,
    Function(String),
    Constants,
    Constant(String),
    Argument(String),
    Branching,
    If,
    Then,
    Else,
    Sum,
    IsSorted,
    UserFunctionCall(String),
    ObjectKey(String),
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default)]
pub struct Path(pub Vector<PathSegment>);

impl std::fmt::Debug for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let collected: Vec<&PathSegment> = self.0.inner.iter().collect();
        f.debug_tuple("Path").field(&collected).finish()
    }
}
