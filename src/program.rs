use std::{collections::BTreeMap, sync::Arc};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{MapAccess, Visitor},
};
use url::Url;

use crate::{default_argument_name::DEFAULT_ARGUMENT_NAME, r#type::Type, value::Value};

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
    EmbeddedFunctionCall(EmbeddedFunctionCall),
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
    BytesValue {
        bytes: String,
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

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct EmbeddedFunctionCall {
    pub embedded_function: EmbeddedFunction,
    pub argument: Arc<Program>,
}

impl Serialize for EmbeddedFunctionCall {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(&self.embedded_function, &self.argument)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for EmbeddedFunctionCall {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EFVisitor;

        impl<'de> Visitor<'de> for EFVisitor {
            type Value = EmbeddedFunctionCall;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a map with a single key-value pair")
            }

            fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let (name, argument) = match access.next_entry()? {
                    Some((key, value)) => (key, value),
                    None => return Err(serde::de::Error::invalid_length(0, &self)),
                };
                if access.next_entry::<EmbeddedFunction, Program>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(2, &self));
                }
                Ok(EmbeddedFunctionCall {
                    embedded_function: name,
                    argument,
                })
            }
        }

        deserializer.deserialize_map(EFVisitor)
    }
}

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash, Copy)]
pub enum EmbeddedFunction {
    #[serde(rename = "sum")]
    Sum,
    #[serde(rename = "is sorted")]
    IsSorted,
    #[serde(rename = "standard input")]
    ReadBytesFromStandardInput,
    #[serde(rename = "parse yaml")]
    ParseYaml,
    #[serde(rename = "key-value pairs")]
    KeyValuePairs,
    #[serde(rename = "flatten")]
    Flatten,
    #[serde(rename = "match groups")]
    MatchGroups,
    #[serde(rename = "concat")]
    Concat,
    #[serde(rename = "read bytes from file")]
    ReadBytesFromFile,
    #[serde(rename = "string from bytes")]
    StringFromBytes,
    #[serde(rename = "write to file")]
    WriteToFile,
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
    #[serde(rename = "embedded function call")]
    EmbeddedFunctionCall(EmbeddedFunction),
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
pub struct Path(pub Vec<PathSegment>);

impl Path {
    pub fn extended<A>(&self, addition: A) -> Self
    where
        A: IntoIterator<Item = PathSegment>,
    {
        let mut result = self.clone();
        for addition_element in addition {
            result.0.push(addition_element);
        }
        result
    }
}
