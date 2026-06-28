use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Error, Result, anyhow};

use crate::{
    containers::{Map, Set},
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    includes_cache::IncludesCache,
    intermediate_representation::{
        self, Case, Content, IntermediateRepresentation, MapThroughs, Node, UserFunction,
    },
    program::{AtSegment, Condition, EmbeddedFunction, From, Path, PathSegment, Program},
    r#type::Type,
    value::Value,
};

#[derive(Clone, Default)]
struct CompilationContext {
    path: Path,
    available_functions: Map<String, Program>,
    available_constants: Map<String, usize>,
    entered_user_functions: Set<Program>,
}

impl CompilationContext {
    fn error(&self, got_type: &Type, expected_type: &Type) -> Error {
        anyhow!(
            "expected {expected_type:#?}, found {got_type:#?} at {:#?}",
            self.path,
        )
    }
}

#[derive(Debug, Clone, Ord, PartialEq, PartialOrd, Eq)]
pub struct ConstantDefinition {
    pub name_clustered_index: usize,
    pub index: usize,
}

fn assert_contains(
    got_type: &Type,
    expected_type: &Type,
    compilation_context: &CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
) -> Result<()> {
    if expected_type == &Type::Any || expected_type.contains(got_type) {
        Ok(())
    } else {
        match (got_type, expected_type) {
            (Type::Array(got_element_type), Type::Array(expected_element_type)) => assert_contains(
                got_element_type,
                expected_element_type,
                compilation_context,
                global_compilation_context,
            ),
            (Type::Array(got_element_type), Type::Tuple(expected_elements_types)) => {
                for expected_element_type in expected_elements_types {
                    assert_contains(
                        got_element_type,
                        expected_element_type,
                        compilation_context,
                        global_compilation_context,
                    )?
                }
                Ok(())
            }
            (Type::Tuple(got_elements_types), Type::Array(expected_element_type)) => {
                for got_element_type in got_elements_types {
                    assert_contains(
                        got_element_type,
                        expected_element_type,
                        compilation_context,
                        global_compilation_context,
                    )?
                }
                Ok(())
            }
            (Type::Object(got_inner_types), Type::Object(expected_inner_types)) => {
                for (expected_value_key, expected_value_type) in expected_inner_types {
                    if let Some(got_value_type) = got_inner_types.get(expected_value_key) {
                        assert_contains(
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
            (Type::Unknown(_), Type::Unknown(_)) => Ok(()),
            (Type::Unknown(got_program_index), expected_type)
            | (expected_type, Type::Unknown(got_program_index)) => {
                match &global_compilation_context.user_functions[*got_program_index].1 {
                    ProgramOrNode::Program(got_program) => {
                        let previously_resolved_type = &global_compilation_context
                            .user_function_to_index_and_type_option
                            .get(got_program)
                            .unwrap()
                            .1;
                        if let Type::Unknown(_) = previously_resolved_type {
                            global_compilation_context
                                .user_function_to_index_and_type_option
                                .get_mut(got_program)
                                .unwrap()
                                .1 = expected_type.clone();
                        } else {
                            if previously_resolved_type != expected_type {
                                return Err(compilation_context
                                    .error(&previously_resolved_type, expected_type));
                            }
                        }
                    }
                    ProgramOrNode::Node(got_node) => {
                        let previously_resolved_type = &global_compilation_context
                            .user_function_node_to_index_and_type_option
                            .get(got_node)
                            .unwrap()
                            .1;
                        if let Type::Unknown(_) = previously_resolved_type {
                            global_compilation_context
                                .user_function_node_to_index_and_type_option
                                .get_mut(got_node)
                                .unwrap()
                                .1 = expected_type.clone();
                        } else {
                            if previously_resolved_type != expected_type {
                                return Err(compilation_context
                                    .error(&previously_resolved_type, expected_type));
                            }
                        }
                    }
                }
                Ok(())
            }
            (Type::Union(got_union_types), Type::Union(expected_union_types)) => {
                if !got_union_types.is_subset(expected_union_types) {
                    for one_of_got_types in got_union_types {
                        let mut found = false;
                        for one_of_expected_types in expected_union_types {
                            if assert_contains(
                                one_of_got_types,
                                one_of_expected_types,
                                compilation_context,
                                global_compilation_context,
                            )
                            .is_ok()
                            {
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            return Err(compilation_context.error(got_type, expected_type));
                        }
                    }
                }
                Ok(())
            }
            (Type::Union(got_union_types), expected_type) => {
                for one_of_got_types in got_union_types {
                    assert_contains(
                        one_of_got_types,
                        expected_type,
                        compilation_context,
                        global_compilation_context,
                    )?;
                }
                Ok(())
            }
            (got_type, Type::Union(expected_union_types)) => {
                if !expected_union_types.contains(expected_type) {
                    for one_of_expected_types in expected_union_types {
                        if assert_contains(
                            got_type,
                            one_of_expected_types,
                            compilation_context,
                            global_compilation_context,
                        )
                        .is_ok()
                        {
                            return Ok(());
                        }
                    }
                    return Err(compilation_context.error(got_type, expected_type));
                }
                Ok(())
            }
            (Type::Literal(got_value), expected_type) => assert_contains(
                &Value::r#type(got_value),
                expected_type,
                compilation_context,
                global_compilation_context,
            ),
            _ => Err(compilation_context.error(got_type, expected_type)),
        }
    }
}

#[repr(u8)]
#[derive(Debug)]
enum ProgramOrNode {
    Program(Program),
    Node(Node),
}

impl ProgramOrNode {
    fn as_node(&self) -> Option<&Node> {
        match self {
            &ProgramOrNode::Node(ref node) => Some(node),
            &ProgramOrNode::Program(_) => None,
        }
    }
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct NodeAndMetadata {
    node: Node,
    r#type: Type,
    external_constants_name_clustered_indices: BTreeSet<usize>,
    is_pure: bool,
}

#[derive(Default)]
struct GlobalCompilationContext {
    user_function_to_index_and_type_option: BTreeMap<Program, (usize, Type)>,
    user_function_node_to_index_and_type_option: BTreeMap<Node, (usize, Type)>,
    user_functions: Vec<(Vec<usize>, ProgramOrNode, bool)>,
    constants_names_to_name_clustered_constants_indices: BTreeMap<String, usize>,
    constants: Vec<Type>,
    includes_cache: IncludesCache,
}

fn process_from_at_program_path_part(
    from: &From,
    at: &Vec<AtSegment>,
    includes_cache: &mut IncludesCache,
) -> Result<(Program, Option<usize>)> {
    let mut result = includes_cache.get(&from)?;
    let mut current_path_segment_index = 0;
    while let Some(current_path_segment) = at.get(current_path_segment_index) {
        match current_path_segment {
            AtSegment::ProgramPathSegment(program_path_segment) => {
                match (result, program_path_segment) {
                    (Program::Tuple(tuple), PathSegment::ArrayIndex(tuple_index)) => {
                        result = tuple.get(*tuple_index).unwrap().clone();
                    }
                    (Program::Object(object), PathSegment::ObjectKey(object_key)) => {
                        result = object.get(object_key).unwrap().clone();
                    }
                    (
                        Program::Value(Some(Value::Tuple(array))),
                        PathSegment::ArrayIndex(array_index),
                    ) => {
                        result = Program::Value(array.inner.get(*array_index).unwrap().clone());
                    }
                    (
                        Program::Value(Some(Value::Object(object))),
                        PathSegment::ObjectKey(object_key),
                    ) => {
                        result = Program::Value(object.inner.get(object_key).unwrap().clone());
                    }
                    (
                        Program::Scope {
                            functions,
                            constants,
                            compute: _,
                        },
                        PathSegment::Functions | PathSegment::Constants,
                    ) => {
                        let programs_map = match current_path_segment {
                            AtSegment::ProgramPathSegment(PathSegment::Functions) => functions,
                            AtSegment::ProgramPathSegment(PathSegment::Constants) => constants,
                            _ => {
                                return Err(anyhow!(
                                    "Can not get program from {:#?} at {:#?}: stuck at path \
                                     segment {}: {current_path_segment:#?}",
                                    from,
                                    at,
                                    current_path_segment_index + 1
                                ));
                            }
                        };
                        current_path_segment_index += 1;
                        let current_path_segment_option = at.get(current_path_segment_index);
                        match current_path_segment_option {
                            Some(AtSegment::ProgramPathSegment(PathSegment::ObjectKey(
                                program_name,
                            ))) => match programs_map.get(program_name) {
                                Some(program_body) => {
                                    result = program_body.clone();
                                }
                                None => {
                                    return Err(anyhow!(
                                        "Can not get program from {:#?} at {:#?}: stuck at path \
                                         segment {}: {current_path_segment:#?}",
                                        from,
                                        at,
                                        current_path_segment_index + 1
                                    ));
                                }
                            },
                            _ => {
                                return Err(anyhow!(
                                    "Can not get program from {:#?} at {:#?}: stuck at path \
                                     segment {}: {current_path_segment:#?}",
                                    from,
                                    at,
                                    current_path_segment_index + 1
                                ));
                            }
                        }
                    }
                    (
                        Program::Scope {
                            functions: _,
                            constants: _,
                            compute,
                        },
                        PathSegment::Compute,
                    ) => {
                        result = *compute.clone();
                    }
                    _ => {
                        return Err(anyhow!(
                            "Can not get program from {:#?} at {:#?}: stuck at path segment {}: \
                             {current_path_segment:#?}",
                            from,
                            at,
                            current_path_segment_index + 1
                        ));
                    }
                }
            }
            _ => {
                break;
            }
        };
        current_path_segment_index += 1;
    }
    Ok((
        result,
        if current_path_segment_index < at.len() {
            Some(current_path_segment_index)
        } else {
            None
        },
    ))
}

pub fn compile(program: &Program) -> Result<IntermediateRepresentation> {
    let mut global_compilation_context = GlobalCompilationContext::default();
    let result_root = compile_with_context(
        program,
        CompilationContext::default(),
        &mut global_compilation_context,
    )?
    .node;
    Ok(IntermediateRepresentation {
        root: result_root,
        user_functions: global_compilation_context
            .user_functions
            .into_iter()
            .map(
                |(external_constants_name_clustered_indices, program_or_node, is_pure)| {
                    UserFunction {
                        external_constants_name_clustered_indices,
                        node: program_or_node.as_node().unwrap().clone(),
                        is_pure,
                    }
                },
            )
            .collect(),
        unique_constants_names_count: global_compilation_context
            .constants_names_to_name_clustered_constants_indices
            .len(),
    })
}

fn define_constant(
    name: String,
    r#type: Type,
    compilation_context: &mut CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
) -> ConstantDefinition {
    let result = ConstantDefinition {
        index: {
            let result = global_compilation_context.constants.len();
            global_compilation_context.constants.push(r#type);
            result
        },
        name_clustered_index: if let Some(constant_name_clustered_index) =
            global_compilation_context
                .constants_names_to_name_clustered_constants_indices
                .get(&name)
        {
            *constant_name_clustered_index
        } else {
            let result = global_compilation_context
                .constants_names_to_name_clustered_constants_indices
                .len();
            global_compilation_context
                .constants_names_to_name_clustered_constants_indices
                .insert(name.clone(), result);
            result
        },
    };
    compilation_context
        .available_constants
        .extend([(name, result.index)]);
    result
}

fn compile_with_context(
    program: &Program,
    compilation_context: CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
) -> Result<NodeAndMetadata> {
    Ok(match program {
        Program::Tuple(tuple) => {
            if tuple.is_empty() {
                return Err(anyhow!(
                    "Expected non-empty tuple at {:#?}",
                    compilation_context.path
                ));
            }
            let mut result_content = Vec::with_capacity(tuple.len());
            let mut result_external_constants_name_clustered_indices = BTreeSet::new();
            let mut result_elements_types = Vec::with_capacity(tuple.len());
            let mut result_is_pure = true;
            for (element_index, element) in tuple.iter().enumerate() {
                let mut element_compilation_context = compilation_context.clone();
                element_compilation_context
                    .path
                    .0
                    .extend([PathSegment::ArrayIndex(element_index)]);
                let mut compiled_element = compile_with_context(
                    element,
                    element_compilation_context.clone(),
                    global_compilation_context,
                )?;
                result_content.push(compiled_element.node);
                result_external_constants_name_clustered_indices
                    .append(&mut compiled_element.external_constants_name_clustered_indices);
                result_elements_types.push(compiled_element.r#type.clone());
                result_is_pure &= compiled_element.is_pure;
            }
            NodeAndMetadata {
                r#type: Type::Tuple(result_elements_types),
                external_constants_name_clustered_indices:
                    result_external_constants_name_clustered_indices,
                node: Node {
                    content: Content::Tuple(result_content),
                },
                is_pure: result_is_pure,
            }
        }
        Program::Scope {
            functions,
            constants,
            compute,
        } => {
            let mut compute_compilation_context = compilation_context.clone();
            compute_compilation_context
                .path
                .0
                .extend([PathSegment::Compute]);
            let mut new_constants = Vec::with_capacity(constants.len());
            let mut result_external_constants_name_clustered_indices = BTreeSet::new();
            let mut constants_name_clustered_indices = Vec::with_capacity(constants.len());
            let mut result_is_pure = true;
            for (constant_name, constant_compute_body) in constants.iter() {
                let mut constant_compilation_context = compilation_context.clone();
                constant_compilation_context.path.0.extend([
                    PathSegment::Constants,
                    PathSegment::Constant(constant_name.clone()),
                ]);
                let mut compiled_constant = compile_with_context(
                    constant_compute_body,
                    constant_compilation_context,
                    global_compilation_context,
                )?;
                constants_name_clustered_indices.push(
                    *global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .get(constant_name)
                        .unwrap(),
                );
                result_external_constants_name_clustered_indices
                    .append(&mut compiled_constant.external_constants_name_clustered_indices);
                let constant_definition = define_constant(
                    constant_name.clone(),
                    compiled_constant.r#type,
                    &mut compute_compilation_context,
                    global_compilation_context,
                );
                new_constants.push(intermediate_representation::ConstantDefinition {
                    name_clustered_index: constant_definition.name_clustered_index,
                    node: compiled_constant.node,
                });
                result_is_pure &= compiled_constant.is_pure;
            }
            for (function_name, function_body) in functions.iter() {
                if !function_name.ends_with(":") {
                    let mut function_compilation_context = compilation_context.clone();
                    function_compilation_context.path.0.extend([
                        PathSegment::Functions,
                        PathSegment::Function(function_name.clone()),
                    ]);
                    return Err(anyhow!(
                        "expected function named {:?}, found function named {function_name:?} at \
                         {:#?}",
                        format!("{function_name}:"),
                        function_compilation_context.path
                    ));
                }
                compute_compilation_context
                    .available_functions
                    .extend([(function_name.clone(), function_body.clone())]);
            }
            let mut compiled_compute = compile_with_context(
                compute,
                compute_compilation_context,
                global_compilation_context,
            )?;
            for constant_name_clustered_index in constants_name_clustered_indices {
                compiled_compute
                    .external_constants_name_clustered_indices
                    .remove(&constant_name_clustered_index);
            }
            result_external_constants_name_clustered_indices
                .append(&mut compiled_compute.external_constants_name_clustered_indices);
            result_is_pure &= compiled_compute.is_pure;
            NodeAndMetadata {
                r#type: compiled_compute.r#type,
                external_constants_name_clustered_indices:
                    result_external_constants_name_clustered_indices,
                node: Node {
                    content: Content::Scope {
                        constants: new_constants,
                        compute: Box::new(compiled_compute.node),
                    },
                },
                is_pure: result_is_pure,
            }
        }
        Program::Constant {
            constant: constant_name,
        } => {
            if let Some(constant_index) = compilation_context
                .available_constants
                .inner
                .get(constant_name)
            {
                let constant_type = global_compilation_context.constants[*constant_index].clone();
                let name_clustered_constant_index = *global_compilation_context
                    .constants_names_to_name_clustered_constants_indices
                    .get(constant_name)
                    .unwrap();
                NodeAndMetadata {
                    r#type: constant_type,
                    external_constants_name_clustered_indices: BTreeSet::from_iter([
                        name_clustered_constant_index,
                    ]),
                    node: Node {
                        content: Content::Constant(name_clustered_constant_index),
                    },
                    is_pure: true,
                }
            } else {
                return Err(anyhow!(
                    "expected one of available constants {:#?}, found {constant_name:?} at {:#?}",
                    compilation_context
                        .available_constants
                        .inner
                        .keys()
                        .collect::<Vec<_>>(),
                    compilation_context.path,
                ));
            }
        }
        Program::DefaultArgument(_) => compile_with_context(
            &Program::Constant {
                constant: DEFAULT_ARGUMENT_NAME.to_string(),
            },
            compilation_context,
            global_compilation_context,
        )?,
        Program::FromAt { from, at } => {
            let (extracted_from, first_non_program_path_segment_index_option) =
                process_from_at_program_path_part(
                    from,
                    at,
                    &mut global_compilation_context.includes_cache,
                )?;
            let mut from_program_compilation_context = compilation_context.clone();
            from_program_compilation_context
                .path
                .0
                .extend([PathSegment::From]);
            let compiled_extracted_from = compile_with_context(
                &extracted_from,
                from_program_compilation_context,
                global_compilation_context,
            )?;
            let external_constants_name_clustered_indices =
                compiled_extracted_from.external_constants_name_clustered_indices;
            let mut static_value_path_segments = Vec::new();
            let mut current_type = compiled_extracted_from.r#type;
            if let Some(first_non_program_path_segment_index) =
                first_non_program_path_segment_index_option
            {
                for (value_path_segment_shifted_index, value_path_segment) in at.iter().enumerate()
                {
                    let value_path_segment_index =
                        value_path_segment_shifted_index + first_non_program_path_segment_index;
                    let mut value_path_segment_compilation_context = compilation_context.clone();
                    value_path_segment_compilation_context.path.0.extend([
                        PathSegment::At,
                        PathSegment::ArrayIndex(value_path_segment_index),
                    ]);
                    match (&current_type, value_path_segment) {
                        (_, AtSegment::ProgramPathSegment(_)) => {
                            return Err(anyhow!(
                                "expected value path segment: array index or object key, found \
                                 {:#?} at {:#?}",
                                value_path_segment,
                                value_path_segment_compilation_context.path
                            ));
                        }
                        (Type::Array(element_type), AtSegment::ValueArrayIndex(array_index)) => {
                            static_value_path_segments.push(
                                intermediate_representation::ValuePathSegment::ArrayIndex(
                                    *array_index,
                                ),
                            );
                            current_type = *element_type.clone();
                        }
                        (Type::Tuple(elements_types), AtSegment::ValueArrayIndex(tuple_index)) => {
                            if *tuple_index >= elements_types.len() {
                                return Err(anyhow!(
                                    "expected tuple with at least {} elements, found tuple with \
                                     only {} elements at {:#?}",
                                    tuple_index + 1,
                                    elements_types.len(),
                                    value_path_segment_compilation_context.path
                                ));
                            }
                            static_value_path_segments.push(
                                intermediate_representation::ValuePathSegment::ArrayIndex(
                                    *tuple_index,
                                ),
                            );
                            current_type = elements_types[*tuple_index].clone();
                        }
                        (Type::Array(_), path_segment) => {
                            return Err(anyhow!(
                                "expected value array index, found {path_segment:#?} at {:#?}",
                                value_path_segment_compilation_context.path
                            ));
                        }
                        (
                            Type::Object(object_inner_types),
                            AtSegment::ValueObjectKey(object_key),
                        ) => {
                            static_value_path_segments.push(
                                intermediate_representation::ValuePathSegment::ObjectKey(
                                    object_key.clone(),
                                ),
                            );
                            if let Some(inner_type) = object_inner_types.get(object_key) {
                                current_type = inner_type.clone();
                            } else {
                                return Err(anyhow!(
                                    "expected object with key {object_key:?}, found \
                                     {object_inner_types:?} at {:#?}",
                                    value_path_segment_compilation_context.path
                                ));
                            }
                        }
                        (Type::Object(_), path_segment) => {
                            return Err(anyhow!(
                                "expected object key, found {path_segment:#?} at {:#?}",
                                value_path_segment_compilation_context.path
                            ));
                        }
                        (_, path_segment) => {
                            return Err(anyhow!(
                                "expected end of path when current type is {current_type:#?}, \
                                 found {path_segment:#?} at {:#?}",
                                value_path_segment_compilation_context.path
                            ));
                        }
                    };
                }
            }
            NodeAndMetadata {
                node: Node {
                    content: Content::FromAt {
                        from: Box::new(compiled_extracted_from.node),
                        value_path_segments: static_value_path_segments,
                    },
                },
                r#type: current_type,
                external_constants_name_clustered_indices,
                is_pure: compiled_extracted_from.is_pure,
            }
        }
        Program::EmbeddedFunction(embedded_function) => match &**embedded_function {
            EmbeddedFunction::Sum(argument) => {
                let mut argument_compilation_context = compilation_context.clone();
                argument_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Sum]);
                let compiled_argument = compile_with_context(
                    &argument,
                    argument_compilation_context.clone(),
                    global_compilation_context,
                )?;
                let expected_type = Type::Array(Box::new(Type::Number));
                assert_contains(
                    &compiled_argument.r#type,
                    &expected_type,
                    &compilation_context,
                    global_compilation_context,
                )?;
                NodeAndMetadata {
                    r#type: Type::Number,
                    external_constants_name_clustered_indices: compiled_argument
                        .external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::EmbeddedFunctionCall {
                            path: None,
                            embedded_function: Box::new(
                                intermediate_representation::EmbeddedFunction::Sum(
                                    compiled_argument.node,
                                ),
                            ),
                        },
                    },
                    is_pure: compiled_argument.is_pure,
                }
            }
            EmbeddedFunction::IsSorted(argument) => {
                let mut argument_compilation_context = compilation_context.clone();
                argument_compilation_context
                    .path
                    .0
                    .extend([PathSegment::IsSorted]);
                let compiled_argument = compile_with_context(
                    &argument,
                    argument_compilation_context.clone(),
                    global_compilation_context,
                )?;
                let expected_type = Type::Array(Box::new(Type::Any));
                assert_contains(
                    &compiled_argument.r#type,
                    &expected_type,
                    &compilation_context,
                    global_compilation_context,
                )?;
                NodeAndMetadata {
                    r#type: Type::Bool,
                    external_constants_name_clustered_indices: compiled_argument
                        .external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::EmbeddedFunctionCall {
                            path: None,
                            embedded_function: Box::new(
                                intermediate_representation::EmbeddedFunction::IsSorted(
                                    compiled_argument.node,
                                ),
                            ),
                        },
                    },
                    is_pure: compiled_argument.is_pure,
                }
            }
            EmbeddedFunction::StandardInput => NodeAndMetadata {
                r#type: Type::String,
                external_constants_name_clustered_indices: BTreeSet::new(),
                node: Node {
                    content: Content::EmbeddedFunctionCall {
                        path: Some(Path(
                            compilation_context
                                .path
                                .0
                                .extended([PathSegment::StandardInput]),
                        )),
                        embedded_function: Box::new(
                            intermediate_representation::EmbeddedFunction::StandardInput,
                        ),
                    },
                },
                is_pure: true,
            },
            EmbeddedFunction::ParseYaml(argument) => {
                let mut argument_compilation_context = compilation_context.clone();
                argument_compilation_context
                    .path
                    .0
                    .extend([PathSegment::ParseYaml]);
                let compiled_argument = compile_with_context(
                    &argument,
                    argument_compilation_context.clone(),
                    global_compilation_context,
                )?;
                let expected_type = Type::String;
                assert_contains(
                    &compiled_argument.r#type,
                    &expected_type,
                    &argument_compilation_context,
                    global_compilation_context,
                )?;
                NodeAndMetadata {
                    r#type: Type::Any,
                    external_constants_name_clustered_indices: compiled_argument
                        .external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::EmbeddedFunctionCall {
                            path: Some(Path(
                                compilation_context
                                    .path
                                    .0
                                    .extended([PathSegment::ParseYaml]),
                            )),
                            embedded_function: Box::new(
                                intermediate_representation::EmbeddedFunction::ParseYaml(
                                    compiled_argument.node,
                                ),
                            ),
                        },
                    },
                    is_pure: compiled_argument.is_pure,
                }
            }
            EmbeddedFunction::KeyValuePairs(argument) => {
                let mut argument_compilation_context = compilation_context.clone();
                argument_compilation_context
                    .path
                    .0
                    .extend([PathSegment::KeyValuePairs]);
                let compiled_argument = compile_with_context(
                    &argument,
                    argument_compilation_context.clone(),
                    global_compilation_context,
                )?;
                if let Type::Object(argument_object_values_types) = compiled_argument.r#type {
                    NodeAndMetadata {
                        r#type: Type::Tuple(
                            argument_object_values_types
                                .iter()
                                .map(|(key, value)| {
                                    Type::Tuple(vec![
                                        Type::Literal(Some(Value::String(ropey::Rope::from(
                                            key.clone(),
                                        )))),
                                        value.clone(),
                                    ])
                                })
                                .collect(),
                        ),
                        external_constants_name_clustered_indices: compiled_argument
                            .external_constants_name_clustered_indices,
                        node: Node {
                            content: Content::EmbeddedFunctionCall {
                                path: None,
                                embedded_function: Box::new(
                                    intermediate_representation::EmbeddedFunction::KeyValuePairs(
                                        compiled_argument.node,
                                    ),
                                ),
                            },
                        },
                        is_pure: compiled_argument.is_pure,
                    }
                } else {
                    return Err(anyhow!(
                        "expected object, found {:#?} at {:#?}",
                        compiled_argument.r#type,
                        compilation_context.path
                    ));
                }
            }
        },
        Program::Match {
            r#match,
            r#as,
            cases,
        } => {
            let mut match_compilation_context = compilation_context.clone();
            match_compilation_context
                .path
                .0
                .extend([PathSegment::Match]);
            let compiled_match = compile_with_context(
                r#match,
                match_compilation_context,
                global_compilation_context,
            )?;
            let mut result_cases = Vec::new();
            let mut result_types = BTreeSet::new();
            let mut result_external_constants_name_clustered_indices =
                compiled_match.external_constants_name_clustered_indices;
            let mut case_is_pure = true;
            let mut covered_types = BTreeSet::new();
            let match_constant_name_clustered_index_option = if let Some(match_constant_name) = r#as
            {
                Some(
                    if let Some(result) = global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .get(match_constant_name)
                    {
                        *result
                    } else {
                        global_compilation_context
                            .constants_names_to_name_clustered_constants_indices
                            .len()
                    },
                )
            } else {
                None
            };
            for (case_condition, case) in cases {
                let mut case_compilation_context = compilation_context.clone();
                case_compilation_context.path.0.extend([
                    PathSegment::Cases,
                    PathSegment::Case(case_condition.clone()),
                ]);
                match case_condition {
                    Condition::Type(refined_match_type) => {
                        if compiled_match
                            .r#type
                            .intersection(refined_match_type)
                            .is_none()
                        {
                            continue;
                        };
                        if let Some(match_constant_name) = r#as {
                            define_constant(
                                match_constant_name.clone(),
                                refined_match_type.clone(),
                                &mut case_compilation_context,
                                global_compilation_context,
                            );
                        };
                        let mut compiled_case = compile_with_context(
                            case,
                            case_compilation_context,
                            global_compilation_context,
                        )?;
                        result_types.insert(compiled_case.r#type);
                        result_external_constants_name_clustered_indices
                            .append(&mut compiled_case.external_constants_name_clustered_indices);
                        covered_types.insert(refined_match_type.clone());

                        case_is_pure &= compiled_case.is_pure;
                        result_cases.push(Case {
                            condition: intermediate_representation::Condition::Type(
                                refined_match_type.clone(),
                            ),
                            node: compiled_case.node,
                        });
                    }
                    Condition::Value(condition) => {
                        let compiled_condition = compile_with_context(
                            condition,
                            case_compilation_context.clone(),
                            global_compilation_context,
                        )?;
                        let refined_match_type = compiled_condition.r#type;
                        if compiled_match
                            .r#type
                            .intersection(&refined_match_type)
                            .is_none()
                        {
                            continue;
                        };
                        if let Some(match_constant_name) = r#as {
                            define_constant(
                                match_constant_name.clone(),
                                refined_match_type.clone(),
                                &mut case_compilation_context,
                                global_compilation_context,
                            );
                        }
                        let mut compiled_case = compile_with_context(
                            case,
                            case_compilation_context,
                            global_compilation_context,
                        )?;
                        result_types.insert(compiled_case.r#type);
                        result_external_constants_name_clustered_indices
                            .append(&mut compiled_case.external_constants_name_clustered_indices);
                        covered_types.insert(refined_match_type);

                        case_is_pure &= compiled_condition.is_pure && compiled_case.is_pure;
                        result_cases.push(Case {
                            condition: intermediate_representation::Condition::Value(
                                compiled_condition.node,
                            ),
                            node: compiled_case.node,
                        });
                    }
                }
            }
            let covered = Type::from(covered_types);
            if !covered.contains(&compiled_match.r#type) {
                return Err(anyhow!(
                    "expected coverage for {:#?}, found coverage only for {covered:#?} at {:#?}",
                    compiled_match.r#type,
                    compilation_context.path
                ));
            }
            match result_cases.len() {
                0 => compile_with_context(
                    &Program::Value(None),
                    compilation_context,
                    global_compilation_context,
                )?,
                _ => NodeAndMetadata {
                    node: Node {
                        content: Content::Match {
                            r#match: Box::new(compiled_match.node),
                            cases: result_cases,
                            match_constant_name_clustered_index_option,
                        },
                    },
                    r#type: Type::from(result_types),
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    is_pure: compiled_match.is_pure && case_is_pure,
                },
            }
        }
        Program::Map { map, r#as, through } => {
            let mut map_compilation_context = compilation_context.clone();
            map_compilation_context.path.0.extend([PathSegment::Map]);
            let compiled_map = compile_with_context(
                map,
                map_compilation_context.clone(),
                global_compilation_context,
            )?;
            let mut result_external_constants_name_clustered_indices =
                compiled_map.external_constants_name_clustered_indices;
            let mut is_pure = true;
            let map_constant_name_clustered_index = if let Some(result) = global_compilation_context
                .constants_names_to_name_clustered_constants_indices
                .get(r#as)
            {
                *result
            } else {
                global_compilation_context
                    .constants_names_to_name_clustered_constants_indices
                    .len()
            };
            match compiled_map.r#type {
                Type::Tuple(map_tuple_elements_types) => {
                    let mut result_elements_types =
                        Vec::with_capacity(map_tuple_elements_types.len());
                    let mut result_elements_nodes_indexes =
                        Vec::with_capacity(map_tuple_elements_types.len());
                    let mut compiled_throughs: indexmap::IndexSet<NodeAndMetadata> =
                        indexmap::IndexSet::new();
                    let mut element_type_to_compiled_through_index: BTreeMap<Type, usize> =
                        BTreeMap::new();
                    for element_type in map_tuple_elements_types {
                        if let Some(element_through_index) =
                            element_type_to_compiled_through_index.get(&element_type)
                        {
                            result_elements_types
                                .push(compiled_throughs[*element_through_index].r#type.clone());
                            result_elements_nodes_indexes.push(*element_through_index);
                        } else {
                            let mut through_compilation_context = compilation_context.clone();
                            through_compilation_context
                                .path
                                .0
                                .extend([PathSegment::Through(element_type.clone())]);
                            define_constant(
                                r#as.clone(),
                                element_type.clone(),
                                &mut through_compilation_context,
                                global_compilation_context,
                            );
                            let compiled_through = compile_with_context(
                                through,
                                through_compilation_context,
                                global_compilation_context,
                            )?;
                            result_external_constants_name_clustered_indices.extend(
                                compiled_through
                                    .external_constants_name_clustered_indices
                                    .clone(),
                            );
                            is_pure &= compiled_through.is_pure;
                            result_elements_types.push(compiled_through.r#type.clone());
                            let compiled_through_index = if let Some(compiled_through_index) =
                                compiled_throughs.get_index_of(&compiled_through)
                            {
                                compiled_through_index
                            } else {
                                compiled_throughs.insert(compiled_through);
                                compiled_throughs.len() - 1
                            };
                            element_type_to_compiled_through_index
                                .insert(element_type, compiled_through_index);
                            result_elements_nodes_indexes.push(compiled_through_index);
                        }
                    }
                    NodeAndMetadata {
                        node: Node {
                            content: Content::Map {
                                map: Box::new(compiled_map.node),
                                throughs: MapThroughs::Tuple {
                                    elements_nodes_indexes: result_elements_nodes_indexes,
                                    nodes: compiled_throughs
                                        .into_iter()
                                        .map(|compiled_through| compiled_through.node)
                                        .collect(),
                                },
                                map_constant_name_clustered_index,
                            },
                        },
                        r#type: Type::Tuple(result_elements_types),
                        external_constants_name_clustered_indices:
                            result_external_constants_name_clustered_indices,
                        is_pure,
                    }
                }
                Type::Array(map_array_element_type) => {
                    let mut through_compilation_context = compilation_context.clone();
                    through_compilation_context
                        .path
                        .0
                        .extend([PathSegment::Through(*map_array_element_type.clone())]);
                    define_constant(
                        r#as.clone(),
                        *map_array_element_type.clone(),
                        &mut through_compilation_context,
                        global_compilation_context,
                    );
                    let compiled_through = compile_with_context(
                        through,
                        through_compilation_context,
                        global_compilation_context,
                    )?;
                    result_external_constants_name_clustered_indices
                        .extend(compiled_through.external_constants_name_clustered_indices);
                    is_pure &= compiled_through.is_pure;
                    NodeAndMetadata {
                        node: Node {
                            content: Content::Map {
                                map: Box::new(compiled_map.node),
                                throughs: MapThroughs::Array(Box::new(compiled_through.node)),
                                map_constant_name_clustered_index,
                            },
                        },
                        r#type: Type::Array(Box::new(compiled_through.r#type)),
                        external_constants_name_clustered_indices:
                            result_external_constants_name_clustered_indices,
                        is_pure,
                    }
                }
                _ => {
                    return Err(anyhow!(
                        "expected tuple or array, found {:#?} at {:#?}",
                        compiled_map.r#type,
                        map_compilation_context.path
                    ));
                }
            }
        }
        Program::Object(object) => {
            match object.len() {
                0 => {
                    return Ok(NodeAndMetadata {
                        r#type: Type::Object(BTreeMap::new()),
                        external_constants_name_clustered_indices: BTreeSet::new(),
                        node: Node {
                            content: Content::Object(BTreeMap::new()),
                        },
                        is_pure: true,
                    });
                }
                1 => {
                    let (function_name, function_argument) = object.iter().next().unwrap();
                    if function_name.ends_with(":") {
                        if let Some(function_body) = compilation_context
                            .available_functions
                            .inner
                            .get(function_name)
                        {
                            let mut arguments_is_pure = true;
                            let mut body_compilation_context = compilation_context.clone();
                            body_compilation_context
                                .path
                                .0
                                .extend([PathSegment::UserFunctionCall(function_name.clone())]);
                            let arguments_iterator = match function_argument {
                                Program::Object(function_arguments) => {
                                    let arguments_iterator: Box<
                                        dyn Iterator<Item = (&str, &Program)>,
                                    > = if function_arguments.len() > 1 {
                                        Box::new(
                                            function_arguments
                                                .iter()
                                                .map(|(key, value)| (key.as_str(), value)),
                                        )
                                    } else {
                                        Box::new(
                                            [(DEFAULT_ARGUMENT_NAME, function_argument)]
                                                .into_iter(),
                                        )
                                    };
                                    arguments_iterator
                                }
                                _ => Box::new(
                                    [(DEFAULT_ARGUMENT_NAME, function_argument)].into_iter(),
                                ),
                            };
                            let mut new_constants_definitions = Vec::new();
                            let mut result_external_constants_name_clustered_indices =
                                BTreeSet::new();
                            for (function_argument_name, function_argument_body) in
                                arguments_iterator
                            {
                                let mut argument_compilation_context = compilation_context.clone();
                                argument_compilation_context.path.0.extend([
                                    PathSegment::UserFunctionCall(function_name.clone()),
                                    PathSegment::Argument(function_argument_name.to_string()),
                                ]);
                                let mut compiled_constant = compile_with_context(
                                    &function_argument_body,
                                    argument_compilation_context,
                                    global_compilation_context,
                                )?;
                                result_external_constants_name_clustered_indices.append(
                                    &mut compiled_constant
                                        .external_constants_name_clustered_indices,
                                );
                                let constant_definition = define_constant(
                                    function_argument_name.to_string(),
                                    compiled_constant.r#type,
                                    &mut body_compilation_context,
                                    global_compilation_context,
                                );
                                new_constants_definitions.push(
                                    intermediate_representation::ConstantDefinition {
                                        name_clustered_index: constant_definition
                                            .name_clustered_index,
                                        node: compiled_constant.node,
                                    },
                                );
                                arguments_is_pure &= compiled_constant.is_pure;
                            }
                            if compilation_context
                                .entered_user_functions
                                .inner
                                .contains(function_body)
                            {
                                let (function_index, function_type) = global_compilation_context
                                    .user_function_to_index_and_type_option
                                    .get(function_body)
                                    .unwrap();
                                return Ok(NodeAndMetadata {
                                    r#type: function_type.clone(),
                                    external_constants_name_clustered_indices: BTreeSet::new(),
                                    node: Node {
                                        content: Content::UserFunctionCall {
                                            arguments: new_constants_definitions,
                                            body: *function_index,
                                        },
                                    },
                                    is_pure: arguments_is_pure,
                                });
                            } else {
                                body_compilation_context
                                    .entered_user_functions
                                    .extend([function_body.clone()]);
                                let function_index =
                                    global_compilation_context.user_functions.len();
                                global_compilation_context.user_functions.push((
                                    Vec::new(),
                                    ProgramOrNode::Program(function_body.clone()),
                                    arguments_is_pure,
                                ));
                                global_compilation_context
                                    .user_function_to_index_and_type_option
                                    .insert(
                                        function_body.clone(),
                                        (function_index, Type::Unknown(function_index)),
                                    );
                                let mut compiled_function = compile_with_context(
                                    function_body,
                                    body_compilation_context,
                                    global_compilation_context,
                                )?;
                                global_compilation_context
                                    .user_function_to_index_and_type_option
                                    .get_mut(function_body)
                                    .unwrap()
                                    .1 = compiled_function.r#type.clone();
                                global_compilation_context
                                    .user_function_node_to_index_and_type_option
                                    .insert(
                                        compiled_function.node.clone(),
                                        (function_index, compiled_function.r#type.clone()),
                                    );
                                global_compilation_context.user_functions[function_index] = (
                                    Vec::from_iter(
                                        compiled_function
                                            .external_constants_name_clustered_indices
                                            .iter()
                                            .cloned(),
                                    ),
                                    ProgramOrNode::Node(compiled_function.node.clone()),
                                    compiled_function.is_pure,
                                );
                                result_external_constants_name_clustered_indices.append(
                                    &mut compiled_function
                                        .external_constants_name_clustered_indices,
                                );
                                return Ok(NodeAndMetadata {
                                    r#type: compiled_function.r#type,
                                    external_constants_name_clustered_indices:
                                        result_external_constants_name_clustered_indices.clone(),
                                    node: Node {
                                        content: Content::UserFunctionCall {
                                            arguments: new_constants_definitions,
                                            body: function_index,
                                        },
                                    },
                                    is_pure: arguments_is_pure && compiled_function.is_pure,
                                });
                            }
                        } else {
                            return Err(anyhow!(
                                "expected one of available functions {:#?}, found function \
                                 {function_name:?} at {:#?}",
                                compilation_context
                                    .available_functions
                                    .inner
                                    .keys()
                                    .collect::<Vec<_>>(),
                                compilation_context.path,
                            ));
                        }
                    }
                }
                2.. => {}
            };
            let mut result_inner_types = BTreeMap::new();
            let mut result_content = BTreeMap::new();
            let mut result_external_constants_name_clustered_indices = BTreeSet::new();
            let mut is_pure = true;
            for (object_key, object_value) in object.iter() {
                let mut object_value_compilation_context = compilation_context.clone();
                object_value_compilation_context
                    .path
                    .0
                    .extend([PathSegment::ObjectKey(object_key.clone())]);
                let mut compiled_object_value = compile_with_context(
                    object_value,
                    object_value_compilation_context,
                    global_compilation_context,
                )?;
                result_external_constants_name_clustered_indices
                    .append(&mut compiled_object_value.external_constants_name_clustered_indices);
                result_content.insert(object_key.clone(), compiled_object_value.node);
                result_inner_types.insert(object_key.clone(), compiled_object_value.r#type);
                is_pure &= compiled_object_value.is_pure;
            }
            NodeAndMetadata {
                r#type: Type::Object(result_inner_types),
                external_constants_name_clustered_indices:
                    result_external_constants_name_clustered_indices,
                node: Node {
                    content: Content::Object(result_content),
                },
                is_pure,
            }
        }
        Program::Value(value) => NodeAndMetadata {
            r#type: Type::Literal(value.clone()),
            external_constants_name_clustered_indices: BTreeSet::new(),
            node: Node {
                content: Content::Value(unsafe { std::mem::transmute(value.clone()) }),
            },
            is_pure: true,
        },
    })
}
