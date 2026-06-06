use std::collections::BTreeMap;

use crate::{r#type::Type, value::Value};

#[derive(Debug, Clone)]
pub struct IntermediateRepresentation {
    pub r#type: Type,
    pub content: Content,
    pub available_functions: rpds::RedBlackTreeMapSync<String, IntermediateRepresentation>,
    pub available_constants: rpds::RedBlackTreeMapSync<String, IntermediateRepresentation>,
    pub external_dependencies: ExternalDependencies,
}

#[derive(Debug, Clone)]
pub enum Content {
    Array(Vec<IntermediateRepresentation>),
    Clause(Clause),
    EmbeddedFunctionCall(Box<EmbeddedFunction>),
    UserFunctionCall(Box<IntermediateRepresentation>),
    Object(BTreeMap<String, IntermediateRepresentation>),
    Value(Value),
}

#[derive(Debug, Clone)]
pub enum Clause {
    Scope(Box<IntermediateRepresentation>),
    Branching {
        r#if: Box<IntermediateRepresentation>,
        then: Box<IntermediateRepresentation>,
        r#else: Box<IntermediateRepresentation>,
    },
    Constant(String),
    DefaultArgument,
}

#[derive(Debug, Clone)]
pub enum EmbeddedFunction {
    Sum(IntermediateRepresentation),
    IsSorted(IntermediateRepresentation),
}

#[derive(Debug, Clone)]
pub struct ExternalDependencies {
    pub functions: rpds::RedBlackTreeMapSync<String, IntermediateRepresentation>,
    pub constants_names: rpds::RedBlackTreeSetSync<String>,
}

impl ExternalDependencies {
    pub fn extended<P, F, C>(&self, functions: F, constants_names: C) -> Self
    where
        F: IntoIterator<Item = (String, IntermediateRepresentation)>,
        C: IntoIterator<Item = String>,
    {
        Self {
            functions: {
                let mut result = self.functions.clone();
                for function in functions {
                    result.insert_mut(function.0, function.1);
                }
                result
            },
            constants_names: {
                let mut result = self.constants_names.clone();
                for constant_name in constants_names {
                    result.insert_mut(constant_name);
                }
                result
            },
        }
    }
}
