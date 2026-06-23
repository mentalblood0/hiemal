use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{containers::Vector, r#type::Type, value::Value};

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Program {
    Tuple(Vec<Program>),
    Scope {
        #[serde(default)]
        functions: BTreeMap<String, Program>,
        #[serde(default)]
        constants: BTreeMap<String, Program>,
        compute: Box<Program>,
    },
    Constant {
        constant: String,
    },
    DefaultArgument(DefaultArgument),
    FromAt {
        from: From,
        at: Vec<AtSegment>,
    },
    EmbeddedFunction(Box<EmbeddedFunction>),
    Match {
        r#match: Box<Program>,
        #[serde(default = "default_match_as")]
        r#as: Option<String>,
        cases: Vec<(Condition, Program)>,
    },
    Object(BTreeMap<String, Program>),
    Value(Option<Value>),
}

pub fn default_match_as() -> Option<String> {
    None
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Condition {
    Type(Type),
    Value(Program),
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
pub enum DefaultArgument {
    #[serde(rename = "_")]
    Underline,
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum From {
    Url(Url),
    File(std::path::PathBuf),
    Program(Box<Program>),
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum AtSegment {
    ProgramPathSegment(PathSegment),
    ValueArrayIndex(usize),
    ValueObjectKey(String),
}

impl Default for Program {
    fn default() -> Self {
        Self::Value(None)
    }
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub enum EmbeddedFunction {
    #[serde(rename = "sum")]
    Sum(Program),
    #[serde(rename = "is sorted")]
    IsSorted(Program),
    #[serde(rename = "standard input")]
    StandardInput,
    #[serde(rename = "parse yaml")]
    ParseYaml(Program),
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Default, Hash)]
pub enum PathSegment {
    #[serde(rename = "array index")]
    ArrayIndex(usize),
    #[serde(rename = "from")]
    From,
    #[serde(rename = "at")]
    At,
    #[serde(rename = "match")]
    Match,
    #[serde(rename = "cases")]
    Cases,
    #[serde(rename = "case")]
    Case(Condition),
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
    #[serde(rename = "standard input")]
    StandardInput,
    #[serde(rename = "parse yaml")]
    ParseYaml,
    #[serde(rename = "user function call")]
    UserFunctionCall(String),
    #[serde(rename = "object key")]
    ObjectKey(String),
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Hash)]
pub struct Path(pub Vector<PathSegment>);

impl std::fmt::Debug for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let collected: Vec<&PathSegment> = self.0.inner.iter().collect();
        f.debug_tuple("Path").field(&collected).finish()
    }
}
