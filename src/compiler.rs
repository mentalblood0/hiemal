use std::collections::BTreeMap;

use anyhow::{Error, Result, anyhow};

use crate::{
    containers::Map,
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    intermediate_representation::{
        self, Content, ExternalDependencies, IntermediateRepresentation, Node,
    },
    program::{Clause, EmbeddedFunction, Path, PathSegment, Program, Scope},
    r#type::Type,
    value::Value,
};

#[derive(Clone, Default)]
pub struct CompilationContext {
    pub path: Path,
    pub available_functions: Map<String, usize>,
    pub available_constants: Map<String, usize>,
    pub entered_user_functions: rpds::RedBlackTreeSetSync<usize>,
}

impl CompilationContext {
    pub fn error(&self, got_type: &Type, expected_type: &Type) -> Error {
        anyhow!(
            "Got {got_type:#?} but expected {expected_type:#?} at {:#?}",
            self.path,
        )
    }
}

fn get_value_type(value: &Value, compilation_context: CompilationContext) -> Result<Type> {
    Ok(match value {
        Value::Number(_) => Type::Number,
        Value::String(_) => Type::String,
        Value::Bool(_) => Type::Bool,
        Value::Null => Type::Null,
        Value::Array(array) => {
            let mut element_type_option = None;
            for (element_index, element) in array.inner.iter().enumerate() {
                let mut current_element_compilation_context = compilation_context.clone();
                current_element_compilation_context
                    .path
                    .0
                    .extend([PathSegment::ArrayIndex(element_index)]);
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
            for (object_key, object_value) in object.inner.iter() {
                let mut current_object_value_compilation_context = compilation_context.clone();
                current_object_value_compilation_context
                    .path
                    .0
                    .extend([PathSegment::ObjectKey(object_key.clone())]);
                let current_object_value_type =
                    get_value_type(object_value, current_object_value_compilation_context)?;
                result_inner_types.insert(object_key.clone(), current_object_value_type);
            }
            Type::Object(result_inner_types)
        }
    })
}

fn resolve_types(
    got_type: &Type,
    expected_type: &Type,
    compilation_context: &CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
) -> Result<()> {
    match (got_type, expected_type) {
        (Type::Number, Type::Number)
        | (Type::String, Type::String)
        | (Type::Bool, Type::Bool)
        | (Type::Null, Type::Null) => Ok(()),
        (Type::Array(got_element_type), Type::Array(expected_element_type)) => resolve_types(
            got_element_type,
            expected_element_type,
            compilation_context,
            global_compilation_context,
        ),
        (Type::Object(got_element_inner_types), Type::Object(expected_element_inner_types)) => {
            for (expected_value_key, expected_value_type) in expected_element_inner_types {
                if let Some(got_value_type) = got_element_inner_types.get(expected_value_key) {
                    resolve_types(
                        got_value_type,
                        expected_value_type,
                        compilation_context,
                        global_compilation_context,
                    )?;
                } else {
                    return Err(compilation_context.error(got_type, expected_type));
                }
            }
            Ok(())
        }
        (Type::Unknown(got_program), expected_type)
        | (expected_type, Type::Unknown(got_program)) => {
            if let Some((_, Some(previously_resolved_type))) = global_compilation_context
                .user_function_to_index_and_type_option
                .get(got_program)
            {
                if previously_resolved_type != expected_type {
                    return Err(compilation_context.error(got_type, expected_type));
                }
            } else {
                global_compilation_context
                    .user_function_to_index_and_type_option
                    .get_mut(got_program)
                    .unwrap()
                    .1 = Some(expected_type.clone());
            }
            Ok(())
        }
        _ => Err(compilation_context.error(got_type, expected_type)),
    }
}

#[derive(Default)]
pub struct GlobalCompilationContext {
    pub user_function_to_index_and_type_option: BTreeMap<Program, (usize, Option<Type>)>,
    pub user_functions: Vec<Content>,
    pub constant_to_index: BTreeMap<Program, usize>,
    pub constants: Vec<Content>,
}

pub fn compile(program: &Program) -> Result<IntermediateRepresentation> {
    let mut global_compilation_context = GlobalCompilationContext::default();
    Ok(IntermediateRepresentation {
        root: compile_with_context(
            program,
            CompilationContext::default(),
            &mut global_compilation_context,
        )?
        .1,
        user_functions: global_compilation_context.user_functions,
        constants: global_compilation_context.constants,
    })
}

fn compile_with_context(
    program: &Program,
    compilation_context: CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
) -> Result<(Type, Node)> {
    Ok(match program {
        Program::Array(array) => {
            if array.is_empty() {
                return Err(anyhow!(
                    "Expected non-empty array at {:#?}",
                    compilation_context.path
                ));
            }
            let mut result_content = Vec::with_capacity(array.len());
            let mut previous_element_type_option = None;
            for (element_index, element) in array.iter().enumerate() {
                let mut element_compilation_context = compilation_context.clone();
                element_compilation_context
                    .path
                    .0
                    .extend([PathSegment::ArrayIndex(element_index)]);
                let (element_type, element_node) = compile_with_context(
                    element,
                    element_compilation_context.clone(),
                    global_compilation_context,
                )?;
                if let Some(previous_element_type) = previous_element_type_option {
                    resolve_types(
                        &element_type,
                        &previous_element_type,
                        &compilation_context,
                        global_compilation_context,
                    )?;
                }
                result_content.push(element_node);
            }
            (
                Type::Array(Box::new(previous_element_type_option.unwrap())),
                Node {
                    path: compilation_context.path.clone(),
                    content: Content::Array(result_content),
                },
            )
        }
        Program::Clause(clause) => match clause {
            Clause::Scope(Scope {
                functions,
                constants,
                compute,
            }) => {
                let mut compiled_constants = Vec::with_capacity(constants.len());
                for (constant_name, constant_compute_body) in constants.iter() {
                    let compiled_constant = compile_with_context(
                        constant_compute_body,
                        compilation_context.extended(
                            [
                                PathSegment::Scope,
                                PathSegment::Constants,
                                PathSegment::Constant(constant_name.clone()),
                            ],
                            [],
                            [],
                            [],
                        ),
                    )?;
                    compiled_constants.push((constant_name.clone(), compiled_constant));
                }
                let mut compiled_functions = Vec::with_capacity(functions.len());
                for (function_name, function_body) in functions.iter() {
                    if !function_name.ends_with(":") {
                        let function_compilation_context = compilation_context.extended(
                            [
                                PathSegment::Functions,
                                PathSegment::Function(function_name.clone()),
                            ],
                            [],
                            [],
                            [],
                        );
                        return Err(anyhow!(
                            "Got function named {function_name:?}, but expect function named {:?} \
                             at {:#?}",
                            format!("{function_name}:"),
                            function_compilation_context.path
                        ));
                    }
                    compiled_functions.push((function_name.clone(), function_body.clone()));
                }
                let compute_compilation_context = compilation_context.extended(
                    [PathSegment::Scope, PathSegment::Compute],
                    compiled_functions,
                    compiled_constants,
                    [],
                );
                let compiled_compute = compile_with_context(compute, compute_compilation_context)?;
                let mut result_external_dependencies =
                    compiled_compute.external_dependencies.clone();
                for function_name in compiled_compute
                    .external_dependencies
                    .functions_names
                    .iter()
                {
                    if compilation_context
                        .available_functions
                        .contains_key(function_name)
                    {
                        result_external_dependencies
                            .functions_names
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
                        compiled_compute.clone(),
                    ))),
                    available_functions: compilation_context.available_functions,
                    available_constants: compilation_context.available_constants,
                    external_dependencies: result_external_dependencies,
                    resolved_types: compiled_compute.resolved_types,
                }
            }
            Clause::Branching { r#if, then, r#else } => {
                let if_compilation_context = compilation_context.extended(
                    [PathSegment::Branching, PathSegment::If],
                    [],
                    [],
                    [],
                );
                let if_compiled = compile_with_context(r#if, if_compilation_context.clone())?;
                if if_compiled.r#type != Type::Bool {
                    return Err(if_compilation_context.error(&if_compiled.r#type, &Type::Bool));
                }
                let then_compiled = compile_with_context(
                    then,
                    compilation_context.extended(
                        [PathSegment::Branching, PathSegment::Then],
                        [],
                        [],
                        [],
                    ),
                )?;
                let else_compilation_context = compilation_context.extended(
                    [PathSegment::Branching, PathSegment::Else],
                    [],
                    [],
                    [],
                );
                let else_compiled = compile_with_context(r#else, else_compilation_context.clone())?;
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
                    resolved_types: extended_map(
                        &if_compiled.resolved_types,
                        else_compiled
                            .resolved_types
                            .iter()
                            .chain(then_compiled.resolved_types.iter()),
                    ),
                }
            }
            Clause::Constant(constant_name) => {
                if let Some(compiled_constant) =
                    compilation_context.available_constants.get(constant_name)
                {
                    IntermediateRepresentation {
                        r#type: compiled_constant.r#type.clone(),
                        content: Content::Constant(constant_name.clone()),
                        available_functions: compilation_context.available_functions.clone(),
                        available_constants: compilation_context.available_constants.clone(),
                        external_dependencies: ExternalDependencies {
                            functions_names: rpds::RedBlackTreeSetSync::new_sync(),
                            constants_names: rpds::RedBlackTreeSetSync::from_iter([
                                constant_name.clone()
                            ]),
                        },
                        resolved_types: rpds::RedBlackTreeMapSync::new_sync(),
                    }
                } else {
                    return Err(anyhow!(
                        "Got no constant {constant_name:?} at {:#?}, available constants are {:?}",
                        compilation_context.path,
                        compilation_context
                            .available_constants
                            .keys()
                            .collect::<Vec<_>>()
                    ));
                }
            }
            Clause::DefaultArgument => compile_with_context(
                &Program::Clause(Clause::Constant(DEFAULT_ARGUMENT_NAME.to_string())),
                compilation_context,
            )?,
        },
        Program::EmbeddedFunction(embedded_function) => match &**embedded_function {
            EmbeddedFunction::Sum(argument) => {
                let argument_compilation_context =
                    compilation_context.extended([PathSegment::Sum], [], [], []);
                let compiled_argument =
                    compile_with_context(&argument, argument_compilation_context.clone())?;
                let expected_type = Type::Array(Box::new(Type::Number));
                let mut result_resolved_types = rpds::RedBlackTreeMapSync::new_sync();
                resolve_types(
                    &compiled_argument.r#type,
                    &expected_type,
                    &compilation_context,
                    &mut result_resolved_types,
                )?;
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
                    resolved_types: result_resolved_types,
                }
            }
            EmbeddedFunction::IsSorted(argument) => {
                let argument_compilation_context =
                    compilation_context.extended([PathSegment::Sum], [], [], []);
                let compiled_argument =
                    compile_with_context(&argument, argument_compilation_context)?;
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
                    resolved_types: rpds::RedBlackTreeMapSync::new_sync(),
                }
            }
        },
        Program::Object(object) => {
            match object.len() {
                0 => {
                    return Err(anyhow!(
                        "Expected non-empty object at {:#?}",
                        compilation_context.path
                    ));
                }
                1 => {
                    let (function_name, function_argument) = object.iter().next().unwrap();
                    if function_name.ends_with(":") {
                        if let Some(function_body) =
                            compilation_context.available_functions.get(function_name)
                        {
                            let argument_compilation_context = compilation_context.extended(
                                [PathSegment::UserFunctionCall(function_name.clone())],
                                [],
                                [],
                                [function_body.clone()],
                            );
                            let compiled_argument = compile_with_context(
                                function_argument,
                                argument_compilation_context.clone(),
                            )?;
                            for required_constant_name in compiled_argument
                                .external_dependencies
                                .constants_names
                                .iter()
                            {
                                if !argument_compilation_context
                                    .available_constants
                                    .contains_key(required_constant_name)
                                {
                                    return Err(anyhow!(
                                        "Got function call which requires constant \
                                         {required_constant_name:?} but no such constant available"
                                    ));
                                }
                            }
                            let mut function_body_available_constants_from_arguments = Vec::new();
                            if let Content::Object(function_arguments) = compiled_argument.content {
                                if function_arguments.len() > 1 {
                                    function_body_available_constants_from_arguments.push((
                                        DEFAULT_ARGUMENT_NAME.to_string(),
                                        function_arguments.values().next().unwrap().clone(),
                                    ));
                                } else {
                                    for (function_argument_name, function_argument_body) in
                                        function_arguments.iter()
                                    {
                                        function_body_available_constants_from_arguments.push((
                                            function_argument_name.clone(),
                                            function_argument_body.clone(),
                                        ));
                                    }
                                }
                            } else {
                                function_body_available_constants_from_arguments
                                    .push((DEFAULT_ARGUMENT_NAME.to_string(), compiled_argument));
                            }
                            if compilation_context
                                .entered_user_functions
                                .contains(&function_body)
                            {
                                return Ok(IntermediateRepresentation {
                                    r#type: Type::Unknown(function_body.clone()),
                                    content: Content::RecursedUserFunctionCall(
                                        function_name.clone(),
                                    ),
                                    available_functions: compilation_context
                                        .available_functions
                                        .clone(),
                                    available_constants: compilation_context
                                        .extended(
                                            [],
                                            [],
                                            function_body_available_constants_from_arguments
                                                .clone(),
                                            [],
                                        )
                                        .available_constants,
                                    external_dependencies: ExternalDependencies::new(),
                                    resolved_types: rpds::RedBlackTreeMapSync::new_sync(),
                                });
                            } else {
                                let compiled_function_body = compile_with_context(
                                    function_body,
                                    argument_compilation_context.extended(
                                        [],
                                        [],
                                        function_body_available_constants_from_arguments.clone(),
                                        [],
                                    ),
                                )?;
                                let mut result_external_dependencies =
                                    compiled_function_body.external_dependencies.clone();
                                for (constant_name, _) in
                                    function_body_available_constants_from_arguments
                                {
                                    result_external_dependencies
                                        .constants_names
                                        .remove_mut(&constant_name);
                                }
                                return Ok(IntermediateRepresentation {
                                    r#type: compiled_function_body.r#type.clone(),
                                    content: Content::UserFunctionCall(Box::new(
                                        compiled_function_body.clone(),
                                    )),
                                    available_functions: compiled_function_body.available_functions,
                                    available_constants: compiled_function_body.available_constants,
                                    external_dependencies: result_external_dependencies,
                                    resolved_types: compiled_function_body.resolved_types.clone(),
                                });
                            }
                        } else {
                            return Err(anyhow!(
                                "Got no function {function_name:?} at {:#?}, available functions \
                                 are {:?}",
                                compilation_context.path,
                                compilation_context
                                    .available_functions
                                    .keys()
                                    .collect::<Vec<_>>()
                            ));
                        }
                    }
                }
                2.. => {}
            };
            let mut result_inner_types = BTreeMap::new();
            let mut result_content = BTreeMap::new();
            let mut result_external_dependencies = ExternalDependencies {
                functions_names: rpds::RedBlackTreeSetSync::new_sync(),
                constants_names: rpds::RedBlackTreeSetSync::new_sync(),
            };
            let mut result_resolved_types = rpds::RedBlackTreeMapSync::new_sync();
            for (object_key, object_value) in object.iter() {
                let object_value_compilation_context = compilation_context.extended(
                    [PathSegment::ObjectKey(object_key.clone())],
                    [],
                    [],
                    [],
                );
                let compiled_object_value =
                    compile_with_context(object_value, object_value_compilation_context)?;
                result_content.insert(object_key.clone(), compiled_object_value.clone());
                result_inner_types.insert(object_key.clone(), compiled_object_value.r#type);
                extend_set(
                    &mut result_external_dependencies.functions_names,
                    compiled_object_value
                        .external_dependencies
                        .functions_names
                        .iter(),
                );
                extend_set(
                    &mut result_external_dependencies.constants_names,
                    compiled_object_value
                        .external_dependencies
                        .constants_names
                        .iter(),
                );
                extend_map(
                    &mut result_resolved_types,
                    compiled_object_value.resolved_types.iter(),
                );
            }
            IntermediateRepresentation {
                r#type: Type::Object(result_inner_types),
                content: Content::Object(result_content),
                available_functions: compilation_context.available_functions,
                available_constants: compilation_context.available_constants,
                external_dependencies: result_external_dependencies,
                resolved_types: result_resolved_types,
            }
        }
        Program::Value(value) => IntermediateRepresentation {
            r#type: get_value_type(value, compilation_context.clone())?,
            content: Content::Value(value.clone()),
            available_functions: compilation_context.available_functions,
            available_constants: compilation_context.available_constants,
            external_dependencies: ExternalDependencies {
                functions_names: rpds::RedBlackTreeSetSync::new_sync(),
                constants_names: rpds::RedBlackTreeSetSync::new_sync(),
            },
            resolved_types: rpds::RedBlackTreeMapSync::new_sync(),
        },
    })
}
