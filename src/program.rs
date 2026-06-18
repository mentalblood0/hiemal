use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{containers::Vector, value::Value};

#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord)]
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
    DefaultArgument(DefaultArgument),
    Include {
        #[serde(deserialize_with = "IncludeFromAt::deserialize")]
        include: IncludeFromAt,
    },
    EmbeddedFunction(Box<EmbeddedFunction>),
    Object(BTreeMap<String, Program>),
    Value(Value),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord)]
pub struct Branching {
    pub r#if: Program,
    pub then: Program,
    #[serde(default)]
    pub r#else: Program,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord)]
pub enum DefaultArgument {
    #[serde(rename = "_")]
    Underline,
}

#[derive(Serialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord)]
pub struct IncludeFromAt {
    pub from: IncludeFrom,
    #[serde(default)]
    pub at: Path,
}

impl<'de> serde::Deserialize<'de> for IncludeFromAt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut values = VecDeque::deserialize(deserializer)?;
        let from = if let Some(first_value) = values.pop_front() {
            serde_json::from_value(first_value).map_err(serde::de::Error::custom)?
        } else {
            return Err(serde::de::Error::invalid_length(
                0,
                &"at least one element (url or file path)",
            ));
        };
        let mut at = rpds::VectorSync::new_sync();
        for value in values {
            at.push_back_mut(serde_json::from_value(value).map_err(serde::de::Error::custom)?);
        }
        Ok(IncludeFromAt {
            from,
            at: Path(Vector { inner: at }),
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord)]
#[serde(untagged)]
pub enum IncludeFrom {
    Url(Url),
    File(std::path::PathBuf),
}

impl Default for Program {
    fn default() -> Self {
        Self::Value(Value::Null)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub enum EmbeddedFunction {
    #[serde(rename = "sum")]
    Sum(Program),
    #[serde(rename = "is sorted")]
    IsSorted(Program),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Default)]
pub enum PathSegment {
    #[serde(rename = "array index")]
    ArrayIndex(usize),
    #[serde(rename = "compute")]
    #[default]
    Compute,
    #[serde(rename = "functions")]
    Functions,
    #[serde(rename = "function")]
    Function(String),
    #[serde(rename = "constants")]
    Constants,
    #[serde(rename = "constant")]
    Constant(String),
    #[serde(rename = "argument")]
    Argument(String),
    #[serde(rename = "if")]
    If,
    #[serde(rename = "then")]
    Then,
    #[serde(rename = "else")]
    Else,
    #[serde(rename = "sum")]
    Sum,
    #[serde(rename = "is sorted")]
    IsSorted,
    #[serde(rename = "user function call")]
    UserFunctionCall(String),
    #[serde(rename = "object key")]
    ObjectKey(String),
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize)]
pub struct Path(pub Vector<PathSegment>);

impl std::fmt::Debug for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let collected: Vec<&PathSegment> = self.0.inner.iter().collect();
        f.debug_tuple("Path").field(&collected).finish()
    }
}
