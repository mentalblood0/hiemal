use core::todo;
use std::{collections::BTreeMap, hash::Hash, sync::Arc};

use bytes::Bytes;
use enumset::EnumSet;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, IgnoredAny, MapAccess, Visitor},
};

use crate::{
    containers::Set,
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    embedded_functions::EmbeddedFunction,
    r#type::{Capability, Type},
    value::{Value, deserialize_bytes},
};

#[repr(u8)]
#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Program {
    Tuple(Vec<Arc<ProgramAndMetadata>>),
    Scope {
        #[serde(default)]
        define: Define,
        #[serde(default, rename = "allow")]
        allow: Option<EnumSet<Capability>>,
        #[serde(default, rename = "forbid")]
        forbid: Option<EnumSet<Capability>>,
        compute: Arc<ProgramAndMetadata>,
    },
    Constant(Constant),
    FunctionCall(FunctionCall),
    Match {
        r#match: Match,
        #[serde(default = "default_match_as")]
        r#as: Option<Arc<String>>,
        cases: Vec<(Condition, ProgramAndMetadata)>,
    },
    Map {
        map: Arc<ProgramAndMetadata>,
        #[serde(default = "default_map_as")]
        r#as: Arc<String>,
        through: Arc<ProgramAndMetadata>,
    },
    Filter {
        filter: Arc<ProgramAndMetadata>,
        #[serde(default = "default_filter_as")]
        r#as: Arc<String>,
        through: Arc<ProgramAndMetadata>,
    },
    Flatten {
        flatten: Arc<ProgramAndMetadata>,
    },
    Fold {
        fold: Arc<ProgramAndMetadata>,
        #[serde(default = "default_fold_as")]
        r#as: Arc<String>,
        #[serde(default = "default_starting_with")]
        #[serde(rename = "starting with")]
        starting_with: Arc<ProgramAndMetadata>,
        #[serde(default = "default_accumulating_in")]
        #[serde(rename = "accumulating in")]
        accumulating_in: Arc<String>,
        through: Arc<ProgramAndMetadata>,
    },
    Metaprogram {
        metaprogram: Arc<ProgramAndMetadata>,
    },
    Sequence {
        #[serde(rename = "starting with")]
        starting_with: Arc<ProgramAndMetadata>,
        #[serde(default = "default_sequence_as")]
        r#as: Arc<String>,
        next: Arc<ProgramAndMetadata>,
        #[serde(default = "default_sequence_while")]
        r#while: Option<Arc<ProgramAndMetadata>>,
    },
    Loop {
        #[serde(rename = "starting with")]
        starting_with: Arc<ProgramAndMetadata>,
        #[serde(default = "default_sequence_as")]
        r#as: Arc<String>,
        r#loop: Arc<ProgramAndMetadata>,
        #[serde(default = "default_sequence_while")]
        r#while: Option<Arc<ProgramAndMetadata>>,
    },
    Pipe {
        pipe: Vec<Arc<ProgramAndMetadata>>,
        #[serde(default = "default_pipe_as")]
        r#as: Option<Arc<String>>,
    },
    Try {
        r#try: Arc<ProgramAndMetadata>,
        #[serde(default = "default_try_as")]
        r#as: Arc<String>,
        or: Arc<ProgramAndMetadata>,
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
    Object(BTreeMap<Arc<String>, Arc<ProgramAndMetadata>>),
    Value(Arc<Option<Value>>),
}

#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(from = "Program")]
#[serde(into = "Program")]
pub struct ProgramAndMetadata {
    pub program: Program,
    pub hash: u128,
    pub compiles_easily: bool,
    pub value_only: bool,
    pub external_constants_names: Set<String>,
    pub external_functions_names: Set<UserFunctionPath>,
}

