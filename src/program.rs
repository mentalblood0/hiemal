use crate::{clause::Clause, value::Value};

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Program {
    Array(rpds::VectorSync<Program>),
    Clause(Clause),
    Object(rpds::RedBlackTreeMapSync<String, Program>),
    Value(Value),
}
