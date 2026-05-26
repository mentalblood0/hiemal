use url::Url;

use crate::{default_argument_name::DEFAULT_ARGUMENT_NAME, program::Program, value::Value};

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
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

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub enum DefaultArgument {
    #[serde(rename = "_")]
    Underline,
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum IncludeFrom {
    Url(Url),
    File(std::path::PathBuf),
}

#[derive(PartialEq, Debug, Clone)]
pub struct IncludeFromAt {
    pub from: IncludeFrom,
    pub at: Vec<AtSegment>,
}

impl<'de> serde::Deserialize<'de> for IncludeFromAt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut values = Vec::deserialize(deserializer)?;
        let from = if let Some(first_value) = values.pop() {
            serde_json::from_value(first_value).map_err(serde::de::Error::custom)?
        } else {
            return Err(serde::de::Error::invalid_length(
                0,
                &"at least one element (url or file path)",
            ));
        };
        let at: Vec<AtSegment> = values
            .into_iter()
            .map(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(IncludeFromAt { from, at })
    }
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub struct Include {
    #[serde(deserialize_with = "IncludeFromAt::deserialize")]
    pub include: IncludeFromAt,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
pub struct Constant {
    pub constant: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamedProgram(pub String, pub Program);

impl<'de> serde::Deserialize<'de> for NamedProgram {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct NamedProgramVisitor;

        impl<'de> serde::de::Visitor<'de> for NamedProgramVisitor {
            type Value = NamedProgram;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an object with a single key-value pair")
            }

            fn visit_map<M>(self, mut map: M) -> Result<NamedProgram, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                let (program_name, program_body) = match map.next_entry()? {
                    Some((k, v)) => (k, v),
                    None => return Err(serde::de::Error::invalid_length(0, &self)),
                };
                if map.next_entry::<String, Value>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(2, &self));
                }
                Ok(NamedProgram(program_name, program_body))
            }
        }

        deserializer.deserialize_map(NamedProgramVisitor)
    }
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub enum ProgramsListElement {
    NamedProgram(NamedProgram),
    Link(IncludeFromAt),
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub struct With {
    #[serde(default)]
    pub functions: rpds::ListSync<ProgramsListElement>,
    #[serde(default)]
    pub constants: rpds::ListSync<ProgramsListElement>,
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub struct WithCompute {
    pub with: With,
    pub compute: Program,
}

fn default_alias() -> String {
    DEFAULT_ARGUMENT_NAME.to_string()
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub struct Map {
    pub map: Program,
    #[serde(default = "default_alias")]
    pub r#as: String,
    pub through: Program,
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
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

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
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

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub struct Branching {
    pub r#if: Program,
    pub then: Program,
    pub r#else: Program,
}

fn default_error_alias() -> String {
    "error".to_string()
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub struct TryOr {
    pub r#try: Program,
    pub or: Program,
    #[serde(rename = "with error", default = "default_error_alias")]
    pub with_error: String,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum AtSegment {
    ObjectKey(String),
    ArrayIndex(usize),
}

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
pub struct FromAt {
    pub from: Program,
    pub at: rpds::ListSync<AtSegment>,
}
