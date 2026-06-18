use std::collections::BTreeMap;

use crate::{containers::Vector, value::Value};

#[derive(serde::Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord)]
#[serde(untagged)]
pub enum Program {
    Array(Vec<Program>),
    Scope {
        #[serde(default)]
        functions: BTreeMap<String, Program>,
        #[serde(default)]
        constants: BTreeMap<String, Program>,
        compute: Box<Program>,
    },
    Branching(Box<Branching>),
    Constant {
        constant: String,
    },
    Shortcut(Shortcut),
    EmbeddedFunction(Box<EmbeddedFunction>),
    Object(BTreeMap<String, Program>),
    Value(Value),
}

#[derive(serde::Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord)]
pub struct Branching {
    pub r#if: Program,
    pub then: Program,
    #[serde(default)]
    pub r#else: Program,
}

#[derive(serde::Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord)]
pub enum Shortcut {
    #[serde(rename = "_")]
    DefaultArgument,
}

impl Default for Program {
    fn default() -> Self {
        Self::Value(Value::Null)
    }
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
