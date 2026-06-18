use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Error, Result, anyhow};

use crate::{
    containers::{Map, Set},
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    includes_cache::IncludesCache,
    intermediate_representation::{
        self, ConstantDefinition, Content, IntermediateRepresentation, Node, UserFunction,
    },
    program::{EmbeddedFunction, IncludeFromAt, Path, PathSegment, Program},
    value::Value,
};

#[derive(PartialEq, Debug, Clone, Eq)]
pub enum Type {
    Number,
    String,
    Bool,
    Null,
    Array(Box<Type>),
    Object(BTreeMap<String, Type>),
    Unknown(usize),
}

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
            let mut previous_element_type_option = None;
            for (element_index, element) in array.inner.iter().enumerate() {
                let mut current_element_compilation_context = compilation_context.clone();
                current_element_compilation_context
                    .path
                    .0
                    .extend([PathSegment::ArrayIndex(element_index)]);
                let current_element_type =
                    get_value_type(element, current_element_compilation_context.clone())?;
                if let Some(ref element_type) = previous_element_type_option {
                    if &current_element_type != element_type {
                        return Err(current_element_compilation_context
                            .error(&current_element_type, element_type));
                    }
                } else {
                    previous_element_type_option = Some(current_element_type);
                }
            }
            if let Some(element_type) = previous_element_type_option {
                element_type
            } else {
                return Err(anyhow!(
                    "Expected non-empty array at {:#?}",
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

fn assert_equal(
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
        (Type::Array(got_element_type), Type::Array(expected_element_type)) => assert_equal(
            got_element_type,
            expected_element_type,
            compilation_context,
            global_compilation_context,
        ),
        (Type::Object(got_element_inner_types), Type::Object(expected_element_inner_types)) => {
            for (expected_value_key, expected_value_type) in expected_element_inner_types {
                if let Some(got_value_type) = got_element_inner_types.get(expected_value_key) {
                    assert_equal(
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
                            return Err(
                                compilation_context.error(&previously_resolved_type, expected_type)
                            );
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
                            return Err(
                                compilation_context.error(&previously_resolved_type, expected_type)
                            );
                        }
                    }
                }
            }
            Ok(())
        }
        _ => Err(compilation_context.error(got_type, expected_type)),
    }
}

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

#[derive(Clone, Debug)]
struct NodeAndMetadata {
    node: Node,
    r#type: Type,
    external_constants_name_clustered_indices: BTreeSet<usize>,
}

#[derive(Default)]
struct GlobalCompilationContext {
    user_function_to_index_and_type_option: BTreeMap<Program, (usize, Type)>,
    user_function_node_to_index_and_type_option: BTreeMap<Node, (usize, Type)>,
    user_functions: Vec<(Vec<usize>, ProgramOrNode)>,
    constants_names_to_name_clustered_constants_indices: BTreeMap<String, usize>,
    constants: Vec<(Type, Node)>,
    includes_cache: IncludesCache,
}

fn process_include(
    include_clause: &IncludeFromAt,
    includes_cache: &mut IncludesCache,
) -> Result<Program> {
    let mut result = includes_cache.get(&include_clause.from)?;
    let mut current_path_segment_index = 0;
    while current_path_segment_index < include_clause.at.0.inner.len() {
        let mut current_path_segment = include_clause.at.0.inner.get(current_path_segment_index);
        match (result, current_path_segment) {
            (Program::Array(array), Some(PathSegment::ArrayIndex(array_index))) => {
                result = array.get(*array_index).unwrap().clone();
            }
            (Program::Object(object), Some(PathSegment::ObjectKey(object_key))) => {
                result = object.get(object_key).unwrap().clone();
            }
            (Program::Value(Value::Array(array)), Some(PathSegment::ArrayIndex(array_index))) => {
                result = Program::Value(array.inner.get(*array_index).unwrap().clone());
            }
            (Program::Value(Value::Object(object)), Some(PathSegment::ObjectKey(object_key))) => {
                result = Program::Value(object.inner.get(object_key).unwrap().clone());
            }
            (
                Program::Scope {
                    functions,
                    constants,
                    compute: _,
                },
                Some(PathSegment::Functions | PathSegment::Constants),
            ) => {
                let programs_map = match current_path_segment {
                    Some(PathSegment::Functions) => functions,
                    Some(PathSegment::Constants) => constants,
                    _ => {
                        return Err(anyhow!(
                            "Can not get program from {:#?} at {:#?}: stuck at path segment {}: \
                             {current_path_segment:#?}",
                            include_clause.from,
                            include_clause.at,
                            current_path_segment_index + 1
                        ));
                    }
                };
                current_path_segment_index += 1;
                current_path_segment = include_clause.at.0.inner.get(current_path_segment_index);
                match current_path_segment {
                    Some(PathSegment::ObjectKey(program_name)) => {
                        match programs_map.get(program_name) {
                            Some(program_body) => {
                                result = program_body.clone();
                            }
                            None => {
                                return Err(anyhow!(
                                    "Can not get program from {:#?} at {:#?}: stuck at path \
                                     segment {}: {current_path_segment:#?}",
                                    include_clause.from,
                                    include_clause.at,
                                    current_path_segment_index + 1
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(anyhow!(
                            "Can not get program from {:#?} at {:#?}: stuck at path segment {}: \
                             {current_path_segment:#?}",
                            include_clause.from,
                            include_clause.at,
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
                Some(PathSegment::Compute),
            ) => {
                result = *compute.clone();
            }
            (Program::Branching(branching_clause), Some(PathSegment::If)) => {
                result = branching_clause.r#if.clone();
            }
            (Program::Branching(branching_clause), Some(PathSegment::Then)) => {
                result = branching_clause.then.clone();
            }
            (Program::Branching(branching_clause), Some(PathSegment::Else)) => {
                result = branching_clause.r#else.clone();
            }
            _ => {
                return Err(anyhow!(
                    "Can not get program from {:#?} at {:#?}: stuck at path segment {}: \
                     {current_path_segment:#?}",
                    include_clause.from,
                    include_clause.at,
                    current_path_segment_index + 1
                ));
            }
        };
        current_path_segment_index += 1;
    }
    Ok(result)
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
                |(external_constants_name_clustered_indices, program_or_node)| UserFunction {
                    external_constants_name_clustered_indices,
                    node: program_or_node.as_node().unwrap().clone(),
                },
            )
            .collect(),
        constants: global_compilation_context
            .constants
            .into_iter()
            .map(|constant| constant.1)
            .collect(),
        unique_constants_names_count: global_compilation_context
            .constants_names_to_name_clustered_constants_indices
            .len(),
    })
}

fn compile_with_context(
    program: &Program,
    compilation_context: CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
) -> Result<NodeAndMetadata> {
    Ok(match program {
        Program::Array(array) => {
            if array.is_empty() {
                return Err(anyhow!(
                    "Expected non-empty array at {:#?}",
                    compilation_context.path
                ));
            }
            let mut result_content = Vec::with_capacity(array.len());
            let mut result_external_constants_name_clustered_indices = BTreeSet::new();
            let mut previous_element_type_option = None;
            for (element_index, element) in array.iter().enumerate() {
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
                result_external_constants_name_clustered_indices
                    .append(&mut compiled_element.external_constants_name_clustered_indices);
                if let Some(ref previous_element_type) = previous_element_type_option {
                    assert_equal(
                        &compiled_element.r#type,
                        &previous_element_type,
                        &element_compilation_context,
                        global_compilation_context,
                    )?;
                } else {
                    previous_element_type_option = Some(compiled_element.r#type);
                }
                result_content.push(compiled_element.node);
            }
            NodeAndMetadata {
                r#type: Type::Array(Box::new(previous_element_type_option.unwrap())),
                external_constants_name_clustered_indices:
                    result_external_constants_name_clustered_indices,
                node: Node {
                    content: Content::Array(result_content),
                },
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
            let mut new_constants_definitions = Vec::with_capacity(constants.len());
            let mut result_external_constants_name_clustered_indices = BTreeSet::new();
            let mut constants_name_clustered_indices = Vec::with_capacity(constants.len());
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
                let constant_definition = ConstantDefinition {
                    index: global_compilation_context.constants.len(),
                    name_clustered_index: if let Some(constant_name_clustered_index) =
                        global_compilation_context
                            .constants_names_to_name_clustered_constants_indices
                            .get(constant_name)
                    {
                        *constant_name_clustered_index
                    } else {
                        let result = global_compilation_context
                            .constants_names_to_name_clustered_constants_indices
                            .len();
                        global_compilation_context
                            .constants_names_to_name_clustered_constants_indices
                            .insert(constant_name.clone(), result);
                        result
                    },
                };
                compute_compilation_context
                    .available_constants
                    .extend([(constant_name.clone(), constant_definition.index)]);
                new_constants_definitions.push(constant_definition);
                global_compilation_context
                    .constants
                    .push((compiled_constant.r#type, compiled_constant.node));
            }
            for (function_name, function_body) in functions.iter() {
                if !function_name.ends_with(":") {
                    let mut function_compilation_context = compilation_context.clone();
                    function_compilation_context.path.0.extend([
                        PathSegment::Functions,
                        PathSegment::Function(function_name.clone()),
                    ]);
                    return Err(anyhow!(
                        "Got function named {function_name:?}, but expect function named {:?} at \
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
            NodeAndMetadata {
                r#type: compiled_compute.r#type,
                external_constants_name_clustered_indices:
                    result_external_constants_name_clustered_indices,
                node: Node {
                    content: Content::Scope {
                        constants: new_constants_definitions,
                        compute: Box::new(compiled_compute.node),
                    },
                },
            }
        }
        Program::Branching(branching_clause) => {
            let mut result_external_constants_name_clustered_indices = BTreeSet::new();
            let mut if_compilation_context = compilation_context.clone();
            if_compilation_context.path.0.extend([PathSegment::If]);
            let mut compiled_if = compile_with_context(
                &branching_clause.r#if,
                if_compilation_context.clone(),
                global_compilation_context,
            )?;
            result_external_constants_name_clustered_indices
                .append(&mut compiled_if.external_constants_name_clustered_indices);
            assert_equal(
                &compiled_if.r#type,
                &Type::Bool,
                &compilation_context,
                global_compilation_context,
            )?;
            let mut then_compilation_context = compilation_context.clone();
            then_compilation_context.path.0.extend([PathSegment::Then]);
            let mut compiled_then = compile_with_context(
                &branching_clause.then,
                then_compilation_context,
                global_compilation_context,
            )?;
            result_external_constants_name_clustered_indices
                .append(&mut compiled_then.external_constants_name_clustered_indices);
            let mut else_compilation_context = compilation_context.clone();
            else_compilation_context.path.0.extend([PathSegment::Else]);
            let mut compiled_else = compile_with_context(
                &branching_clause.r#else,
                else_compilation_context.clone(),
                global_compilation_context,
            )?;
            result_external_constants_name_clustered_indices
                .append(&mut compiled_else.external_constants_name_clustered_indices);
            assert_equal(
                &compiled_else.r#type,
                &compiled_then.r#type,
                &compilation_context,
                global_compilation_context,
            )?;
            NodeAndMetadata {
                r#type: compiled_then.r#type,
                external_constants_name_clustered_indices:
                    result_external_constants_name_clustered_indices,
                node: Node {
                    content: Content::Branching(Box::new(intermediate_representation::Branching {
                        r#if: compiled_if.node,
                        then: compiled_then.node,
                        r#else: compiled_else.node,
                    })),
                },
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
                let (constant_type, _) =
                    global_compilation_context.constants[*constant_index].clone();
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
                }
            } else {
                return Err(anyhow!(
                    "Got no constant {constant_name:?} at {:#?}, available constants are {:#?}",
                    compilation_context.path,
                    compilation_context
                        .available_constants
                        .inner
                        .keys()
                        .collect::<Vec<_>>()
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
        Program::Include {
            include: include_clause,
        } => compile_with_context(
            &process_include(
                include_clause,
                &mut global_compilation_context.includes_cache,
            )?,
            compilation_context,
            global_compilation_context,
        )?,
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
                assert_equal(
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
                            path: Some(argument_compilation_context.path.clone()),
                            embedded_function: Box::new(
                                intermediate_representation::EmbeddedFunction::Sum(
                                    compiled_argument.node,
                                ),
                            ),
                        },
                    },
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
                if let Type::Array(_) = compiled_argument.r#type {
                } else {
                    return Err(anyhow!(
                        "Got {:?} but expected Array",
                        compiled_argument.r#type,
                    ));
                }
                NodeAndMetadata {
                    r#type: Type::Bool,
                    external_constants_name_clustered_indices: compiled_argument
                        .external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::EmbeddedFunctionCall {
                            path: Some(argument_compilation_context.path.clone()),
                            embedded_function: Box::new(
                                intermediate_representation::EmbeddedFunction::IsSorted(
                                    compiled_argument.node,
                                ),
                            ),
                        },
                    },
                }
            }
        },
        Program::Object(object) => {
            match object.len() {
                0 => {
                    return Ok(NodeAndMetadata {
                        r#type: Type::Object(BTreeMap::new()),
                        external_constants_name_clustered_indices: BTreeSet::new(),
                        node: Node {
                            content: Content::Object(BTreeMap::new()),
                        },
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
                            let mut body_compilation_context = compilation_context.clone();
                            body_compilation_context
                                .path
                                .0
                                .extend([PathSegment::UserFunctionCall(function_name.clone())]);
                            let arguments_iterator = match function_argument {
                                Program::Object(function_arguments) => {
                                    if function_arguments.is_empty() {
                                        return Err(anyhow!(
                                            "Got zero arguments, but expected at least one at \
                                             {:#?}",
                                            compilation_context.path
                                        ));
                                    }
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
                                let constant_definition = ConstantDefinition {
                                    index: global_compilation_context.constants.len(),
                                    name_clustered_index: if let Some(
                                        constant_name_clustered_index,
                                    ) = global_compilation_context
                                        .constants_names_to_name_clustered_constants_indices
                                        .get(function_argument_name)
                                    {
                                        *constant_name_clustered_index
                                    } else {
                                        let result = global_compilation_context
                                            .constants_names_to_name_clustered_constants_indices
                                            .len();
                                        global_compilation_context
                                            .constants_names_to_name_clustered_constants_indices
                                            .insert(function_argument_name.to_string(), result);
                                        result
                                    },
                                };
                                body_compilation_context.available_constants.extend([(
                                    function_argument_name.to_string(),
                                    constant_definition.index,
                                )]);
                                new_constants_definitions.push(constant_definition);
                                global_compilation_context
                                    .constants
                                    .push((compiled_constant.r#type, compiled_constant.node));
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
                                });
                            }
                        } else {
                            return Err(anyhow!(
                                "Got function {function_name:?} at {:#?}, but expected one of \
                                 available functions: {:#?}",
                                compilation_context.path,
                                compilation_context
                                    .available_functions
                                    .inner
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
            let mut result_external_constants_name_clustered_indices = BTreeSet::new();
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
            }
            NodeAndMetadata {
                r#type: Type::Object(result_inner_types),
                external_constants_name_clustered_indices:
                    result_external_constants_name_clustered_indices,
                node: Node {
                    content: Content::Object(result_content),
                },
            }
        }
        Program::Value(value) => NodeAndMetadata {
            r#type: get_value_type(value, compilation_context.clone())?,
            external_constants_name_clustered_indices: BTreeSet::new(),
            node: Node {
                content: Content::Value(unsafe { std::mem::transmute(value.clone()) }),
            },
        },
    })
}
