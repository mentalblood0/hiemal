use std::{collections::BTreeMap, sync::LazyLock};

use serde::{Deserialize, Serialize};

use crate::r#type::{TypeKind, TypeProperties};

#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash, Copy)]
pub enum EmbeddedFunction {
    #[serde(rename = "bufferized reader")]
    BufferizedReader,
    #[serde(rename = "bufferized writer")]
    BufferizedWriter,
    #[serde(rename = "bytes from string")]
    BytesFromString,
    #[serde(rename = "compile regex")]
    CompileRegex,
    #[serde(rename = "concat")]
    Concat,
    #[serde(rename = "divide")]
    Divide,
    #[serde(rename = "exponentiate")]
    Exponentiate,
    #[serde(rename = "file metadata")]
    FileMetadata,
    #[serde(rename = "glob")]
    Glob,
    #[serde(rename = "hash")]
    Hash,
    #[serde(rename = "HTTP client")]
    HttpClient,
    #[serde(rename = "is sorted")]
    IsSorted,
    #[serde(rename = "key-value pairs")]
    KeyValuePairs,
    #[serde(rename = "match regex")]
    MatchRegex,
    #[serde(rename = "mod")]
    Mod,
    #[serde(rename = "object from")]
    ObjectFrom,
    #[serde(rename = "parse")]
    Parse,
    #[serde(rename = "random number generator")]
    RandomNumberGenerator,
    #[serde(rename = "random bytes")]
    RandomBytes,
    #[serde(rename = "read bytes")]
    ReadBytes,
    #[serde(rename = "read bytes from standard input")]
    ReadBytesFromStandardInput,
    #[serde(rename = "remove file")]
    RemoveFile,
    #[serde(rename = "round")]
    Round,
    #[serde(rename = "run")]
    Run,
    #[serde(rename = "send request")]
    SendRequest,
    #[serde(rename = "serialize to")]
    SerializeTo,
    #[serde(rename = "sort")]
    Sort,
    #[serde(rename = "string from bytes")]
    StringFromBytes,
    #[serde(rename = "sleep")]
    Sleep,
    #[serde(rename = "substract")]
    Substract,
    #[serde(rename = "sum")]
    Sum,
    #[serde(rename = "write bytes")]
    WriteBytes,
}

#[derive(Deserialize, Default)]
pub struct Return {
    #[serde(default)]
    pub r#type: Option<TypeKind>,
    #[serde(default)]
    pub properties: Option<TypeProperties>,
}

#[derive(Deserialize)]
pub struct EmbeddedFunctionSignature {
    pub argument: TypeKind,
    #[serde(default = "default_argument_is_computible")]
    pub argument_is_computible: bool,
    #[serde(default)]
    pub r#return: Return,
}

pub fn default_argument_is_computible() -> bool {
    true
}

pub static EMBEDDED_FUNCTIONS: LazyLock<Vec<Vec<EmbeddedFunctionSignature>>> =
    LazyLock::new(|| {
        let mut name_and_signature_pairs = serde_saphyr::from_str::<
            BTreeMap<EmbeddedFunction, Vec<EmbeddedFunctionSignature>>,
        >(include_str!("embedded_functions.yml"))
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();
        name_and_signature_pairs.sort_by_key(|a| a.0);
        name_and_signature_pairs
            .into_iter()
            .map(|(_, signature)| signature)
            .collect()
    });
