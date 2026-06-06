use anyhow::{Error, Result, anyhow};
use serde_json::ser::CompactFormatter;

use crate::{
    intermediate_representation::{Content, ExternalDependencies, IntermediateRepresentation},
    program::{Clause, Path, PathSegment, Program},
    r#type::Type,
    value::Value,
};

pub struct CompilationContext {
    pub path: Path,
    pub available_functions: rpds::RedBlackTreeMapSync<String, IntermediateRepresentation>,
    pub available_constants: rpds::RedBlackTreeMapSync<String, IntermediateRepresentation>,
}

impl CompilationContext {
    pub fn extended<P, F, C>(&self, path: P, available_functions: F, available_constants: C) -> Self
    where
        P: IntoIterator<Item = PathSegment>,
        F: IntoIterator<Item = (String, IntermediateRepresentation)>,
        C: IntoIterator<Item = (String, IntermediateRepresentation)>,
    {
        Self {
            path: {
                let mut result = self.path.clone();
                result.0.extend(path);
                result
            },
            available_functions: {
                let mut result = self.available_functions.clone();
                for function in available_functions {
                    result.insert_mut(function.0, function.1);
                }
                result
            },
            available_constants: {
                let mut result = self.available_constants.clone();
                for constant in available_constants {
                    result.insert_mut(constant.0, constant.1);
                }
                result
            },
        }
    }

    pub fn error(&self, got_type: &Type, expected_type: &Type) -> Error {
        anyhow!(
            "Got {got_type:?} but expected {expected_type:?} at {:#?}",
            self.path,
        )
    }
}

pub fn compile(
    program: &Program,
    compilation_context: CompilationContext,
) -> Result<IntermediateRepresentation> {
    Ok(match program {
        Program::Array(array) => {
            if array.is_empty() {
                return Err(anyhow!(
                    "Expected non-empty list at {:#?}",
                    compilation_context.path
                ));
            }
            let mut result_content = Vec::with_capacity(array.len());
            for (element_index, element) in array.iter().enumerate() {
                let element_compilation_context =
                    compilation_context.extended([PathSegment::ArrayIndex(element_index)], [], []);
                let compiled_element = compile(element, element_compilation_context)?;
                if let Some(previous_element_type) = result_content.last().and_then(
                    |last_compiled_element: &IntermediateRepresentation| {
                        Some(last_compiled_element.r#type)
                    },
                ) {
                    if compiled_element.r#type != previous_element_type {
                        return Err(element_compilation_context
                            .error(&compiled_element.r#type, &previous_element_type));
                    }
                }
                result_content.push(compiled_element);
            }
            let mut external_dependencies = ExternalDependencies {
                functions: rpds::RedBlackTreeMapSync::new_sync(),
                constants_names: rpds::VectorSync::new_sync(),
            };
            for compiled_element in result_content {
                for (function_name, function_body) in
                    compiled_element.external_dependencies.functions.iter()
                {
                    external_dependencies
                        .functions
                        .insert_mut(function_name.clone(), function_body.clone());
                }
                for constant_name in compiled_element
                    .external_dependencies
                    .constants_names
                    .iter()
                {
                    external_dependencies
                        .constants_names
                        .push_back_mut(constant_name.clone());
                }
            }
            IntermediateRepresentation {
                content: Content::Array(result_content),
                external_dependencies,
            }
        }
        Program::Clause(clause) => match clause {
            Clause::Scope {
                functions,
                constants,
                compute,
            } => {
                let mut compiled_functions = Vec::with_capacity(functions.len());
                for (function_name, function_body) in functions.iter() {
                    let compiled_function = compile(
                        function_body,
                        compilation_context.extended(
                            [PathSegment::Scope, PathSegment::Compute],
                            [],
                            [],
                        ),
                    );
                }
                let mut compiled_constants = Vec::with_capacity(constants.len());
                for (constant_name, constant_compute_body) in constants.iter() {
                    let compiled_function = compile(
                        constant_compute_body,
                        compilation_context.extended(
                            [PathSegment::Scope, PathSegment::Compute],
                            [],
                            [],
                        ),
                    );
                }
                let compute_compilation_context = compilation_context.extended(
                    [PathSegment::Scope, PathSegment::Compute],
                    compiled_functions,
                    compiled_constants,
                );
                let mut result = IntermediateRepresentation {
                    r#type: Type,
                    content: (),
                    external_dependencies: (),
                };
                result
            }
        },
    })
}
