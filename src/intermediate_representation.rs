use std::collections::BTreeMap;

use crate::{program::Path, value::Value};

#[derive(Debug, Clone)]
pub struct IntermediateRepresentation {
    pub root: Node,
    pub user_functions: Vec<Node>,
    pub constants: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub path: Path,
    pub content: Content,
}

#[derive(Debug, Clone)]
pub enum Content {
    Array(Vec<Node>),
    Clause(Clause),
    EmbeddedFunctionCall(Box<EmbeddedFunction>),
    Constant(String),
    UserFunctionCall(usize),
    Object(BTreeMap<String, Node>),
    Value(Value),
}

#[derive(Debug, Clone)]
pub enum Clause {
    Scope(Box<Node>),
    Branching {
        r#if: Box<Node>,
        then: Box<Node>,
        r#else: Box<Node>,
    },
    Constant(usize),
    DefaultArgument,
}

#[derive(Debug, Clone)]
pub enum EmbeddedFunction {
    Sum(Node),
    IsSorted(Node),
}

#[derive(Debug, Clone)]
pub struct ExternalDependencies {
    pub functions_names: rpds::RedBlackTreeSetSync<String>,
    pub constants_names: rpds::RedBlackTreeSetSync<String>,
}

impl ExternalDependencies {
    pub fn new() -> Self {
        Self {
            functions_names: rpds::RedBlackTreeSetSync::new_sync(),
            constants_names: rpds::RedBlackTreeSetSync::new_sync(),
        }
    }

    pub fn extended<P, F, C>(&self, functions: F, constants_names: C) -> Self
    where
        F: IntoIterator<Item = String>,
        C: IntoIterator<Item = String>,
    {
        Self {
            functions_names: {
                let mut result = self.functions_names.clone();
                for function in functions {
                    result.insert_mut(function);
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

    pub fn merged<E>(&self, external_dependencies: E) -> Self
    where
        E: IntoIterator<Item = ExternalDependencies>,
    {
        let mut result_functions = self.functions_names.clone();
        let mut result_constants_names = self.constants_names.clone();
        for external_dependencies_instance in external_dependencies.into_iter() {
            for function in external_dependencies_instance.functions_names.iter() {
                result_functions.insert_mut(function.clone());
            }
            for constant_name in external_dependencies_instance.constants_names.iter() {
                result_constants_names.insert_mut(constant_name.clone());
            }
        }
        Self {
            functions_names: result_functions,
            constants_names: result_constants_names,
        }
    }
}
