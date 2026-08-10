use std::{collections::BTreeMap, sync::Arc};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    containers::Vector, default_argument_name::DEFAULT_ARGUMENT_NAME, r#type::Type, value::Value,
};

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Program {
    Tuple(Vec<Program>),
    Scope {
        #[serde(default)]
        functions: BTreeMap<Arc<String>, Arc<Program>>,
        #[serde(default)]
        constants: BTreeMap<Arc<String>, Arc<Program>>,
        compute: Arc<Program>,
    },
    Constant {
        constant: Arc<String>,
    },
    DefaultArgument(DefaultArgument),
    FromAt {
        from: From,
        at: Vec<AtSegment>,
        #[serde(default = "default_from_at_default")]
        default: Box<Program>,
    },
    EmbeddedFunction(Box<EmbeddedFunction>),
    Match {
        r#match: Box<Program>,
        #[serde(default = "default_match_as")]
        r#as: Option<Arc<String>>,
        cases: Vec<(Condition, Program)>,
    },
    Map {
        map: Box<Program>,
        #[serde(default = "default_map_as")]
        r#as: Arc<String>,
        through: Box<Program>,
    },
    Filter {
        filter: Box<Program>,
        #[serde(default = "default_filter_as")]
        r#as: Arc<String>,
        through: Box<Program>,
    },
    Fold {
        fold: Box<Program>,
        #[serde(default = "default_fold_as")]
        r#as: Arc<String>,
        #[serde(default = "default_starting_with")]
        #[serde(rename = "starting with")]
        starting_with: Box<Program>,
        #[serde(default = "default_accumulating_in")]
        #[serde(rename = "accumulating in")]
        accumulating_in: Arc<String>,
        through: Box<Program>,
    },
    Metaprogram {
        metaprogram: Box<Program>,
    },
    Sequence {
        #[serde(rename = "starting with")]
        starting_with: Box<Program>,
        #[serde(default = "default_sequence_as")]
        r#as: Arc<String>,
        next: Box<Program>,
    },
    Object(BTreeMap<Arc<String>, Arc<Program>>),
    Value(Arc<Option<Value>>),
}

pub fn default_accumulating_in() -> Arc<String> {
    "accumulator".to_string().into()
}

pub fn default_starting_with() -> Box<Program> {
    Box::new(Program::Value(Arc::new(None)))
}

pub fn default_match_as() -> Option<Arc<String>> {
    None
}

pub fn default_from_at_default() -> Box<Program> {
    Box::new(Program::Value(None.into()))
}

pub fn default_map_as() -> Arc<String> {
    DEFAULT_ARGUMENT_NAME.to_string().into()
}

pub fn default_filter_as() -> Arc<String> {
    DEFAULT_ARGUMENT_NAME.to_string().into()
}

pub fn default_fold_as() -> Arc<String> {
    "current".to_string().into()
}

pub fn default_sequence_as() -> Arc<String> {
    DEFAULT_ARGUMENT_NAME.to_string().into()
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
    Program(Arc<Program>),
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
    ValueArrayRange((Box<RangeBound>, Box<RangeBound>)),
}

impl Default for Program {
    fn default() -> Self {
        Self::Value(Arc::new(None))
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
    #[serde(rename = "is match")]
    IsMatch { string: Program, regex: Program },
    #[serde(rename = "match groups")]
    MatchGroups { string: Program, regex: Program },
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
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "match")]
    Match,
    #[serde(rename = "map")]
    Map,
    #[serde(rename = "filter")]
    Filter,
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
    Function(Arc<String>),
    #[serde(rename = "constants")]
    Constants,
    #[serde(rename = "constant")]
    Constant(Arc<String>),
    #[serde(rename = "argument")]
    Argument(Arc<String>),
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
    #[serde(rename = "is match")]
    IsMatch,
    #[serde(rename = "match groups")]
    MatchGroups,
    #[serde(rename = "string")]
    String,
    #[serde(rename = "regex")]
    Regex,
    #[serde(rename = "user function call")]
    UserFunctionCall(Arc<String>),
    #[serde(rename = "object key")]
    ObjectKey(Arc<String>),
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Hash, Debug)]
pub struct Path(pub Vector<PathSegment>);
