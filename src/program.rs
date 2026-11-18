use std::{collections::BTreeMap, hash::Hash, sync::Arc};

use bytes::Bytes;
use enumset::EnumSet;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor},
};

use crate::{
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    embedded_functions::EmbeddedFunction,
    r#type::{Capability, Type},
    value::{Value, deserialize_bytes},
};

#[repr(u8)]
#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Program {
    Tuple(Vec<Arc<ProgramWithHash>>),
    Scope {
        #[serde(default)]
        functions: BTreeMap<Arc<String>, Arc<ProgramWithHash>>,
        #[serde(default)]
        constants: BTreeMap<Arc<String>, Arc<ProgramWithHash>>,
        #[serde(default, rename = "allow")]
        allow: Option<EnumSet<Capability>>,
        #[serde(default, rename = "forbid")]
        forbid: Option<EnumSet<Capability>>,
        compute: Arc<ProgramWithHash>,
    },
    Constant {
        constant: Arc<String>,
    },
    DefaultArgument(DefaultArgument),
    EmbeddedFunctionCall(EmbeddedFunctionCall),
    Match {
        r#match: Match,
        #[serde(default = "default_match_as")]
        r#as: Option<Arc<String>>,
        cases: Vec<(Condition, ProgramWithHash)>,
    },
    Map {
        map: Box<ProgramWithHash>,
        #[serde(default = "default_map_as")]
        r#as: Arc<String>,
        through: Box<ProgramWithHash>,
    },
    Filter {
        filter: Box<ProgramWithHash>,
        #[serde(default = "default_filter_as")]
        r#as: Arc<String>,
        through: Box<ProgramWithHash>,
    },
    Flatten {
        flatten: Box<ProgramWithHash>,
    },
    Fold {
        fold: Box<ProgramWithHash>,
        #[serde(default = "default_fold_as")]
        r#as: Arc<String>,
        #[serde(default = "default_starting_with")]
        #[serde(rename = "starting with")]
        starting_with: Box<ProgramWithHash>,
        #[serde(default = "default_accumulating_in")]
        #[serde(rename = "accumulating in")]
        accumulating_in: Arc<String>,
        through: Box<ProgramWithHash>,
    },
    Metaprogram {
        metaprogram: Box<ProgramWithHash>,
    },
    Sequence {
        #[serde(rename = "starting with")]
        starting_with: Box<ProgramWithHash>,
        #[serde(default = "default_sequence_as")]
        r#as: Arc<String>,
        next: Box<ProgramWithHash>,
        #[serde(default = "default_sequence_while")]
        r#while: Option<Box<ProgramWithHash>>,
    },
    Loop {
        #[serde(rename = "starting with")]
        starting_with: Box<ProgramWithHash>,
        #[serde(default = "default_sequence_as")]
        r#as: Arc<String>,
        r#loop: Box<ProgramWithHash>,
        #[serde(default = "default_sequence_while")]
        r#while: Option<Box<ProgramWithHash>>,
    },
    Pipe {
        pipe: Vec<Arc<ProgramWithHash>>,
        #[serde(default = "default_pipe_as")]
        r#as: Option<Arc<String>>,
    },
    Try {
        r#try: Arc<ProgramWithHash>,
        #[serde(default = "default_try_as")]
        r#as: Arc<String>,
        or: Arc<ProgramWithHash>,
    },
    Bytes {
        #[serde(deserialize_with = "deserialize_bytes")]
        bytes: Bytes,
    },
    Time {
        #[serde(with = "humantime_serde")]
        time: std::time::SystemTime,
    },
    Duration {
        #[serde(with = "humantime_serde")]
        duration: std::time::Duration,
    },
    Object(BTreeMap<Arc<String>, Arc<ProgramWithHash>>),
    Value(Arc<Option<Value>>),
}

#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(from = "Program")]
#[serde(into = "Program")]
pub struct ProgramWithHash {
    pub program: Program,
    pub hash: u128,
}

