use std::{collections::BTreeMap, sync::Arc};

use crate::{clause::Include, intermediate_representation::IntermediateRepresentation};

#[derive(Debug, Clone)]
pub enum Clause {
    With {
        user_functions: Arc<BTreeMap<String, IntermediateRepresentation>>,
        constants: Arc<BTreeMap<String, IntermediateRepresentation>>,
        compute: Box<IntermediateRepresentation>,
    },
    Map(Box<Map>),
    Filter(Box<Filter>),
    Fold(Box<Fold>),
    Branching(Box<Branching>),
    TryOr(Box<TryOr>),
    FromAt(Box<FromAt>),
    Constant(String),
    Include(Include),
    EmbeddedFunctionCall {
        name: String,
        argument: Box<IntermediateRepresentation>,
    },
    UserFunctionCall {
        name: String,
        arguments: Box<BTreeMap<String, IntermediateRepresentation>>,
    },
}

#[derive(Debug, Clone)]
pub struct Map {
    pub map: IntermediateRepresentation,
    pub r#as: String,
    pub through: IntermediateRepresentation,
}

#[derive(Debug, Clone)]
pub struct Filter {
    pub filter: IntermediateRepresentation,
    pub r#as: String,
    pub through: IntermediateRepresentation,
}

#[derive(Debug, Clone)]
pub struct Fold {
    pub fold: IntermediateRepresentation,
    pub r#as: String,
    pub starting_with: IntermediateRepresentation,
    pub accumulating_in: String,
    pub through: IntermediateRepresentation,
}

#[derive(Debug, Clone)]
pub struct Branching {
    pub r#if: IntermediateRepresentation,
    pub then: IntermediateRepresentation,
    pub r#else: IntermediateRepresentation,
}

#[derive(Debug, Clone)]
pub struct TryOr {
    pub r#try: IntermediateRepresentation,
    pub or: IntermediateRepresentation,
    pub with_error: String,
}

#[derive(Debug, Clone)]
pub enum AtSegment {
    ObjectKey(String),
    ArrayIndex(usize),
}

#[derive(Debug, Clone)]
pub struct FromAt {
    pub from: IntermediateRepresentation,
    pub at: rpds::VectorSync<AtSegment>,
}