impl ProgramAndMetadata {
    pub fn write_external_names(&mut self, available_functions: &mut BTreeMap<Arc<UserFunctionPath>, Arc<ProgramAndMetadata>>) {
        match &mut self.program {
            Program::Tuple(elements) => {
                for element in elements {
                    unsafe { Arc::get_mut_unchecked(&mut *element) }.write_external_names(available_functions);
                    self.external_constants_names.extend(element.external_constants_names.iter().cloned());
                    self.external_functions_names.extend(element.external_functions_names.iter().cloned());
                }
            }
            Program::Scope { define, compute, .. } => {
                for (_, constant) in define.constants_mut() {
                    unsafe { Arc::get_mut_unchecked(&mut *constant) }.write_external_names(available_functions);
                    self.external_constants_names.extend(constant.external_constants_names.iter().cloned());
                    self.external_functions_names.extend(constant.external_functions_names.iter().cloned());
                }
                available_functions.extend(define.functions().map(|(key, value)| (key.clone(), value.clone())));
                unsafe { Arc::get_mut_unchecked(&mut *compute) }.write_external_names(available_functions);
                for function_name in define.functions().map(|(name, _)| name) {
                    available_functions.remove(function_name);
                }
                let mut compute_external_constants_names_without_constants = compute.external_constants_names.clone();
                for constant_name in define.constants().map(|(name, _)| name) {
                    compute_external_constants_names_without_constants.remove(constant_name);
                }
                self.external_constants_names.extend(compute_external_constants_names_without_constants.iter().cloned());
                let mut compute_external_functions_names_without_functions = compute.external_functions_names.clone();
                for function_name in define.functions().map(|(name, _)| name) {
                    compute_external_functions_names_without_functions.remove(function_name);
                }
                self.external_functions_names.extend(compute_external_functions_names_without_functions.iter().cloned());
            }
            Program::Constant(Constant { constant }) => {
                self.external_constants_names.insert(constant.clone());
            }
            Program::FunctionCall(FunctionCall { function, argument }) => match &function {
                Function::Embedded(_) => {
                    unsafe { Arc::get_mut_unchecked(&mut *argument) }.write_external_names(available_functions);
                    self.external_constants_names.extend(argument.external_constants_names.iter().cloned());
                    self.external_functions_names.extend(argument.external_functions_names.iter().cloned());
                }
                Function::User(user_function_name) => {
                    self.external_functions_names.insert(user_function_name.clone());
                    if let Some(mut user_function) = available_functions.get(user_function_name).cloned() {
                        unsafe { Arc::get_mut_unchecked(&mut *argument) }.write_external_names(available_functions);
                        self.external_constants_names.extend(argument.external_constants_names.iter().cloned());
                        self.external_functions_names.extend(argument.external_functions_names.iter().cloned());
                        unsafe { Arc::get_mut_unchecked(&mut user_function) }.write_external_names(available_functions);
                        let mut user_function_external_constants_names_without_argument = user_function.external_constants_names.clone();
                        match &argument.program {
                            Program::Object(arguments) => {
                                for argument_name in arguments.keys() {
                                    user_function_external_constants_names_without_argument.remove(argument_name);
                                }
                            }
                            _ => {
                                user_function_external_constants_names_without_argument.remove(&DEFAULT_ARGUMENT_NAME);
                            }
                        }
                        self.external_constants_names.extend(user_function_external_constants_names_without_argument.iter().cloned());
                        self.external_functions_names.extend(user_function.external_functions_names.iter().cloned());
                    }
                }
            },
            Program::Match { r#match, r#as, cases } => {
                match r#match {
                    Match::Constant(constant) => {
                        self.external_constants_names.insert(constant.clone());
                    }
                    Match::Program(program_and_metadata) => {
                        unsafe { Arc::get_mut_unchecked(&mut *program_and_metadata) }.write_external_names(available_functions);
                        self.external_constants_names.extend(program_and_metadata.external_constants_names.iter().cloned());
                        self.external_functions_names.extend(program_and_metadata.external_functions_names.iter().cloned());
                    }
                }
                for case in cases.iter_mut() {
                    if let Condition::Value(condition_program_and_metadata) = &mut case.0 {
                        condition_program_and_metadata.write_external_names(available_functions);
                        self.external_constants_names.extend(condition_program_and_metadata.external_constants_names.iter().cloned());
                        self.external_functions_names.extend(condition_program_and_metadata.external_functions_names.iter().cloned());
                    }
                    case.1.write_external_names(available_functions);
                    self.external_constants_names.extend(
                        case.1.external_constants_names.iter().filter(|case_external_constant_name| Some(case_external_constant_name) != r#as.as_ref().as_ref()).cloned(),
                    );
                    self.external_functions_names.extend(case.1.external_functions_names.iter().cloned());
                }
            }
            Program::Map { map: source, r#as, through } | Program::Filter { filter: source, r#as, through } => {
                unsafe { Arc::get_mut_unchecked(&mut *source) }.write_external_names(available_functions);
                self.external_constants_names.extend(source.external_constants_names.iter().cloned());
                self.external_functions_names.extend(source.external_functions_names.iter().cloned());
                unsafe { Arc::get_mut_unchecked(&mut *through) }.write_external_names(available_functions);
                self.external_constants_names
                    .extend(through.external_constants_names.iter().filter(|through_external_constant_name| through_external_constant_name != &r#as).cloned());
                self.external_functions_names.extend(through.external_functions_names.iter().cloned());
            }
            Program::Flatten { flatten } => {
                unsafe { Arc::get_mut_unchecked(&mut *flatten) }.write_external_names(available_functions);
                self.external_constants_names.extend(flatten.external_constants_names.iter().cloned());
                self.external_functions_names.extend(flatten.external_functions_names.iter().cloned());
            }
            Program::Fold {
                fold: source,
                r#as,
                starting_with,
                accumulating_in,
                through,
            } => {
                unsafe { Arc::get_mut_unchecked(&mut *starting_with) }.write_external_names(available_functions);
                self.external_constants_names.extend(starting_with.external_constants_names.iter().cloned());
                self.external_functions_names.extend(starting_with.external_functions_names.iter().cloned());
                unsafe { Arc::get_mut_unchecked(&mut *source) }.write_external_names(available_functions);
                self.external_constants_names.extend(source.external_constants_names.iter().cloned());
                self.external_functions_names.extend(source.external_functions_names.iter().cloned());
                unsafe { Arc::get_mut_unchecked(&mut *through) }.write_external_names(available_functions);
                self.external_constants_names.extend(
                    through
                        .external_constants_names
                        .iter()
                        .filter(|through_external_constant_name| through_external_constant_name != &r#as && through_external_constant_name != &accumulating_in)
                        .cloned(),
                );
                self.external_functions_names.extend(through.external_functions_names.iter().cloned());
            }
            Program::Metaprogram { .. } => {
                todo!();
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
                unsafe { Arc::get_mut_unchecked(&mut *starting_with) }.write_external_names(available_functions);
                self.external_constants_names.extend(starting_with.external_constants_names.iter().cloned());
                self.external_functions_names.extend(starting_with.external_functions_names.iter().cloned());
                unsafe { Arc::get_mut_unchecked(&mut *next) }.write_external_names(available_functions);
                self.external_constants_names
                    .extend(next.external_constants_names.iter().filter(|next_external_constant_name| next_external_constant_name != &r#as).cloned());
                self.external_functions_names.extend(next.external_functions_names.iter().cloned());
                if let Some(r#while) = r#while {
                    unsafe { Arc::get_mut_unchecked(&mut *r#while) }.write_external_names(available_functions);
                    self.external_constants_names
                        .extend(r#while.external_constants_names.iter().filter(|next_external_constant_name| next_external_constant_name != &r#as).cloned());
                    self.external_functions_names.extend(r#while.external_functions_names.iter().cloned());
                }
            }
            Program::Pipe { pipe, r#as } => {
                let mut is_first = true;
                for element in pipe {
                    unsafe { Arc::get_mut_unchecked(&mut *element) }.write_external_names(available_functions);
                    self.external_constants_names.extend(
                        element
                            .external_constants_names
                            .iter()
                            .filter(|element_external_constant_name| is_first || Some(element_external_constant_name) != r#as.as_ref().as_ref())
                            .cloned(),
                    );
                    self.external_functions_names.extend(element.external_functions_names.iter().cloned());
                    is_first = false;
                }
            }
            Program::Try { r#try, r#as, or } => {
                unsafe { Arc::get_mut_unchecked(&mut *r#try) }.write_external_names(available_functions);
                self.external_constants_names.extend(r#try.external_constants_names.iter().cloned());
                self.external_functions_names.extend(r#try.external_functions_names.iter().cloned());
                unsafe { Arc::get_mut_unchecked(&mut *or) }.write_external_names(available_functions);
                self.external_constants_names.extend(or.external_constants_names.iter().filter(|or_external_constant_name| or_external_constant_name != &r#as).cloned());
                self.external_functions_names.extend(or.external_functions_names.iter().cloned());
            }
            Program::Object(object) => {
                for program_and_metadata in object.values_mut() {
                    unsafe { Arc::get_mut_unchecked(&mut *program_and_metadata) }.write_external_names(available_functions);
                    self.external_constants_names.extend(program_and_metadata.external_constants_names.iter().cloned());
                    self.external_functions_names.extend(program_and_metadata.external_functions_names.iter().cloned());
                }
            }
            Program::Bytes { .. } | Program::Time { .. } | Program::Duration { .. } | Program::Value(_) => {}
        }
    }
}

impl From<Program> for ProgramAndMetadata {
    fn from(program: Program) -> Self {
        let mut compiles_easily = true;
        let mut value_only = true;
        let mut hasher = gxhash::GxHasher::default();
        // SAFETY: `Program` is `#[repr(u8)]`, so its first byte is the discriminant.
        // We read that byte without offsetting the pointer.
        let program_value_discriminant = unsafe { *(&program as *const Program as *const u8) };
        program_value_discriminant.hash(&mut hasher);
        match &program {
            Program::Tuple(elements) => {
                for element in elements {
                    compiles_easily &= element.compiles_easily;
                    value_only &= element.value_only;
                    element.hash.hash(&mut hasher);
                }
            }
            Program::Scope { define, allow, forbid, compute } => {
                compiles_easily = false;
                value_only = false;
                for (name, body) in define.functions() {
                    name.hash(&mut hasher);
                    body.hash.hash(&mut hasher);
                }
                for (name, body) in define.constants() {
                    name.hash(&mut hasher);
                    body.hash.hash(&mut hasher);
                }
                for capability in allow.iter().chain(forbid.iter()) {
                    capability.hash(&mut hasher);
                }
                compute.hash.hash(&mut hasher);
            }
            Program::Constant(Constant { constant }) => {
                value_only = false;
                constant.hash(&mut hasher);
            }
            Program::FunctionCall(FunctionCall { function, argument }) => {
                compiles_easily = false;
                value_only = false;
                function.hash(&mut hasher);
                argument.hash.hash(&mut hasher);
            }
            Program::Match { r#match, r#as, cases } => {
                compiles_easily = false;
                value_only = false;
                match r#match {
                    Match::Constant(constant) => {
                        constant.hash(&mut hasher);
                    }
                    Match::Program(program_and_metadata) => {
                        program_and_metadata.hash.hash(&mut hasher);
                    }
                }
                r#as.hash(&mut hasher);
                for (condition, program_and_metadata) in cases {
                    match condition {
                        Condition::Type(r#type) => {
                            r#type.hash(&mut hasher);
                        }
                        Condition::Value(program_and_metadata) => {
                            program_and_metadata.hash.hash(&mut hasher);
                        }
                    }
                    program_and_metadata.hash.hash(&mut hasher);
                }
            }
            Program::Map { map: source, r#as, through } | Program::Filter { filter: source, r#as, through } => {
                compiles_easily = false;
                value_only = false;
                source.hash.hash(&mut hasher);
                r#as.hash(&mut hasher);
                through.hash.hash(&mut hasher);
            }
            Program::Flatten { flatten } => {
                compiles_easily = false;
                value_only = false;
                flatten.hash.hash(&mut hasher);
            }
            Program::Fold {
                fold: source,
                r#as,
                starting_with,
                accumulating_in,
                through,
            } => {
                compiles_easily = false;
                value_only = false;
                source.hash.hash(&mut hasher);
                r#as.hash(&mut hasher);
                through.hash.hash(&mut hasher);
                starting_with.hash.hash(&mut hasher);
                accumulating_in.hash(&mut hasher);
            }
            Program::Metaprogram { metaprogram } => {
                compiles_easily = false;
                value_only = false;
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
                compiles_easily = false;
                starting_with.hash.hash(&mut hasher);
                r#as.hash(&mut hasher);
                next.hash.hash(&mut hasher);
                if let Some(r#while) = r#while {
                    r#while.hash.hash(&mut hasher);
                }
            }
            Program::Pipe { pipe, r#as } => {
                compiles_easily = false;
                value_only = false;
                for pipe_element in pipe {
                    pipe_element.hash.hash(&mut hasher);
                }
                r#as.hash(&mut hasher);
            }
            Program::Try { r#try, r#as, or } => {
                compiles_easily = false;
                value_only = false;
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
                    compiles_easily &= body.compiles_easily;
                    value_only &= body.value_only;
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
            compiles_easily,
            value_only,
            external_constants_names: Set::default(),
            external_functions_names: Set::default(),
        }
    }
}

pub fn default_accumulating_in() -> Arc<String> {
    "accumulator".to_string().into()
}

pub fn default_starting_with() -> Arc<ProgramAndMetadata> {
    Arc::new(Program::Value(Arc::new(None)).into())
}

pub fn default_match_as() -> Option<Arc<String>> {
    None
}

pub fn default_map_as() -> Arc<String> {
    DEFAULT_ARGUMENT_NAME.clone()
}

pub fn default_filter_as() -> Arc<String> {
    DEFAULT_ARGUMENT_NAME.clone()
}

pub fn default_fold_as() -> Arc<String> {
    "current".to_string().into()
}

pub fn default_sequence_as() -> Arc<String> {
    DEFAULT_ARGUMENT_NAME.clone()
}

pub fn default_sequence_while() -> Option<Arc<ProgramAndMetadata>> {
    None
}

pub fn default_pipe_as() -> Option<Arc<String>> {
    None
}

pub fn default_try_as() -> Arc<String> {
    "error".to_string().into()
}

#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash, Default)]
#[serde(transparent)]
pub struct Define(pub BTreeMap<DefinedName, Arc<ProgramAndMetadata>>);

impl Define {
    pub fn constants(&self) -> impl Iterator<Item = (&Arc<String>, &std::sync::Arc<ProgramAndMetadata>)> {
        self.0.iter().filter_map(|(defined_name, value)| {
            if let DefinedName::Constant(name) = defined_name {
                Some((name, value))
            } else {
                Option::None
            }
        })
    }

    pub fn constants_mut(&mut self) -> impl Iterator<Item = (&Arc<String>, &mut std::sync::Arc<ProgramAndMetadata>)> {
        self.0.iter_mut().filter_map(|(defined_name, value)| {
            if let DefinedName::Constant(name) = defined_name {
                Some((name, value))
            } else {
                Option::None
            }
        })
    }

    pub fn functions(&self) -> impl Iterator<Item = (&Arc<UserFunctionPath>, &std::sync::Arc<ProgramAndMetadata>)> {
        self.0.iter().filter_map(|(defined_name, value)| {
            if let DefinedName::Function(user_function_path) = defined_name {
                Some((user_function_path, value))
            } else {
                Option::None
            }
        })
    }

    pub fn functions_mut(&mut self) -> impl Iterator<Item = (&Arc<UserFunctionPath>, &mut std::sync::Arc<ProgramAndMetadata>)> {
        self.0.iter_mut().filter_map(|(defined_name, value)| {
            if let DefinedName::Function(user_function_path) = defined_name {
                Some((user_function_path, value))
            } else {
                Option::None
            }
        })
    }
}

pub type UserFunctionPath = Vec<Arc<String>>;

#[repr(u8)]
#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum DefinedName {
    Function(Arc<UserFunctionPath>),
    Constant(Arc<String>),
}

#[repr(u8)]
#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Match {
    Constant(Arc<String>),
    Program(Arc<ProgramAndMetadata>),
}

#[repr(u8)]
#[derive(Deserialize, Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Condition {
    Type(Type),
    Value(ProgramAndMetadata),
}

#[derive(Debug, Clone, PartialOrd, PartialEq, Eq, Ord, Hash)]
pub struct Constant {
    pub constant: Arc<String>,
}

impl<'de> Deserialize<'de> for Constant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Inner {
            constant: Arc<String>,
        }

        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Str(String),
            Struct(Inner),
        }

        match Helper::deserialize(deserializer)? {
            Helper::Str(s) if s == **DEFAULT_ARGUMENT_NAME => Ok(Constant { constant: Arc::new(s) }),
            Helper::Str(s) => Err(de::Error::invalid_value(de::Unexpected::Str(&s), &"\"_\" or a struct with a `constant` field")),
            Helper::Struct(inner) => Ok(Constant { constant: inner.constant }),
        }
    }
}

impl Default for Program {
    fn default() -> Self {
        Self::Value(Arc::new(None))
    }
}

impl Default for ProgramAndMetadata {
    fn default() -> Self {
        Self::from(Program::default())
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
#[serde(untagged)]
pub enum Function {
    Embedded((EmbeddedFunction,)),
    User(Arc<UserFunctionPath>),
}

impl Function {
    pub fn as_embedded(&self) -> Option<&EmbeddedFunction> {
        match self {
            Function::Embedded((result,)) => Some(result),
            _ => None,
        }
    }

    pub fn as_user(&self) -> Option<&Arc<UserFunctionPath>> {
        match self {
            Function::User(result) => Some(result),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct FunctionCall {
    pub function: Function,
    pub argument: Arc<ProgramAndMetadata>,
}

impl<'de> Deserialize<'de> for FunctionCall {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UserFunctionCallVisitor;

        impl<'de> Visitor<'de> for UserFunctionCallVisitor {
            type Value = FunctionCall;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a map with a single key-value pair where key is a one-string tuple and value is a program")
            }

            fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let (function, argument) = match access.next_entry::<Function, Arc<ProgramAndMetadata>>()? {
                    Some((key, value)) => (key, value),
                    Option::None => return Err(serde::de::Error::invalid_length(0, &self)),
                };
                if access.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(2, &self));
                }
                Ok(FunctionCall { function, argument })
            }
        }

        deserializer.deserialize_map(UserFunctionCallVisitor)
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
    UserFunctionCall(Arc<UserFunctionPath>),
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
