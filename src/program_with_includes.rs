use std::collections::{BTreeMap, VecDeque};
use url::Url;

use crate::clause::AtSegment;

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
        let mut values = VecDeque::deserialize(deserializer)?;
        let from = if let Some(first_value) = values.pop_front() {
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

#[derive(serde::Deserialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum ProgramWithIncludes {
    Array(Vec<ProgramWithIncludes>),
    Include(Include),
    Object(BTreeMap<String, ProgramWithIncludes>),
    Other(serde_json::Value),
}
