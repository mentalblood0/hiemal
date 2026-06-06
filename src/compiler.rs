use anyhow::{Error, Result, anyhow};

use crate::{
    intermediate_representation::{
        self, Content, ExternalDependencies, IntermediateRepresentation,
    },
    program::{Clause, Path, PathSegment, Program},
    r#type::Type,
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
    mut compilation_context: CompilationContext,
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
            let mut result_external_dependencies = ExternalDependencies {
                functions: rpds::RedBlackTreeMapSync::new_sync(),
                constants_names: rpds::RedBlackTreeSetSync::new_sync(),
            };
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
                for (function_name, function_body) in
                    compiled_element.external_dependencies.functions.iter()
                {
                    result_external_dependencies
                        .functions
                        .insert_mut(function_name.clone(), function_body.clone());
                }
                for constant_name in compiled_element
                    .external_dependencies
                    .constants_names
                    .iter()
                {
                    result_external_dependencies
                        .constants_names
                        .insert_mut(constant_name.clone());
                }
            }
            IntermediateRepresentation {
                r#type: Type::Array(Box::new(result_content.first().unwrap().r#type)),
                content: Content::Array(result_content),
                available_functions: compilation_context.available_functions,
                available_constants: compilation_context.available_constants,
                external_dependencies: result_external_dependencies,
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
                    )?;
                    compiled_functions.push((function_name.clone(), compiled_function));
                    compilation_context
                        .available_functions
                        .insert_mut(function_name.clone(), compiled_function);
                }
                let mut compiled_constants = Vec::with_capacity(constants.len());
                for (constant_name, constant_compute_body) in constants.iter() {
                    let compiled_constant = compile(
                        constant_compute_body,
                        compilation_context.extended(
                            [PathSegment::Scope, PathSegment::Compute],
                            [],
                            [],
                        ),
                    )?;
                    compiled_constants.push((constant_name.clone(), compiled_constant));
                    compilation_context
                        .available_constants
                        .insert_mut(constant_name.clone(), compiled_constant);
                }
                let compiled_compute = compile(
                    compute,
                    compilation_context.extended(
                        [PathSegment::Scope, PathSegment::Compute],
                        compiled_functions,
                        compiled_constants,
                    ),
                )?;
                let mut result_external_dependencies =
                    compiled_compute.external_dependencies.clone();
                for (function_name, function_body) in
                    compiled_compute.external_dependencies.functions.iter()
                {
                    if compilation_context
                        .available_functions
                        .contains_key(function_name)
                    {
                        result_external_dependencies
                            .functions
                            .remove_mut(function_name);
                    }
                }
                for constant_name in compiled_compute
                    .external_dependencies
                    .constants_names
                    .iter()
                {
                    if compilation_context
                        .available_constants
                        .contains_key(constant_name)
                    {
                        result_external_dependencies
                            .constants_names
                            .remove_mut(constant_name);
                    }
                }
                let mut result = IntermediateRepresentation {
                    r#type: compiled_compute.r#type,
                    content: Content::Clause(intermediate_representation::Clause::Scope(Box::new(
                        compiled_compute,
                    ))),
                    available_functions: compilation_context.available_functions,
                    available_constants: compilation_context.available_constants,
                    external_dependencies: result_external_dependencies,
                };
                result
            }
        },
    })
}
