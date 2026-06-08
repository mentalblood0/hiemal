use std::collections::BTreeMap;

use anyhow::{Error, Result, anyhow};

use crate::{
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    intermediate_representation::{
        self, Content, ExternalDependencies, IntermediateRepresentation,
    },
    program::{Clause, EmbeddedFunction, Path, PathSegment, Program},
    r#type::Type,
    value::Value,
};

#[derive(Clone)]
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

pub fn get_value_type(value: &Value, compilation_context: CompilationContext) -> Result<Type> {
    Ok(match value {
        Value::Number(_) => Type::Number,
        Value::String(_) => Type::String,
        Value::Bool(_) => Type::Bool,
        Value::Null => Type::Null,
        Value::Array(array) => {
            let mut element_type_option = None;
            for (element_index, element) in array.iter().enumerate() {
                let current_element_compilation_context =
                    compilation_context.extended([PathSegment::ArrayIndex(element_index)], [], []);
                let current_element_type =
                    get_value_type(element, current_element_compilation_context.clone())?;
                if let Some(element_type) = element_type_option {
                    return Err(current_element_compilation_context
                        .error(&current_element_type, &element_type));
                } else {
                    element_type_option = Some(current_element_type);
                }
            }
            if let Some(element_type) = element_type_option {
                element_type
            } else {
                return Err(anyhow!(
                    "Expected non-empty list at {:#?}",
                    compilation_context.path
                ));
            }
        }
        Value::Object(object) => {
            let mut result_inner_types = BTreeMap::new();
            for (object_key, object_value) in object {
                let current_object_value_compilation_context = compilation_context.extended(
                    [PathSegment::ObjectKey(object_key.clone())],
                    [],
                    [],
                );
                let current_object_value_type =
                    get_value_type(object_value, current_object_value_compilation_context)?;
                result_inner_types.insert(object_key.clone(), current_object_value_type);
            }
            Type::Object(result_inner_types)
        }
    })
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
                let compiled_element = compile(element, element_compilation_context.clone())?;
                if let Some(previous_element_type) = result_content.last().and_then(
                    |last_compiled_element: &IntermediateRepresentation| {
                        Some(last_compiled_element.r#type.clone())
                    },
                ) {
                    if compiled_element.r#type != previous_element_type {
                        return Err(element_compilation_context
                            .error(&compiled_element.r#type, &previous_element_type));
                    }
                }
                result_content.push(compiled_element.clone());
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
                r#type: Type::Array(Box::new(result_content.first().unwrap().r#type.clone())),
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
                    compiled_functions.push((function_name.clone(), compiled_function.clone()));
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
                    compiled_constants.push((constant_name.clone(), compiled_constant.clone()));
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
                for function_name in compiled_compute.external_dependencies.functions.keys() {
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
                IntermediateRepresentation {
                    r#type: compiled_compute.r#type.clone(),
                    content: Content::Clause(intermediate_representation::Clause::Scope(Box::new(
                        compiled_compute,
                    ))),
                    available_functions: compilation_context.available_functions,
                    available_constants: compilation_context.available_constants,
                    external_dependencies: result_external_dependencies,
                }
            }
            Clause::Branching { r#if, then, r#else } => {
                let if_compilation_context =
                    compilation_context.extended([PathSegment::Branching, PathSegment::If], [], []);
                let if_compiled = compile(r#if, if_compilation_context.clone())?;
                if if_compiled.r#type != Type::Bool {
                    return Err(if_compilation_context.error(&if_compiled.r#type, &Type::Bool));
                }
                let then_compiled = compile(
                    then,
                    compilation_context.extended(
                        [PathSegment::Branching, PathSegment::Then],
                        [],
                        [],
                    ),
                )?;
                let else_compilation_context = compilation_context.extended(
                    [PathSegment::Branching, PathSegment::Else],
                    [],
                    [],
                );
                let else_compiled = compile(r#else, else_compilation_context.clone())?;
                if else_compiled.r#type != then_compiled.r#type {
                    return Err(else_compilation_context
                        .error(&else_compiled.r#type, &then_compiled.r#type));
                }
                IntermediateRepresentation {
                    r#type: then_compiled.r#type.clone(),
                    content: Content::Clause(intermediate_representation::Clause::Branching {
                        r#if: Box::new(if_compiled.clone()),
                        then: Box::new(then_compiled.clone()),
                        r#else: Box::new(else_compiled.clone()),
                    }),
                    available_functions: compilation_context.available_functions.clone(),
                    available_constants: compilation_context.available_constants.clone(),
                    external_dependencies: if_compiled.external_dependencies.merged([
                        then_compiled.external_dependencies,
                        else_compiled.external_dependencies,
                    ]),
                }
            }
            Clause::Constant(constant_name) => {
                if let Some(compiled_constant) =
                    compilation_context.available_constants.get(constant_name)
                {
                    compiled_constant.clone()
                } else {
                    return Err(anyhow!(
                        "Got no constant with name {constant_name:?} at {:#?}",
                        compilation_context.path
                    ));
                }
            }
            Clause::DefaultArgument => compile(
                &Program::Clause(Clause::Constant(DEFAULT_ARGUMENT_NAME.to_string())),
                compilation_context,
            )?,
        },
        Program::EmbeddedFunction(embedded_function) => match &**embedded_function {
            EmbeddedFunction::Sum(argument) => {
                let argument_compilation_context =
                    compilation_context.extended([PathSegment::Sum], [], []);
                let compiled_argument = compile(&argument, argument_compilation_context.clone())?;
                let expected_type = Type::Array(Box::new(Type::Number));
                if compiled_argument.r#type != expected_type {
                    return Err(argument_compilation_context
                        .error(&compiled_argument.r#type, &expected_type));
                }
                IntermediateRepresentation {
                    r#type: Type::Number,
                    content: Content::EmbeddedFunctionCall(Box::new(
                        intermediate_representation::EmbeddedFunction::Sum(
                            compiled_argument.clone(),
                        ),
                    )),
                    available_functions: compilation_context.available_functions,
                    available_constants: compilation_context.available_constants,
                    external_dependencies: compiled_argument.external_dependencies,
                }
            }
            EmbeddedFunction::IsSorted(argument) => {
                let argument_compilation_context =
                    compilation_context.extended([PathSegment::Sum], [], []);
                let compiled_argument = compile(&argument, argument_compilation_context)?;
                if let Type::Array(_) = compiled_argument.r#type {
                } else {
                    return Err(anyhow!(
                        "Got {:?} but expected Array",
                        compiled_argument.r#type,
                    ));
                }
                IntermediateRepresentation {
                    r#type: Type::Bool,
                    content: Content::EmbeddedFunctionCall(Box::new(
                        intermediate_representation::EmbeddedFunction::Sum(
                            compiled_argument.clone(),
                        ),
                    )),
                    available_functions: compilation_context.available_functions,
                    available_constants: compilation_context.available_constants,
                    external_dependencies: compiled_argument.external_dependencies,
                }
            }
        },
        Program::Object(object) => {
            match object.len() {
                0 => {
                    return Err(anyhow!(
                        "Expected non-empty list at {:#?}",
                        compilation_context.path
                    ));
                }
                1 => {
                    let (function_name, _) = object.iter().next().unwrap();
                    if let Some(compiled_function) =
                        compilation_context.available_functions.get(function_name)
                    {
                        return Ok(compiled_function.clone());
                    }
                }
                2.. => {}
            };
            let mut result_inner_types = BTreeMap::new();
            let mut result_content = BTreeMap::new();
            let mut result_external_dependencies = ExternalDependencies {
                functions: rpds::RedBlackTreeMapSync::new_sync(),
                constants_names: rpds::RedBlackTreeSetSync::new_sync(),
            };
            for (object_key, object_value) in object.iter() {
                let object_value_compilation_context = compilation_context.extended(
                    [PathSegment::ObjectKey(object_key.clone())],
                    [],
                    [],
                );
                let compiled_object_value =
                    compile(object_value, object_value_compilation_context)?;
                result_content.insert(object_key.clone(), compiled_object_value.clone());
                result_inner_types.insert(object_key.clone(), compiled_object_value.r#type);
                for (function_name, function_body) in
                    compiled_object_value.external_dependencies.functions.iter()
                {
                    result_external_dependencies
                        .functions
                        .insert_mut(function_name.clone(), function_body.clone());
                }
                for constant_name in compiled_object_value
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
                r#type: Type::Object(result_inner_types),
                content: Content::Object(result_content),
                available_functions: compilation_context.available_functions,
                available_constants: compilation_context.available_constants,
                external_dependencies: result_external_dependencies,
            }
        }
        Program::Value(value) => IntermediateRepresentation {
            r#type: get_value_type(value, compilation_context.clone())?,
            content: Content::Value(value.clone()),
            available_functions: compilation_context.available_functions,
            available_constants: compilation_context.available_constants,
            external_dependencies: ExternalDependencies {
                functions: rpds::RedBlackTreeMapSync::new_sync(),
                constants_names: rpds::RedBlackTreeSetSync::new_sync(),
            },
        },
    })
}
