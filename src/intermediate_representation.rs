use std::{collections::BTreeMap, sync::Arc};

use crate::{intermediate_representation_clause::Clause, value::Value};

#[derive(Debug, Clone)]
pub enum IntermediateRepresentation {
    Array(Arc<Vec<IntermediateRepresentation>>),
    Clause(Clause),
    Object(Arc<BTreeMap<String, IntermediateRepresentation>>),
    Value(Value),
}
