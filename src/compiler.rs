use std::collections::BTreeMap;

use anyhow::{Error, Result, anyhow};

use crate::{
    containers::{Map, Set},
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    intermediate_representation::{self, Content, IntermediateRepresentation, Node},
    program::{Clause, EmbeddedFunction, Path, PathSegment, Program, Scope},
    r#type::Type,
    value::Value,
};

#[derive(Clone, Default)]
pub struct CompilationContext {
    pub path: Path,
    pub available_functions: Map<String, Program>,
    pub available_constants: Map<String, usize>,
    pub entered_user_functions: Set<Program>,
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
        (Type::Unknown(got_program_index), expected_type)
        | (expected_type, Type::Unknown(got_program_index)) => {
            let got_program = global_compilation_context.user_functions[*got_program_index]
                .as_program()
                .unwrap();
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
                    return Err(compilation_context.error(&previously_resolved_type, expected_type));
                }
            }
            Ok(())
        }
        _ => Err(compilation_context.error(got_type, expected_type)),
    }
}

#[derive(Debug)]
pub enum ProgramOrNode {
    Program(Program),
    Node(Node),
}

impl ProgramOrNode {
    pub fn as_program(&self) -> Option<&Program> {
        match self {
            &ProgramOrNode::Program(ref program) => Some(program),
            &ProgramOrNode::Node(_) => None,
        }
    }
    pub fn as_node(&self) -> Option<&Node> {
        match self {
            &ProgramOrNode::Node(ref node) => Some(node),
            &ProgramOrNode::Program(_) => None,
        }
    }
}

#[derive(Default)]
pub struct GlobalCompilationContext {
    pub user_function_to_index_and_type_option: BTreeMap<Program, (usize, Type)>,
    pub user_functions: Vec<ProgramOrNode>,
    pub constants_names_to_name_clustered_constants_indices: BTreeMap<String, usize>,
    pub constants: Vec<(Type, Node)>,
}

