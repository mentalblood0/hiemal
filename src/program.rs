use std::{collections::BTreeMap, sync::Arc};

use crate::{clause::Clause, value::Value};

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Program {
    Array(Arc<Vec<Program>>),
    Clause(Clause),
    Object(Arc<BTreeMap<String, Program>>),
    Value(Value),
}
