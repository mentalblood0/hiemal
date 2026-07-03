use std::{collections::BTreeMap, rc::Rc};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    containers::List, default_argument_name::DEFAULT_ARGUMENT_NAME, r#type::Type, value::Value,
};

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Program {
    Tuple(Vec<Program>),
    Scope {
        #[serde(default)]
        functions: BTreeMap<String, Rc<Program>>,
        #[serde(default)]
        constants: BTreeMap<String, Rc<Program>>,
        compute: Rc<Program>,
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
    Map {
        map: Box<Program>,
        #[serde(default = "default_map_as")]
        r#as: String,
        through: Box<Program>,
    },
    Fold {
        fold: Box<Program>,
        #[serde(default = "default_fold_as")]
        r#as: String,
        #[serde(default = "default_starting_with")]
        #[serde(rename = "starting with")]
        starting_with: Box<Program>,
        #[serde(default = "default_accumulating_in")]
        #[serde(rename = "accumulating in")]
        accumulating_in: String,
        through: Box<Program>,
    },
    Metaprogram {
        metaprogram: Box<Program>,
    },
    Sequence {
        #[serde(rename = "starting with")]
        starting_with: Box<Program>,
        #[serde(default = "default_sequence_as")]
        r#as: String,
        next: Box<Program>,
        r#while: Box<Program>,
    },
    Object(BTreeMap<String, Program>),
    Value(Option<Value>),
}

pub fn default_accumulating_in() -> String {
    "accumulator".to_string()
}

pub fn default_starting_with() -> Box<Program> {
    Box::new(Program::Value(None))
}

pub fn default_match_as() -> Option<String> {
    None
}

pub fn default_map_as() -> String {
    DEFAULT_ARGUMENT_NAME.to_string()
}

pub fn default_fold_as() -> String {
    "current".to_string()
}

pub fn default_sequence_as() -> String {
    DEFAULT_ARGUMENT_NAME.to_string()
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
    DefaultArgument(DefaultArgument),
    Url(Url),
    File(std::path::PathBuf),
    Program(Rc<Program>),
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum RangeBound {
    Static(Option<usize>),
    Dynamic(Program),
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum AtSegment {
    ProgramPathSegment(PathSegment),
    ValueArrayIndex(usize),
    ValueObjectKey(String),
    ValueArrayRange(RangeBound, RangeBound),
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
    #[serde(rename = "key-value pairs")]
    KeyValuePairs(Program),
    #[serde(rename = "flatten")]
    Flatten(Program),
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
    #[serde(rename = "map")]
    Map,
    #[serde(rename = "fold")]
    Fold,
    #[serde(rename = "metaprogram")]
    Metaprogram,
    #[serde(rename = "starting from")]
    StartingWith,
    #[serde(rename = "next")]
    Next,
    #[serde(rename = "while")]
    While,
    #[serde(rename = "through")]
    Through(usize),
    #[serde(rename = "cases")]
    Cases,
    #[serde(rename = "case")]
    Case(usize),
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
    #[serde(rename = "key-value pairs")]
    KeyValuePairs,
    #[serde(rename = "flatten")]
    Flatten,
    #[serde(rename = "user function call")]
    UserFunctionCall(String),
    #[serde(rename = "object key")]
    ObjectKey(String),
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Hash)]
pub struct Path(pub List<PathSegment>);

impl std::fmt::Debug for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let collected: Vec<&PathSegment> = self.0.inner.iter().collect();
        f.debug_tuple("Path").field(&collected).finish()
    }
}