impl From<Program> for ProgramWithHash {
    fn from(program: Program) -> Self {
        let mut hasher = gxhash::GxHasher::default();
        // SAFETY: `Program` is `#[repr(u8)]`, so its first byte is the discriminant.
        // We read that byte without offsetting the pointer.
        let program_value_discriminant = unsafe { *(&program as *const Program as *const u8) };
        program_value_discriminant.hash(&mut hasher);
        match &program {
            Program::Tuple(elements) => {
                for element in elements {
                    element.hash(&mut hasher);
                }
            }
            Program::Scope {
                functions,
                constants,
                allow,
                forbid,
                compute,
            } => {
                for (name, body) in functions.iter().chain(constants.iter()) {
                    name.hash(&mut hasher);
                    body.hash.hash(&mut hasher);
                }
                for capability in allow.iter().chain(forbid.iter()) {
                    capability.hash(&mut hasher);
                }
                compute.hash.hash(&mut hasher);
            }
            Program::Constant { constant } => {
                constant.hash(&mut hasher);
            }
            Program::DefaultArgument(default_argument) => {
                default_argument.hash(&mut hasher);
            }
            Program::EmbeddedFunctionCall(EmbeddedFunctionCall {
                embedded_function,
                argument,
            }) => {
                embedded_function.hash(&mut hasher);
                argument.hash.hash(&mut hasher);
            }
            Program::Match {
                r#match,
                r#as,
                cases,
            } => {
                match r#match {
                    Match::Constant(constant) => {
                        constant.hash(&mut hasher);
                    }
                    Match::Program(program_with_hash) => {
                        program_with_hash.hash(&mut hasher);
                    }
                }
                r#as.hash(&mut hasher);
                for (condition, ProgramWithHash { program: _, hash }) in cases {
                    match condition {
                        Condition::Type(r#type) => {
                            r#type.hash(&mut hasher);
                        }
                        Condition::Value(ProgramWithHash { program: _, hash }) => {
                            hash.hash(&mut hasher);
                        }
                    }
                    hash.hash(&mut hasher);
                }
            }
            Program::Map {
                map: source,
                r#as,
                through,
            }
            | Program::Filter {
                filter: source,
                r#as,
                through,
            } => {
                source.hash.hash(&mut hasher);
                r#as.hash(&mut hasher);
                through.hash.hash(&mut hasher);
            }
            Program::Flatten { flatten } => {
                flatten.hash.hash(&mut hasher);
            }
            Program::Fold {
                fold: source,
                r#as,
                starting_with,
                accumulating_in,
                through,
            } => {
                source.hash.hash(&mut hasher);
                r#as.hash(&mut hasher);
                through.hash.hash(&mut hasher);
                starting_with.hash(&mut hasher);
                accumulating_in.hash(&mut hasher);
            }
            Program::Metaprogram { metaprogram } => {
                metaprogram.hash.hash(&mut hasher);
            }
            Program::Sequence {
                starting_with,
                r#as,
                next,
                r#while,
            }
            | Program::Loop {
                starting_with,
                r#as,
                r#loop: next,
                r#while,
            } => {
                starting_with.hash(&mut hasher);
                r#as.hash(&mut hasher);
                next.hash.hash(&mut hasher);
                if let Some(r#while) = r#while {
                    r#while.hash.hash(&mut hasher);
                }
            }
            Program::Pipe { pipe, r#as } => {
                for pipe_element in pipe {
                    pipe_element.hash.hash(&mut hasher);
                }
                r#as.hash(&mut hasher);
            }
            Program::Try { r#try, r#as, or } => {
                r#try.hash.hash(&mut hasher);
                r#as.hash(&mut hasher);
                or.hash.hash(&mut hasher);
            }
            Program::Bytes { bytes } => {
                bytes.hash(&mut hasher);
            }
            Program::Time { time } => {
                time.hash(&mut hasher);
            }
            Program::Duration { duration } => {
                duration.hash(&mut hasher);
            }
            Program::Object(object) => {
                for (key, body) in object {
                    key.hash(&mut hasher);
                    body.hash.hash(&mut hasher);
                }
            }
            Program::Value(value) => {
                value.hash(&mut hasher);
            }
        };
        Self {
            program,
            hash: hasher.finish_u128(),
        }
    }
}

pub fn default_accumulating_in() -> Arc<String> {
    "accumulator".to_string().into()
}

pub fn default_starting_with() -> Box<ProgramWithHash> {
    Box::new(Program::Value(Arc::new(None)).into())
}

pub fn default_match_as() -> Option<Arc<String>> {
    None
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

pub fn default_sequence_while() -> Option<Box<ProgramWithHash>> {
    None
}

pub fn default_pipe_as() -> Option<Arc<String>> {
    None
}

pub fn default_try_as() -> Arc<String> {
    "error".to_string().into()
}

#[repr(u8)]
#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Match {
    Constant(Arc<String>),
    Program(Box<ProgramWithHash>),
}

#[repr(u8)]
#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Condition {
    Type(Type),
    Value(ProgramWithHash),
}

#[repr(u8)]
#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
pub enum DefaultArgument {
    #[serde(rename = "_")]
    Underline,
}

impl Default for Program {
    fn default() -> Self {
        Self::Value(Arc::new(None))
    }
}

impl Default for ProgramWithHash {
    fn default() -> Self {
        Self::from(Program::default())
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct EmbeddedFunctionCall {
    pub embedded_function: EmbeddedFunction,
    pub argument: Arc<ProgramWithHash>,
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
                    Option::None => return Err(serde::de::Error::invalid_length(0, &self)),
                };
                if access
                    .next_entry::<EmbeddedFunction, ProgramWithHash>()?
                    .is_some()
                {
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Default, Hash)]
pub enum PathSegment {
    #[serde(rename = "array index")]
    ArrayIndex(usize),
    #[serde(rename = "match")]
    Match,
    #[serde(rename = "map")]
    Map,
    #[serde(rename = "fold")]
    Fold,
    #[serde(rename = "pipe")]
    Pipe,
    #[serde(rename = "try")]
    Try,
    #[serde(rename = "or")]
    Or,
    #[serde(rename = "flatten")]
    Flatten,
    #[serde(rename = "filter")]
    Filter,
    #[serde(rename = "metaprogram")]
    Metaprogram,
    #[serde(rename = "starting with")]
    StartingWith,
    #[serde(rename = "next")]
    Next(Type),
    #[serde(rename = "loop")]
    Loop(Type),
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
    #[serde(rename = "regex")]
    Regex,
    #[serde(rename = "user function call")]
    UserFunctionCall(Arc<String>),
    #[serde(rename = "object key")]
    ObjectKey(Arc<String>),
}

#[derive(Serialize, Clone, PartialEq, PartialOrd, Eq, Ord, Default, Deserialize, Hash, Debug)]
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