pub fn compile(program: &Program) -> Result<IntermediateRepresentation> {
    let mut global_compilation_context = GlobalCompilationContext::default();
    let result_root = compile_with_context(
        program,
        CompilationContext::default(),
        &mut global_compilation_context,
    )?
    .1;
    Ok(IntermediateRepresentation {
        root: result_root,
        user_functions: global_compilation_context
            .user_functions
            .into_iter()
            .map(|program_or_node| program_or_node.as_node().unwrap().clone())
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
                if let Some(ref previous_element_type) = previous_element_type_option {
                    assert_equal(
                        &element_type,
                        &previous_element_type,
                        &compilation_context,
                        global_compilation_context,
                    )?;
                } else {
                    previous_element_type_option = Some(element_type);
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
                let mut compute_compilation_context = compilation_context.clone();
                compute_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Scope, PathSegment::Compute]);
                let mut new_constants_indices = Vec::with_capacity(constants.len());
                for (constant_name, constant_compute_body) in constants.iter() {
                    let mut constant_compilation_context = compilation_context.clone();
                    constant_compilation_context.path.0.extend([
                        PathSegment::Scope,
                        PathSegment::Constants,
                        PathSegment::Constant(constant_name.clone()),
                    ]);
                    let (constant_type, constant_node) = compile_with_context(
                        constant_compute_body,
                        constant_compilation_context,
                        global_compilation_context,
                    )?;
                    let constant_index = global_compilation_context.constants.len();
                    let constant_name_clustered_index = if let Some(constant_name_clustered_index) =
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
                    };
                    new_constants_indices.push((constant_name_clustered_index, constant_index));
                    compute_compilation_context
                        .available_constants
                        .extend([(constant_name.clone(), constant_index)]);
                    global_compilation_context
                        .constants
                        .push((constant_type, constant_node));
                }
                for (function_name, function_body) in functions.iter() {
                    if !function_name.ends_with(":") {
                        let mut function_compilation_context = compilation_context.clone();
                        function_compilation_context.path.0.extend([
                            PathSegment::Functions,
                            PathSegment::Function(function_name.clone()),
                        ]);
                        return Err(anyhow!(
                            "Got function named {function_name:?}, but expect function named {:?} \
                             at {:#?}",
                            format!("{function_name}:"),
                            function_compilation_context.path
                        ));
                    }
                    compute_compilation_context
                        .available_functions
                        .extend([(function_name.clone(), function_body.clone())]);
                }
                let (compute_type, compute_node) = compile_with_context(
                    compute,
                    compute_compilation_context,
                    global_compilation_context,
                )?;
                (
                    compute_type,
                    Node {
                        path: Path(compilation_context.path.0.extended([PathSegment::Scope])),
                        content: Content::Scope {
                            constants: new_constants_indices,
                            compute: Box::new(compute_node),
                        },
                    },
                )
            }
            Clause::Branching { r#if, then, r#else } => {
                let mut if_compilation_context = compilation_context.clone();
                if_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Branching, PathSegment::If]);
                let (if_type, if_node) = compile_with_context(
                    r#if,
                    if_compilation_context.clone(),
                    global_compilation_context,
                )?;
                assert_equal(
                    &if_type,
                    &Type::Bool,
                    &compilation_context,
                    global_compilation_context,
                )?;
                let mut then_compilation_context = compilation_context.clone();
                then_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Branching, PathSegment::Then]);
                let (then_type, then_node) = compile_with_context(
                    then,
                    then_compilation_context,
                    global_compilation_context,
                )?;
                let mut else_compilation_context = compilation_context.clone();
                else_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Branching, PathSegment::Else]);
                let (else_type, else_node) = compile_with_context(
                    r#else,
                    else_compilation_context.clone(),
                    global_compilation_context,
                )?;
                assert_equal(
                    &else_type,
                    &then_type,
                    &compilation_context,
                    global_compilation_context,
                )?;
                (
                    then_type,
                    Node {
                        path: compilation_context.path.clone(),
                        content: Content::Branching(Box::new(
                            intermediate_representation::Branching {
                                r#if: if_node,
                                then: then_node,
                                r#else: else_node,
                            },
                        )),
                    },
                )
            }
            Clause::Constant(constant_name) => {
                if let Some(constant_index) = compilation_context
                    .available_constants
                    .inner
                    .get(constant_name)
                {
                    let (constant_type, _) =
                        global_compilation_context.constants[*constant_index].clone();
                    (
                        constant_type,
                        Node {
                            path: compilation_context.path.clone(),
                            content: Content::Constant(*constant_index),
                        },
                    )
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
            Clause::DefaultArgument => compile_with_context(
                &Program::Clause(Clause::Constant(DEFAULT_ARGUMENT_NAME.to_string())),
                compilation_context,
                global_compilation_context,
            )?,
        },
        Program::EmbeddedFunction(embedded_function) => match &**embedded_function {
            EmbeddedFunction::Sum(argument) => {
                let mut argument_compilation_context = compilation_context.clone();
                argument_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Sum]);
                let (argument_type, argument_node) = compile_with_context(
                    &argument,
                    argument_compilation_context.clone(),
                    global_compilation_context,
                )?;
                let expected_type = Type::Array(Box::new(Type::Number));
                assert_equal(
                    &argument_type,
                    &expected_type,
                    &compilation_context,
                    global_compilation_context,
                )?;
                (
                    Type::Number,
                    Node {
                        path: compilation_context.path.clone(),
                        content: Content::EmbeddedFunctionCall(Box::new(
                            intermediate_representation::EmbeddedFunction::Sum(argument_node),
                        )),
                    },
                )
            }
            EmbeddedFunction::IsSorted(argument) => {
                let mut argument_compilation_context = compilation_context.clone();
                argument_compilation_context
                    .path
                    .0
                    .extend([PathSegment::IsSorted]);
                let (argument_type, argument_node) = compile_with_context(
                    &argument,
                    argument_compilation_context.clone(),
                    global_compilation_context,
                )?;
                if let Type::Array(_) = argument_type {
                } else {
                    return Err(anyhow!("Got {:?} but expected Array", argument_type,));
                }
                (
                    Type::Bool,
                    Node {
                        path: compilation_context.path.clone(),
                        content: Content::EmbeddedFunctionCall(Box::new(
                            intermediate_representation::EmbeddedFunction::IsSorted(argument_node),
                        )),
                    },
                )
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
                            let mut new_constants_indices = Vec::new();
                            for (function_argument_name, function_argument_body) in
                                arguments_iterator
                            {
                                let mut argument_compilation_context = compilation_context.clone();
                                argument_compilation_context.path.0.extend([
                                    PathSegment::UserFunctionCall(function_name.clone()),
                                    PathSegment::Argument(function_argument_name.to_string()),
                                ]);
                                let (constant_type, constant_node) = compile_with_context(
                                    &function_argument_body,
                                    argument_compilation_context,
                                    global_compilation_context,
                                )?;
                                let constant_index = global_compilation_context.constants.len();
                                let constant_name_clustered_index =
                                    if let Some(constant_name_clustered_index) =
                                        global_compilation_context
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
                                    };
                                new_constants_indices
                                    .push((constant_name_clustered_index, constant_index));
                                global_compilation_context
                                    .constants
                                    .push((constant_type, constant_node));
                                body_compilation_context
                                    .available_constants
                                    .extend([(function_argument_name.to_string(), constant_index)]);
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
                                return Ok((
                                    function_type.clone(),
                                    Node {
                                        path: compilation_context.path.clone(),
                                        content: Content::UserFunctionCall {
                                            arguments: new_constants_indices,
                                            body: *function_index,
                                        },
                                    },
                                ));
                            } else {
                                body_compilation_context
                                    .entered_user_functions
                                    .extend([function_body.clone()]);
                                let function_index =
                                    global_compilation_context.user_functions.len();
                                global_compilation_context
                                    .user_functions
                                    .push(ProgramOrNode::Program(function_body.clone()));
                                global_compilation_context
                                    .user_function_to_index_and_type_option
                                    .insert(
                                        function_body.clone(),
                                        (function_index, Type::Unknown(function_index)),
                                    );
                                let (function_type, function_node) = compile_with_context(
                                    function_body,
                                    body_compilation_context,
                                    global_compilation_context,
                                )?;
                                global_compilation_context
                                    .user_function_to_index_and_type_option
                                    .get_mut(function_body)
                                    .unwrap()
                                    .1 = function_type.clone();
                                global_compilation_context.user_functions[function_index] =
                                    ProgramOrNode::Node(function_node.clone());
                                return Ok((
                                    function_type,
                                    Node {
                                        path: compilation_context.path.clone(),
                                        content: Content::UserFunctionCall {
                                            arguments: new_constants_indices,
                                            body: function_index,
                                        },
                                    },
                                ));
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
            for (object_key, object_value) in object.iter() {
                let mut object_value_compilation_context = compilation_context.clone();
                object_value_compilation_context
                    .path
                    .0
                    .extend([PathSegment::ObjectKey(object_key.clone())]);
                let (object_value_type, object_value_node) = compile_with_context(
                    object_value,
                    object_value_compilation_context,
                    global_compilation_context,
                )?;
                result_content.insert(object_key.clone(), object_value_node);
                result_inner_types.insert(object_key.clone(), object_value_type);
            }
            (
                Type::Object(result_inner_types),
                Node {
                    path: compilation_context.path.clone(),
                    content: Content::Object(result_content),
                },
            )
        }
        Program::Value(value) => (
            get_value_type(value, compilation_context.clone())?,
            Node {
                path: compilation_context.path.clone(),
                content: Content::Value(value.clone()),
            },
        ),
    })
}
