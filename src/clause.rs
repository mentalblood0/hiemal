use std::{collections::VecDeque, sync::Arc};

use url::Url;

use crate::{
    default_argument_name::DEFAULT_ARGUMENT_NAME, path::Path, program::Program, value::SmallMap,
};

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Clause {
    With(Box<WithCompute>),
    Map(Box<Map>),
    Filter(Box<Filter>),
    Fold(Box<Fold>),
    Branching(Box<Branching>),
    TryOr(Box<TryOr>),
    FromAt(Box<FromAt>),
    Constant(Constant),
    DefaultArgument(DefaultArgument),
    Include(Include),
}

#[derive(serde::Deserialize, Debug, Clone)]
pub enum DefaultArgument {
    #[serde(rename = "_")]
    Underline,
}

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum IncludeFrom {
    Url(Url),
    File(std::path::PathBuf),
}

#[derive(Debug, Clone)]
pub struct IncludeFromAt {
    pub from: IncludeFrom,
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
        Ok(IncludeFromAt { from, at: Path(at) })
    }
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Include {
    #[serde(deserialize_with = "IncludeFromAt::deserialize")]
    pub include: IncludeFromAt,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct Constant {
    pub constant: String,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct With {
    #[serde(default)]
    pub functions: Arc<SmallMap<String, Program>>,
    #[serde(default)]
    pub constants: Arc<SmallMap<String, Program>>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct WithCompute {
    pub with: With,
    pub compute: Program,
}

fn default_alias() -> String {
    DEFAULT_ARGUMENT_NAME.to_string()
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Map {
    pub map: Program,
    #[serde(default = "default_alias")]
    pub r#as: String,
    pub through: Program,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Filter {
    pub filter: Program,
    #[serde(default = "default_alias")]
    pub r#as: String,
    pub through: Program,
}

fn default_current_value_alias() -> String {
    "current".to_string()
}

fn default_accumulator_value_alias() -> String {
    "accumulator".to_string()
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Fold {
    pub fold: Program,
    #[serde(default = "default_current_value_alias")]
    pub r#as: String,
    #[serde(rename = "starting with")]
    pub starting_with: Program,
    #[serde(
        rename = "accumulating in",
        default = "default_accumulator_value_alias"
    )]
    pub accumulating_in: String,
    pub through: Program,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Branching {
    pub r#if: Program,
    pub then: Program,
    pub r#else: Program,
}

fn default_error_alias() -> String {
    "error".to_string()
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct TryOr {
    pub r#try: Program,
    pub or: Program,
    #[serde(rename = "with error", default = "default_error_alias")]
    pub with_error: String,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum AtSegment {
    ObjectKey(String),
    ArrayIndex(usize),
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct FromAt {
    pub from: Program,
    pub at: rpds::VectorSync<AtSegment>,
}
