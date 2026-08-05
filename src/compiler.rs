use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use anyhow::{Context, Error, Result, anyhow};
use gxhash::HashMap;

use crate::{
    computer::Computer,
    containers::{Object, Set, Vector},
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    includes_cache::IncludesCache,
    intermediate_representation::{
        self, Case, Content, IntermediateRepresentation, Node, Throughs, UserFunction,
        ValuePathSegment,
    },
    program::{
        self, AtSegment, Condition, EmbeddedFunction, Path, PathSegment, Program, RangeBound,
    },
    r#type::{MaybeType, Type, TypeAtResult},
    value::Value,
};

#[derive(Clone, Default)]
struct CompilationContext {
    path: Path,
    available_functions: Object<String, Program>,
    available_constants: Object<String, usize>,
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

fn resolve_type(
    got_type: &Type,
    expected_type: &Type,
    compilation_context: &CompilationContext,
) -> Result<Type> {
    if expected_type == &Type::Any || expected_type.contains(got_type) {
        Ok(got_type.clone())
    } else {
        match (got_type, expected_type) {
            (Type::Unknown(unknown_type), expected_type)
            | (expected_type, Type::Unknown(unknown_type)) => {
                let mut unknown_type_write_guard = unknown_type.lockable_internals.write();
                match &*unknown_type_write_guard {
                    Some(got_type) => resolve_type(got_type, expected_type, compilation_context),
                    None => {
                        *unknown_type_write_guard = Some(expected_type.clone());
                        Ok(expected_type.clone())
                    }
                }
            }
            (Type::Array(got_element_type), Type::Array(expected_element_type)) => {
                let mut inner_compilation_context = compilation_context.clone();
                inner_compilation_context
                    .path
                    .0
                    .push(Arc::new(PathSegment::ArrayIndex(0)));
                resolve_type(
                    got_element_type,
                    expected_element_type,
                    &inner_compilation_context,
                )
            }
            (Type::Array(got_element_type), Type::Tuple(expected_elements_types)) => {
                let mut result_union_types = BTreeSet::new();
                for expected_element_type in expected_elements_types {
                    result_union_types.insert(resolve_type(
                        got_element_type,
                        expected_element_type,
                        compilation_context,
                    )?);
                }
                Ok(Type::Array(Box::new(Type::from(result_union_types))))
            }
            (Type::Tuple(got_elements_types), Type::Array(expected_element_type)) => {
                let mut result_tuple_types = Vec::with_capacity(got_elements_types.len());
                for got_element_type in got_elements_types {
                    result_tuple_types.push(resolve_type(
                        got_element_type,
                        expected_element_type,
                        compilation_context,
                    )?);
                }
                Ok(Type::Tuple(result_tuple_types))
            }
            (Type::Object(got_inner_types), Type::Object(expected_inner_types)) => {
                let mut result_inner_types = BTreeMap::new();
                for (expected_value_key, expected_value_type) in expected_inner_types {
                    if let Some(got_value_type) = got_inner_types.get(expected_value_key) {
                        result_inner_types.insert(
                            expected_value_key.clone(),
                            resolve_type(got_value_type, expected_value_type, compilation_context)?,
                        );
                    } else {
                        return Err(compilation_context.error(got_type, expected_type));
                    }
                }
                Ok(Type::Object(result_inner_types))
            }
            (Type::Union(got_union_types), Type::Union(expected_union_types)) => {
                let mut result_union_types = BTreeSet::new();
                if !got_union_types.is_subset(expected_union_types) {
                    for one_of_got_types in got_union_types {
                        let mut found = false;
                        for one_of_expected_types in expected_union_types {
                            if let Ok(result_union_type) = resolve_type(
                                one_of_got_types,
                                one_of_expected_types,
                                compilation_context,
                            ) {
                                result_union_types.insert(result_union_type);
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            return Err(compilation_context.error(got_type, expected_type));
                        }
                    }
                }
                Ok(Type::from(result_union_types))
            }
            (Type::Union(got_union_types), expected_type) => {
                let mut result_union_types = BTreeSet::new();
                for one_of_got_types in got_union_types {
                    result_union_types.insert(resolve_type(
                        one_of_got_types,
                        expected_type,
                        compilation_context,
                    )?);
                }
                Ok(Type::Union(result_union_types))
            }
            (got_type, Type::Union(expected_union_types)) => {
                if !expected_union_types.contains(expected_type) {
                    for one_of_expected_types in expected_union_types {
                        if let Ok(result_type) =
                            resolve_type(got_type, one_of_expected_types, compilation_context)
                        {
                            return Ok(result_type);
                        }
                    }
                    return Err(compilation_context.error(got_type, expected_type));
                }
                Ok(got_type.clone())
            }
            _ => Err(compilation_context.error(got_type, expected_type)),
        }
    }
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct NodeAndMetadata {
    node: Node,
    external_constants_name_clustered_indices: BTreeSet<usize>,
    is_pure: bool,
    is_computable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
struct MaybeCompiledProgram {
    program: Arc<Program>,
    node: Option<Node>,
}

impl Hash for MaybeCompiledProgram {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.program.hash(state);
    }
}

impl From<&Arc<Program>> for MaybeCompiledProgram {
    fn from(program: &Arc<Program>) -> Self {
        Self {
            program: program.clone(),
            node: None,
        }
    }
}

impl From<Arc<Program>> for MaybeCompiledProgram {
    fn from(program: Arc<Program>) -> Self {
        Self {
            program,
            node: None,
        }
    }
}

#[derive(Default)]
struct UserFunctionCallDefinition {
    external_constants_name_clustered_indices: Vec<usize>,
    body: MaybeCompiledProgram,
    is_pure: bool,
}

#[derive(Default, Clone)]
struct ConstantMetadata {
    r#type: Type,
    is_computable: bool,
}

#[derive(Default)]
struct GlobalCompilationContext {
    user_function_to_index_and_type_option: HashMap<MaybeCompiledProgram, (usize, Type)>,
    user_functions_definitions: Vec<UserFunctionCallDefinition>,
    constants_names_to_name_clustered_constants_indices: HashMap<Arc<String>, usize>,
    constants: Vec<ConstantMetadata>,
    includes_cache: IncludesCache,
}

fn process_from_at_program_path_part(
    from: &program::From,
    at: &Vec<AtSegment>,
    includes_cache: &mut IncludesCache,
) -> Result<(Arc<Program>, Option<usize>)> {
    let mut result = includes_cache.get(from)?;
    let mut current_path_segment_index = 0;
    while let Some(current_path_segment) = at.get(current_path_segment_index) {
        match current_path_segment {
            AtSegment::ProgramPathSegment(program_path_segment) => {
                match (&*result, program_path_segment) {
                    (Program::Tuple(tuple), PathSegment::ArrayIndex(tuple_index)) => {
                        result = Arc::new(tuple.get(*tuple_index).unwrap().clone());
                    }
                    (Program::Object(object), PathSegment::ObjectKey(object_key)) => {
                        result = object.get(object_key).unwrap().clone();
                    }
                    (Program::Value(value_arc), PathSegment::ArrayIndex(array_index)) => {
                        match &**value_arc {
                            Some(Value::Tuple(array)) => {
                                result =
                                    Program::Value(array.get(*array_index).unwrap().clone()).into();
                            }
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
                    (Program::Value(value_arc), PathSegment::ObjectKey(object_key)) => {
                        match &**value_arc {
                            Some(Value::Object(object)) => {
                                result =
                                    Program::Value(object.get(object_key).unwrap().clone()).into();
                            }
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
                        result = compute.clone();
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

#[derive(Default, Clone)]
pub struct Compiler {
    pub metaprograms_computer: Computer,
}

impl Compiler {
    pub fn compile(&self, program: &Program) -> Result<Arc<IntermediateRepresentation>> {
        let mut global_compilation_context = GlobalCompilationContext::default();
        let result = self.compile_with_context(
            program,
            &CompilationContext::default(),
            &mut global_compilation_context,
        )?;
        if !result.is_computable {
            return Err(anyhow!("expected finite program",));
        }
        Ok(Arc::new(IntermediateRepresentation {
            root: result.node,
            user_functions: global_compilation_context
                .user_functions_definitions
                .into_iter()
                .map(|user_function_definition| UserFunction {
                    external_constants_name_clustered_indices: user_function_definition
                        .external_constants_name_clustered_indices,
                    node: user_function_definition.body.node.unwrap().clone(),
                    is_pure: user_function_definition.is_pure,
                })
                .collect(),
            unique_constants_names_count: global_compilation_context
                .constants_names_to_name_clustered_constants_indices
                .len(),
        }))
    }

    fn define_constant(
        &self,
        name: Arc<String>,
        constant_metadata: ConstantMetadata,
        compilation_context: &mut CompilationContext,
        global_compilation_context: &mut GlobalCompilationContext,
    ) -> ConstantDefinition {
        let result = ConstantDefinition {
            index: {
                let result = global_compilation_context.constants.len();
                global_compilation_context.constants.push(constant_metadata);
                result
            },
            name_clustered_index: if let Some(constant_name_clustered_index) =
                global_compilation_context
                    .constants_names_to_name_clustered_constants_indices
                    .get(&*name)
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
            .insert(name, Arc::new(result.index));
        result
    }

    fn compile_with_context(
        &self,
        program: &Program,
        compilation_context: &CompilationContext,
        global_compilation_context: &mut GlobalCompilationContext,
    ) -> Result<NodeAndMetadata> {
        Ok(match program {
            Program::Tuple(tuple) => {
                if tuple.is_empty() {
                    return Ok(NodeAndMetadata {
                        external_constants_name_clustered_indices: BTreeSet::new(),
                        node: Node {
                            content: Content::Value(
                                Some(intermediate_representation::Value::Tuple(Vector::default()))
                                    .into(),
                            ),
                            r#type: Type::Tuple([].into()),
                        },
                        is_pure: true,
                        is_computable: true,
                    });
                }
                let mut result_content = Vec::with_capacity(tuple.len());
                let mut result_external_constants_name_clustered_indices = BTreeSet::new();
                let mut result_elements_types = Vec::with_capacity(tuple.len());
                let mut is_pure = true;
                let mut is_computable = true;
                for (element_index, element) in tuple.iter().enumerate() {
                    let mut element_compilation_context = compilation_context.clone();
                    element_compilation_context
                        .path
                        .0
                        .extend([PathSegment::ArrayIndex(element_index).into()]);
                    let mut compiled_element = self.compile_with_context(
                        element,
                        &element_compilation_context,
                        global_compilation_context,
                    )?;
                    result_elements_types.push(compiled_element.node.r#type.clone());
                    result_content.push(Arc::new(compiled_element.node));
                    result_external_constants_name_clustered_indices
                        .append(&mut compiled_element.external_constants_name_clustered_indices);
                    is_pure &= compiled_element.is_pure;
                    is_computable &= compiled_element.is_computable;
                }
                NodeAndMetadata {
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::Tuple(result_content),
                        r#type: Type::Tuple(result_elements_types),
                    },
                    is_pure,
                    is_computable,
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
                    .extend([PathSegment::Compute.into()]);
                let mut new_constants = Vec::with_capacity(constants.len());
                let mut result_external_constants_name_clustered_indices = BTreeSet::new();
                let mut constants_name_clustered_indices = Vec::with_capacity(constants.len());
                let mut is_pure = true;
                let mut is_computable = true;
                for (constant_name, constant_compute_body) in constants.iter() {
                    let mut constant_compilation_context = compilation_context.clone();
                    constant_compilation_context.path.0.extend([
                        PathSegment::Constants.into(),
                        PathSegment::Constant(constant_name.clone()).into(),
                    ]);
                    let mut compiled_constant = self.compile_with_context(
                        constant_compute_body,
                        &constant_compilation_context,
                        global_compilation_context,
                    )?;
                    result_external_constants_name_clustered_indices
                        .append(&mut compiled_constant.external_constants_name_clustered_indices);
                    let constant_definition = self.define_constant(
                        constant_name.clone(),
                        ConstantMetadata {
                            r#type: compiled_constant.node.r#type.clone(),
                            is_computable: compiled_constant.is_computable,
                        },
                        &mut compute_compilation_context,
                        global_compilation_context,
                    );
                    constants_name_clustered_indices.push(constant_definition.name_clustered_index);
                    new_constants.push(intermediate_representation::ConstantDefinition {
                        name_clustered_index: constant_definition.name_clustered_index,
                        node: Arc::new(compiled_constant.node),
                    });
                    is_pure &= compiled_constant.is_pure;
                    is_computable &= compiled_constant.is_computable;
                }
                for (function_name, function_body) in functions.iter() {
                    if !function_name.ends_with(":") {
                        let mut function_compilation_context = compilation_context.clone();
                        function_compilation_context.path.0.extend([
                            PathSegment::Functions.into(),
                            PathSegment::Function(function_name.clone()).into(),
                        ]);
                        return Err(anyhow!(
                            "expected function named {:?}, found function named {function_name:?} \
                             at {:#?}",
                            format!("{function_name}:"),
                            function_compilation_context.path
                        ));
                    }
                    compute_compilation_context
                        .available_functions
                        .insert(function_name.clone(), function_body.clone());
                }
                let mut compiled_compute = self.compile_with_context(
                    compute,
                    &compute_compilation_context,
                    global_compilation_context,
                )?;
                for constant_name_clustered_index in constants_name_clustered_indices {
                    compiled_compute
                        .external_constants_name_clustered_indices
                        .remove(&constant_name_clustered_index);
                }
                result_external_constants_name_clustered_indices
                    .append(&mut compiled_compute.external_constants_name_clustered_indices);
                is_pure &= compiled_compute.is_pure;
                is_computable &= compiled_compute.is_computable;
                let result_type = compiled_compute.node.r#type.clone();
                NodeAndMetadata {
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::Scope {
                            constants: new_constants,
                            compute: Box::new(compiled_compute.node),
                        },
                        r#type: result_type,
                    },
                    is_pure,
                    is_computable,
                }
            }
            Program::Constant {
                constant: constant_name,
            } => {
                if let Some(constant_index) =
                    compilation_context.available_constants.get(constant_name)
                {
                    let constant_metadata =
                        global_compilation_context.constants[**constant_index].clone();
                    let name_clustered_constant_index = *global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .get(constant_name)
                        .unwrap();
                    NodeAndMetadata {
                        external_constants_name_clustered_indices: BTreeSet::from_iter([
                            name_clustered_constant_index,
                        ]),
                        node: Node {
                            content: Content::Constant(name_clustered_constant_index),
                            r#type: constant_metadata.r#type,
                        },
                        is_pure: true,
                        is_computable: constant_metadata.is_computable,
                    }
                } else {
                    return Err(anyhow!(
                        "expected one of available constants {:#?}, found {constant_name:?} at \
                         {:#?}",
                        compilation_context
                            .available_constants
                            .keys()
                            .collect::<Vec<_>>(),
                        compilation_context.path,
                    ));
                }
            }
            Program::DefaultArgument(_) => self.compile_with_context(
                &Program::Constant {
                    constant: DEFAULT_ARGUMENT_NAME.to_string().into(),
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
                    .extend([PathSegment::From.into()]);
                let compiled_extracted_from = self.compile_with_context(
                    &extracted_from,
                    &from_program_compilation_context,
                    global_compilation_context,
                )?;
                let mut is_pure = compiled_extracted_from.is_pure;
                let mut is_computable = compiled_extracted_from.is_computable;
                let mut external_constants_name_clustered_indices =
                    compiled_extracted_from.external_constants_name_clustered_indices;
                let value_path = if let Some(first_non_program_path_segment_index) =
                    first_non_program_path_segment_index_option
                {
                    let mut result =
                        Vec::with_capacity(at[first_non_program_path_segment_index..].len());
                    for at_segment in &at[first_non_program_path_segment_index..] {
                        result.push(match at_segment {
                            AtSegment::ValueArrayIndex(array_index) => {
                                ValuePathSegment::ArrayIndex(*array_index)
                            }
                            AtSegment::ValueObjectKey(object_key) => {
                                ValuePathSegment::ObjectKey(object_key.clone())
                            }
                            AtSegment::ValueArrayRange((from, to)) => {
                                ValuePathSegment::ArrayRange {
                                    from: Box::new({
                                        match &**from {
                                            RangeBound::Static(from_static) => {
                                                intermediate_representation::RangeBound::Static(
                                                    *from_static,
                                                )
                                            }
                                            RangeBound::Dynamic(from_dynamic) => {
                                                let mut compiled_from_dynamic = self
                                                    .compile_with_context(
                                                        from_dynamic,
                                                        compilation_context,
                                                        global_compilation_context,
                                                    )?;
                                                resolve_type(
                                                    &compiled_from_dynamic.node.r#type,
                                                    &Type::Number,
                                                    compilation_context,
                                                )?;
                                                is_pure &= compiled_from_dynamic.is_pure;
                                                is_computable &=
                                                    compiled_from_dynamic.is_computable;
                                                external_constants_name_clustered_indices.append(
                                                    &mut compiled_from_dynamic
                                                        .external_constants_name_clustered_indices,
                                                );
                                                intermediate_representation::RangeBound::Dynamic(
                                                    compiled_from_dynamic.node,
                                                )
                                            }
                                        }
                                    }),
                                    to: Box::new({
                                        match &**to {
                                            RangeBound::Static(to_static) => {
                                                intermediate_representation::RangeBound::Static(
                                                    *to_static,
                                                )
                                            }
                                            RangeBound::Dynamic(to_dynamic) => {
                                                let mut compiled_to_dynamic = self
                                                    .compile_with_context(
                                                        to_dynamic,
                                                        compilation_context,
                                                        global_compilation_context,
                                                    )?;
                                                resolve_type(
                                                    &compiled_to_dynamic.node.r#type,
                                                    &Type::Number,
                                                    compilation_context,
                                                )?;
                                                is_pure &= compiled_to_dynamic.is_pure;
                                                is_computable &= compiled_to_dynamic.is_computable;
                                                external_constants_name_clustered_indices.append(
                                                    &mut compiled_to_dynamic
                                                        .external_constants_name_clustered_indices,
                                                );
                                                intermediate_representation::RangeBound::Dynamic(
                                                    compiled_to_dynamic.node,
                                                )
                                            }
                                        }
                                    }),
                                }
                            }
                            _ => {
                                return Err(anyhow!(
                                    "can not treat {at_segment:?} as value path segment"
                                ));
                            }
                        });
                    }
                    result
                } else {
                    vec![]
                };
                let r#type = match compiled_extracted_from
                    .node
                    .r#type
                    .clone()
                    .at_path(&value_path)
                    .with_context(|| {
                        format!(
                            "expected value with path {value_path:#?}, found {:#?} at {:#?}",
                            compiled_extracted_from.node.r#type, compilation_context.path
                        )
                    })? {
                    TypeAtResult::Single(r#type) => r#type,
                    TypeAtResult::Multiple(union_types) => Type::Union(union_types),
                };
                NodeAndMetadata {
                    node: Node {
                        content: Content::FromAt {
                            from: Box::new(compiled_extracted_from.node),
                            value_path_segments: value_path,
                        },
                        r#type,
                    },
                    external_constants_name_clustered_indices,
                    is_pure,
                    is_computable,
                }
            }
            Program::EmbeddedFunction(embedded_function) => match &**embedded_function {
                EmbeddedFunction::Sum(argument) => {
                    let mut argument_compilation_context = compilation_context.clone();
                    argument_compilation_context
                        .path
                        .0
                        .extend([PathSegment::Sum.into()]);
                    let compiled_argument = self.compile_with_context(
                        argument,
                        &argument_compilation_context,
                        global_compilation_context,
                    )?;
                    if !compiled_argument.is_computable {
                        return Err(anyhow!(
                            "expected computable argument, found {argument:#?} at {:#?}",
                            argument_compilation_context.path
                        ));
                    }
                    resolve_type(
                        &compiled_argument.node.r#type,
                        &Type::Array(Box::new(Type::Number)),
                        compilation_context,
                    )?;
                    NodeAndMetadata {
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
                            r#type: Type::Number,
                        },
                        is_pure: compiled_argument.is_pure,
                        is_computable: true,
                    }
                }
                EmbeddedFunction::IsSorted(argument) => {
                    let mut argument_compilation_context = compilation_context.clone();
                    argument_compilation_context
                        .path
                        .0
                        .extend([PathSegment::IsSorted.into()]);
                    let compiled_argument = self.compile_with_context(
                        argument,
                        &argument_compilation_context,
                        global_compilation_context,
                    )?;
                    if !compiled_argument.is_computable {
                        return Err(anyhow!(
                            "expected computable argument, found {argument:#?} at {:#?}",
                            argument_compilation_context.path
                        ));
                    }
                    resolve_type(
                        &compiled_argument.node.r#type,
                        &Type::Array(Box::new(Type::Any)),
                        compilation_context,
                    )?;
                    NodeAndMetadata {
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
                            r#type: Type::Bool,
                        },
                        is_pure: compiled_argument.is_pure,
                        is_computable: compiled_argument.is_computable,
                    }
                }
                EmbeddedFunction::StandardInput => NodeAndMetadata {
                    external_constants_name_clustered_indices: BTreeSet::new(),
                    node: Node {
                        content: Content::EmbeddedFunctionCall {
                            path: Some(Path(
                                compilation_context
                                    .path
                                    .0
                                    .extended([PathSegment::StandardInput.into()]),
                            )),
                            embedded_function: Box::new(
                                intermediate_representation::EmbeddedFunction::StandardInput,
                            ),
                        },
                        r#type: Type::String,
                    },
                    is_pure: true,
                    is_computable: true,
                },
                EmbeddedFunction::ParseYaml(argument) => {
                    let mut argument_compilation_context = compilation_context.clone();
                    argument_compilation_context
                        .path
                        .0
                        .extend([PathSegment::ParseYaml.into()]);
                    let compiled_argument = self.compile_with_context(
                        argument,
                        &argument_compilation_context,
                        global_compilation_context,
                    )?;
                    if !compiled_argument.is_computable {
                        return Err(anyhow!(
                            "expected computable argument, found {argument:#?} at {:#?}",
                            argument_compilation_context.path
                        ));
                    }
                    resolve_type(
                        &compiled_argument.node.r#type,
                        &Type::String,
                        &argument_compilation_context,
                    )?;
                    NodeAndMetadata {
                        external_constants_name_clustered_indices: compiled_argument
                            .external_constants_name_clustered_indices,
                        node: Node {
                            content: Content::EmbeddedFunctionCall {
                                path: Some(Path(
                                    compilation_context
                                        .path
                                        .0
                                        .extended([PathSegment::ParseYaml.into()]),
                                )),
                                embedded_function: Box::new(
                                    intermediate_representation::EmbeddedFunction::ParseYaml(
                                        compiled_argument.node,
                                    ),
                                ),
                            },
                            r#type: Type::Any,
                        },
                        is_pure: compiled_argument.is_pure,
                        is_computable: true,
                    }
                }
                EmbeddedFunction::KeyValuePairs(argument) => {
                    let mut argument_compilation_context = compilation_context.clone();
                    argument_compilation_context
                        .path
                        .0
                        .extend([PathSegment::KeyValuePairs.into()]);
                    let compiled_argument = self.compile_with_context(
                        argument,
                        &argument_compilation_context,
                        global_compilation_context,
                    )?;
                    if let Type::Object(argument_object_values_types) =
                        compiled_argument.node.r#type.clone()
                    {
                        NodeAndMetadata {
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
                                    r#type: Type::Tuple(
                                        argument_object_values_types.values().map(|value| {
                                                Type::Tuple(vec![
                                                    Type::String,
                                                    value.clone(),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                },
                                is_pure: compiled_argument.is_pure,
                                is_computable: compiled_argument.is_computable
                            }
                    } else {
                        return Err(anyhow!(
                            "expected object, found {:#?} at {:#?}",
                            compiled_argument.node.r#type,
                            compilation_context.path
                        ));
                    }
                }
                EmbeddedFunction::Flatten(argument) => {
                    let mut argument_compilation_context = compilation_context.clone();
                    argument_compilation_context
                        .path
                        .0
                        .extend([PathSegment::Flatten.into()]);
                    let compiled_argument = self.compile_with_context(
                        argument,
                        &argument_compilation_context,
                        global_compilation_context,
                    )?;
                    let compiled_argument_resolved_type = resolve_type(
                        &compiled_argument.node.r#type,
                        &Type::Array(Box::new(Type::Array(Box::new(Type::Any)))),
                        compilation_context,
                    )?;
                    let r#type = compiled_argument_resolved_type.flatten().with_context(|| {
                        format!(
                            "expected flattenable type, found \
                             {compiled_argument_resolved_type:#?} at {:#?}",
                            compilation_context.path
                        )
                    })?;
                    NodeAndMetadata {
                        external_constants_name_clustered_indices: compiled_argument
                            .external_constants_name_clustered_indices,
                        node: Node {
                            content: Content::EmbeddedFunctionCall {
                                path: None,
                                embedded_function: Box::new(
                                    intermediate_representation::EmbeddedFunction::Flatten(
                                        compiled_argument.node,
                                    ),
                                ),
                            },
                            r#type,
                        },
                        is_pure: compiled_argument.is_pure,
                        is_computable: compiled_argument.is_computable,
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
                    .extend([PathSegment::Match.into()]);
                let compiled_match = self.compile_with_context(
                    r#match,
                    &match_compilation_context,
                    global_compilation_context,
                )?;
                if !compiled_match.is_computable {
                    return Err(anyhow!(
                        "expected computable match, found {compiled_match:#?} at {:#?}",
                        match_compilation_context.path
                    ));
                }
                if !compiled_match.node.r#type.is_known() {
                    return Err(anyhow!(
                        "expected match of known type, found {:#?} at {:#?}",
                        compiled_match.node.r#type,
                        match_compilation_context.path
                    ));
                }
                let mut result_cases = Vec::new();
                let mut result_types = BTreeSet::new();
                let mut result_external_constants_name_clustered_indices =
                    compiled_match.external_constants_name_clustered_indices;
                let mut case_is_pure = true;
                let mut case_is_computable = true;
                let mut covered_types = BTreeSet::new();
                let match_constant_name_clustered_index_option =
                    if let Some(match_constant_name) = r#as {
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
                for (case_index, (case_condition, case)) in cases.iter().enumerate() {
                    let mut case_compilation_context = compilation_context.clone();
                    case_compilation_context.path.0.extend([
                        PathSegment::Cases.into(),
                        PathSegment::Case(case_index).into(),
                    ]);
                    match case_condition {
                        Condition::Type(refined_match_type) => {
                            if compiled_match
                                .node
                                .r#type
                                .intersection(refined_match_type)
                                .is_none()
                            {
                                continue;
                            };
                            if let Some(match_constant_name) = r#as {
                                self.define_constant(
                                    match_constant_name.clone(),
                                    ConstantMetadata {
                                        r#type: refined_match_type.clone(),
                                        is_computable: compiled_match.is_computable,
                                    },
                                    &mut case_compilation_context,
                                    global_compilation_context,
                                );
                            };
                            let mut compiled_case = self.compile_with_context(
                                case,
                                &case_compilation_context,
                                global_compilation_context,
                            )?;
                            result_types.insert(compiled_case.node.r#type.clone());
                            result_external_constants_name_clustered_indices.append(
                                &mut compiled_case.external_constants_name_clustered_indices,
                            );
                            covered_types.insert(refined_match_type.clone());

                            case_is_pure &= compiled_case.is_pure;
                            case_is_computable &= compiled_case.is_computable;
                            result_cases.push(Case {
                                condition: intermediate_representation::Condition::Type(
                                    refined_match_type.clone(),
                                ),
                                node: compiled_case.node,
                            });
                        }
                        Condition::Value(condition) => {
                            let compiled_condition = self.compile_with_context(
                                condition,
                                &case_compilation_context,
                                global_compilation_context,
                            )?;
                            if !compiled_condition.is_computable {
                                return Err(anyhow!(
                                    "expected computable condition, found {condition:#?} at {:#?}",
                                    case_compilation_context.path
                                ));
                            }
                            let refined_match_type = compiled_condition.node.r#type.clone();
                            if compiled_match
                                .node
                                .r#type
                                .intersection(&refined_match_type)
                                .is_none()
                                && compiled_match
                                    .node
                                    .r#type
                                    .intersection(&refined_match_type)
                                    .is_none()
                            {
                                continue;
                            };
                            if let Some(match_constant_name) = r#as {
                                self.define_constant(
                                    match_constant_name.clone(),
                                    ConstantMetadata {
                                        r#type: refined_match_type.clone(),
                                        is_computable: compiled_match.is_computable,
                                    },
                                    &mut case_compilation_context,
                                    global_compilation_context,
                                );
                            }
                            let mut compiled_case = self.compile_with_context(
                                case,
                                &case_compilation_context,
                                global_compilation_context,
                            )?;
                            result_types.insert(compiled_case.node.r#type.clone());
                            result_external_constants_name_clustered_indices.append(
                                &mut compiled_case.external_constants_name_clustered_indices,
                            );
                            if refined_match_type == Type::LiteralTrue
                                || refined_match_type == Type::LiteralFalse
                            {
                                covered_types.insert(refined_match_type);
                            }
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
                if !covered.contains(&compiled_match.node.r#type) {
                    return Err(anyhow!(
                        "expected coverage for {:#?}, found coverage only for {covered:#?} at \
                         {:#?}",
                        compiled_match.node.r#type,
                        compilation_context.path
                    ));
                }
                match result_cases.len() {
                    0 => {
                        return Err(anyhow!(
                            "expected at least one valid case for match {:#?} at path {:#?}",
                            compiled_match.node.r#type,
                            match_compilation_context.path
                        ));
                    }
                    _ => NodeAndMetadata {
                        node: Node {
                            content: Content::Match {
                                r#match: Box::new(compiled_match.node),
                                cases: result_cases,
                                match_constant_name_clustered_index_option,
                            },
                            r#type: Type::from(result_types),
                        },
                        external_constants_name_clustered_indices:
                            result_external_constants_name_clustered_indices,
                        is_pure: compiled_match.is_pure && case_is_pure,
                        is_computable: case_is_computable,
                    },
                }
            }
            Program::Map { map, r#as, through } => {
                let mut map_compilation_context = compilation_context.clone();
                map_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Map.into()]);
                let compiled_map = self.compile_with_context(
                    map,
                    &map_compilation_context,
                    global_compilation_context,
                )?;
                let mut result_external_constants_name_clustered_indices =
                    compiled_map.external_constants_name_clustered_indices;
                let mut is_pure = compiled_map.is_pure;
                let mut is_computable = compiled_map.is_computable;
                let map_constant_name_clustered_index = if let Some(result) =
                    global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .get(r#as)
                {
                    *result
                } else {
                    global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .len()
                };
                let mut map_concrete_type_and_throughs =
                    Vec::with_capacity(compiled_map.node.r#type.union_types_len());
                let mut result_union_types = BTreeSet::new();
                for map_concrete_type in compiled_map.node.r#type.union_types() {
                    map_concrete_type_and_throughs.push((
                        map_concrete_type.clone(),
                        match map_concrete_type {
                            Type::Tuple(map_tuple_elements_types) => {
                                let mut result_elements_types =
                                    Vec::with_capacity(map_tuple_elements_types.len());
                                let mut result_throughs_nodes_indexes =
                                    Vec::with_capacity(map_tuple_elements_types.len());
                                let mut compiled_throughs: indexmap::IndexSet<NodeAndMetadata> =
                                    indexmap::IndexSet::new();
                                let mut element_type_to_compiled_through_index: BTreeMap<
                                    Type,
                                    usize,
                                > = BTreeMap::new();
                                for (element_type_index, element_type) in
                                    map_tuple_elements_types.iter().enumerate()
                                {
                                    if let Some(element_through_index) =
                                        element_type_to_compiled_through_index.get(element_type)
                                    {
                                        result_elements_types.push(
                                            compiled_throughs[*element_through_index]
                                                .node
                                                .r#type
                                                .clone(),
                                        );
                                        result_throughs_nodes_indexes.push(*element_through_index);
                                    } else {
                                        let mut through_compilation_context =
                                            compilation_context.clone();
                                        through_compilation_context.path.0.extend([
                                            PathSegment::Through(element_type_index).into(),
                                        ]);
                                        self.define_constant(
                                            r#as.clone(),
                                            ConstantMetadata {
                                                r#type: element_type.clone(),
                                                is_computable: compiled_map.is_computable,
                                            },
                                            &mut through_compilation_context,
                                            global_compilation_context,
                                        );
                                        let compiled_through = self.compile_with_context(
                                            through,
                                            &through_compilation_context,
                                            global_compilation_context,
                                        )?;
                                        result_elements_types
                                            .push(compiled_through.node.r#type.clone());
                                        let compiled_through_index =
                                            if let Some(compiled_through_index) =
                                                compiled_throughs.get_index_of(&compiled_through)
                                            {
                                                compiled_through_index
                                            } else {
                                                result_external_constants_name_clustered_indices
                                                    .extend(
                                                    compiled_through
                                                        .external_constants_name_clustered_indices
                                                        .clone(),
                                                );
                                                is_pure &= compiled_through.is_pure;
                                                is_computable &= compiled_through.is_computable;
                                                compiled_throughs.insert(compiled_through);
                                                compiled_throughs.len() - 1
                                            };
                                        element_type_to_compiled_through_index
                                            .insert(element_type.clone(), compiled_through_index);
                                        result_throughs_nodes_indexes.push(compiled_through_index);
                                    }
                                }
                                result_union_types.insert(Type::Tuple(result_elements_types));
                                Throughs::Tuple {
                                    nodes_indexes: result_throughs_nodes_indexes,
                                    nodes: compiled_throughs
                                        .into_iter()
                                        .map(|compiled_through| compiled_through.node)
                                        .collect(),
                                }
                            }
                            Type::Array(map_array_element_type) => {
                                let mut through_compilation_context = compilation_context.clone();
                                through_compilation_context
                                    .path
                                    .0
                                    .extend([PathSegment::Through(0).into()]);
                                self.define_constant(
                                    r#as.clone(),
                                    ConstantMetadata {
                                        r#type: *map_array_element_type.clone(),
                                        is_computable: compiled_map.is_computable,
                                    },
                                    &mut through_compilation_context,
                                    global_compilation_context,
                                );
                                let compiled_through = self.compile_with_context(
                                    through,
                                    &through_compilation_context,
                                    global_compilation_context,
                                )?;
                                result_external_constants_name_clustered_indices.extend(
                                    compiled_through.external_constants_name_clustered_indices,
                                );
                                is_pure &= compiled_through.is_pure;
                                is_computable &= compiled_through.is_computable;
                                result_union_types.insert(Type::Array(Box::new(
                                    compiled_through.node.r#type.clone(),
                                )));
                                Throughs::Array(Box::new(compiled_through.node))
                            }
                            _ => {
                                return Err(anyhow!(
                                    "expected tuple or array, found {:#?} at {:#?}",
                                    map_concrete_type,
                                    map_compilation_context.path
                                ));
                            }
                        },
                    ));
                }
                NodeAndMetadata {
                    node: Node {
                        content: Content::Map(Arc::new(intermediate_representation::Map {
                            map: Box::new(compiled_map.node),
                            map_concrete_type_and_throughs,
                            map_constant_name_clustered_index,
                        })),
                        r#type: Type::from(result_union_types),
                    },
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    is_pure,
                    is_computable,
                }
            }
            Program::Filter {
                filter,
                r#as,
                through,
            } => {
                let mut filter_compilation_context = compilation_context.clone();
                filter_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Filter.into()]);
                let compiled_filter = self.compile_with_context(
                    filter,
                    &filter_compilation_context,
                    global_compilation_context,
                )?;
                let mut result_external_constants_name_clustered_indices =
                    compiled_filter.external_constants_name_clustered_indices;
                let mut is_pure = compiled_filter.is_pure;
                let mut is_computable = compiled_filter.is_computable;
                let filter_constant_name_clustered_index = if let Some(result) =
                    global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .get(r#as)
                {
                    *result
                } else {
                    global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .len()
                };
                let mut filter_concrete_type_and_throughs =
                    Vec::with_capacity(compiled_filter.node.r#type.union_types_len());
                let mut result_union_types = BTreeSet::new();
                for filter_concrete_type in compiled_filter.node.r#type.union_types() {
                    filter_concrete_type_and_throughs.push((
                        filter_concrete_type.clone(),
                        match filter_concrete_type {
                            Type::Tuple(filter_tuple_elements_types) => {
                                let mut result_throughs_nodes_indexes =
                                    Vec::with_capacity(filter_tuple_elements_types.len());
                                let mut compiled_throughs: indexmap::IndexSet<NodeAndMetadata> =
                                    indexmap::IndexSet::new();
                                for (element_type_index, element_type) in
                                    filter_tuple_elements_types.iter().enumerate()
                                {
                                    let mut through_compilation_context =
                                        compilation_context.clone();
                                    through_compilation_context
                                        .path
                                        .0
                                        .extend([PathSegment::Through(element_type_index).into()]);
                                    self.define_constant(
                                        r#as.clone(),
                                        ConstantMetadata {
                                            r#type: element_type.clone(),
                                            is_computable: compiled_filter.is_computable,
                                        },
                                        &mut through_compilation_context,
                                        global_compilation_context,
                                    );
                                    let compiled_through = self.compile_with_context(
                                        through,
                                        &through_compilation_context,
                                        global_compilation_context,
                                    )?;
                                    resolve_type(
                                        &compiled_through.node.r#type,
                                        &Type::Bool,
                                        compilation_context,
                                    )?;
                                    let compiled_through_index =
                                        if let Some(compiled_through_index) =
                                            compiled_throughs.get_index_of(&compiled_through)
                                        {
                                            compiled_through_index
                                        } else {
                                            result_external_constants_name_clustered_indices
                                                .extend(
                                                    compiled_through
                                                        .external_constants_name_clustered_indices
                                                        .clone(),
                                                );
                                            is_pure &= compiled_through.is_pure;
                                            is_computable &= compiled_through.is_computable;
                                            compiled_throughs.insert(compiled_through);
                                            compiled_throughs.len() - 1
                                        };
                                    result_throughs_nodes_indexes.push(compiled_through_index);
                                }
                                result_union_types.insert(Type::Array(Box::new(Type::from(
                                    BTreeSet::from_iter(
                                        filter_tuple_elements_types.iter().cloned(),
                                    ),
                                ))));
                                Throughs::Tuple {
                                    nodes_indexes: result_throughs_nodes_indexes,
                                    nodes: compiled_throughs
                                        .into_iter()
                                        .map(|compiled_through| compiled_through.node)
                                        .collect(),
                                }
                            }
                            Type::Array(filter_array_element_type) => {
                                let mut through_compilation_context = compilation_context.clone();
                                through_compilation_context
                                    .path
                                    .0
                                    .extend([PathSegment::Through(0).into()]);
                                self.define_constant(
                                    r#as.clone(),
                                    ConstantMetadata {
                                        r#type: *filter_array_element_type.clone(),
                                        is_computable: compiled_filter.is_computable,
                                    },
                                    &mut through_compilation_context,
                                    global_compilation_context,
                                );
                                let compiled_through = self.compile_with_context(
                                    through,
                                    &through_compilation_context,
                                    global_compilation_context,
                                )?;
                                resolve_type(
                                    &compiled_through.node.r#type,
                                    &Type::Bool,
                                    compilation_context,
                                )?;
                                result_external_constants_name_clustered_indices.extend(
                                    compiled_through.external_constants_name_clustered_indices,
                                );
                                is_pure &= compiled_through.is_pure;
                                is_computable &= compiled_through.is_computable;
                                result_union_types.insert(filter_concrete_type.clone());
                                Throughs::Array(Box::new(compiled_through.node))
                            }
                            _ => {
                                return Err(anyhow!(
                                    "expected tuple or array, found {:#?} at {:#?}",
                                    filter_concrete_type,
                                    filter_compilation_context.path
                                ));
                            }
                        },
                    ));
                }
                NodeAndMetadata {
                    node: Node {
                        content: Content::Filter(Arc::new(intermediate_representation::Filter {
                            filter: Box::new(compiled_filter.node),
                            filter_concrete_type_and_throughs,
                            filter_constant_name_clustered_index,
                        })),
                        r#type: Type::from(result_union_types),
                    },
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    is_pure,
                    is_computable,
                }
            }
            Program::Fold {
                fold,
                r#as,
                starting_with,
                accumulating_in,
                through,
            } => {
                let mut fold_compilation_context = compilation_context.clone();
                fold_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Fold.into()]);
                let compiled_fold = self.compile_with_context(
                    fold,
                    &fold_compilation_context,
                    global_compilation_context,
                )?;
                if !compiled_fold.is_computable {
                    return Err(anyhow!(
                        "expected computable fold, found {fold:#?} at {:#?}",
                        fold_compilation_context.path
                    ));
                }
                let mut result_external_constants_name_clustered_indices =
                    compiled_fold.external_constants_name_clustered_indices;
                let mut is_pure = compiled_fold.is_pure;
                let mut starting_with_compilation_context = compilation_context.clone();
                starting_with_compilation_context
                    .path
                    .0
                    .extend([PathSegment::StartingWith.into()]);
                let compiled_starting_with = self.compile_with_context(
                    starting_with,
                    &starting_with_compilation_context,
                    global_compilation_context,
                )?;
                if !compiled_fold.is_computable {
                    return Err(anyhow!(
                        "expected computable starting-with, found {starting_with:#?} at {:#?}",
                        starting_with_compilation_context.path
                    ));
                }
                result_external_constants_name_clustered_indices
                    .extend(compiled_starting_with.external_constants_name_clustered_indices);
                is_pure &= compiled_starting_with.is_pure;
                let fold_constant_name_clustered_index = if let Some(result) =
                    global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .get(r#as)
                {
                    *result
                } else {
                    global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .len()
                };
                let accumulating_in_constant_name_clustered_index = if let Some(result) =
                    global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .get(accumulating_in)
                {
                    *result
                } else {
                    global_compilation_context
                        .constants_names_to_name_clustered_constants_indices
                        .len()
                        + 1
                };
                let mut fold_concrete_type_and_throughs =
                    Vec::with_capacity(compiled_fold.node.r#type.union_types_len());
                let mut result_union_types = BTreeSet::new();
                for fold_concrete_type in compiled_fold.node.r#type.union_types() {
                    fold_concrete_type_and_throughs.push((
                        fold_concrete_type.clone(),
                        match &fold_concrete_type {
                            Type::Tuple(fold_tuple_elements_types) => {
                                let mut result_type = compiled_starting_with.node.r#type.clone();
                                let mut result_throughs_nodes_indexes =
                                    Vec::with_capacity(fold_tuple_elements_types.len());
                                let mut compiled_throughs: indexmap::IndexSet<NodeAndMetadata> =
                                    indexmap::IndexSet::new();
                                let mut current_type_and_accumulating_in_type_to_compiled_through_index: BTreeMap<(Type, Type), usize> =
                                BTreeMap::new();
                                for (current_type_index, current_type) in
                                    fold_tuple_elements_types.iter().enumerate()
                                {
                                    if let Some(element_through_index) =
                                        current_type_and_accumulating_in_type_to_compiled_through_index
                                            .get(&(current_type.clone(), result_type.clone()))
                                    {
                                        result_type = compiled_throughs[*element_through_index]
                                            .node
                                            .r#type
                                            .clone();
                                        result_throughs_nodes_indexes.push(*element_through_index);
                                    } else {
                                        let mut through_compilation_context = compilation_context.clone();
                                        through_compilation_context
                                            .path
                                            .0
                                            .extend([PathSegment::Through(current_type_index).into()]);
                                        self.define_constant(
                                            r#as.clone(),
                                            ConstantMetadata {
                                                r#type: current_type.clone(),
                                                is_computable: true,
                                            },
                                            &mut through_compilation_context,
                                            global_compilation_context,
                                        );
                                        self.define_constant(
                                            accumulating_in.clone(),
                                            ConstantMetadata {
                                                r#type: result_type.clone(),
                                                is_computable: true,
                                            },
                                            &mut through_compilation_context,
                                            global_compilation_context,
                                        );
                                        let compiled_through = self.compile_with_context(
                                            through,
                                            &through_compilation_context,
                                            global_compilation_context,
                                        )?;
                                        if !compiled_fold.is_computable {
                                            return Err(anyhow!(
                                                "expected computable through, found {through:#?} at {:#?}",
                                                through_compilation_context.path
                                            ));
                                        }
                                        result_type = compiled_through.node.r#type.clone();
                                        let compiled_through_index = if let Some(compiled_through_index) =
                                            compiled_throughs.get_index_of(&compiled_through)
                                        {
                                            compiled_through_index
                                        } else {
                                            result_external_constants_name_clustered_indices.extend(
                                                compiled_through
                                                    .external_constants_name_clustered_indices
                                                    .clone(),
                                            );
                                            is_pure &= compiled_through.is_pure;
                                            compiled_throughs.insert(compiled_through);
                                            compiled_throughs.len() - 1
                                        };
                                        current_type_and_accumulating_in_type_to_compiled_through_index
                                            .insert(
                                                (current_type.clone(), result_type.clone()),
                                                compiled_through_index,
                                            );
                                        result_throughs_nodes_indexes.push(compiled_through_index);
                                    }
                                }
                                result_union_types.insert(result_type);
                                Throughs::Tuple {
                                    nodes_indexes: result_throughs_nodes_indexes,
                                    nodes: compiled_throughs
                                        .into_iter()
                                        .map(|compiled_through| compiled_through.node)
                                        .collect(),
                                }
                            }
                            Type::Array(fold_array_element_type) => {
                                let mut through_compilation_context = compilation_context.clone();
                                through_compilation_context
                                    .path
                                    .0
                                    .extend([PathSegment::Through(0).into()]);
                                let starting_with_type = compiled_starting_with.node.r#type.clone();
                                self.define_constant(
                                    r#as.clone(),
                                    ConstantMetadata {
                                        r#type: *fold_array_element_type.clone(),
                                        is_computable: true,
                                    },
                                    &mut through_compilation_context,
                                    global_compilation_context,
                                );
                                self.define_constant(
                                    accumulating_in.clone(),
                                    ConstantMetadata {
                                        r#type: starting_with_type.clone(),
                                        is_computable: true,
                                    },
                                    &mut through_compilation_context,
                                    global_compilation_context,
                                );
                                let compiled_through = self.compile_with_context(
                                    through,
                                    &through_compilation_context,
                                    global_compilation_context,
                                )?;
                                if !compiled_fold.is_computable {
                                    return Err(anyhow!(
                                        "expected computable through, found {through:#?} at {:#?}",
                                        through_compilation_context.path
                                    ));
                                }
                                let compiled_through_resolved_type = resolve_type(
                                    &compiled_through.node.r#type,
                                    &starting_with_type,
                                    &through_compilation_context,
                                )?;
                                result_external_constants_name_clustered_indices
                                    .extend(compiled_through.external_constants_name_clustered_indices);
                                is_pure &= compiled_through.is_pure;
                                result_union_types.insert(compiled_through_resolved_type);
                                Throughs::Array(Box::new(compiled_through.node))
                            }
                            _ => {
                                return Err(anyhow!(
                                    "expected tuple or array, found {:#?} at {:#?}",
                                    fold_concrete_type,
                                    fold_compilation_context.path
                                ));
                            }
                        })
                    );
                }
                NodeAndMetadata {
                    node: Node {
                        content: Content::Fold {
                            fold: Box::new(compiled_fold.node),
                            fold_constant_name_clustered_index,
                            starting_with: Box::new(compiled_starting_with.node),
                            accumulating_in_constant_name_clustered_index,
                            fold_concrete_type_and_throughs,
                        },
                        r#type: Type::from(result_union_types),
                    },
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    is_pure,
                    is_computable: true,
                }
            }
            Program::Metaprogram { metaprogram } => {
                let mut metaprogram_compilation_context = compilation_context.clone();
                metaprogram_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Metaprogram.into()]);
                let compiled_metaprogram =
                    Arc::new(self.compile(metaprogram).with_context(|| {
                        format!(
                            "expected valid metaprogram at {:#?}",
                            metaprogram_compilation_context.path
                        )
                    })?);
                self.compile_with_context(
                    &serde_saphyr::from_str(&serde_saphyr::to_string(
                        &self
                            .metaprograms_computer
                            .compute(&compiled_metaprogram)
                            .with_context(|| {
                                format!(
                                    "expected to succesfully compute metaprogram at {:#?}",
                                    metaprogram_compilation_context.path
                                )
                            })?,
                    )?)?,
                    &metaprogram_compilation_context,
                    global_compilation_context,
                )?
            }
            Program::Sequence {
                starting_with,
                r#as,
                next,
            } => {
                let mut starting_with_compilation_context = compilation_context.clone();
                starting_with_compilation_context
                    .path
                    .0
                    .extend([PathSegment::StartingWith.into()]);
                let compiled_starting_with = self.compile_with_context(
                    starting_with,
                    &starting_with_compilation_context,
                    global_compilation_context,
                )?;
                let mut result_external_constants_name_clustered_indices =
                    compiled_starting_with.external_constants_name_clustered_indices;
                let mut next_compilation_context = compilation_context.clone();
                next_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Next.into()]);
                let current_constant_name_clustered_index = self
                    .define_constant(
                        r#as.clone(),
                        ConstantMetadata {
                            r#type: compiled_starting_with.node.r#type.clone(),
                            is_computable: compiled_starting_with.is_computable,
                        },
                        &mut next_compilation_context,
                        global_compilation_context,
                    )
                    .name_clustered_index;
                let mut compiled_next = self.compile_with_context(
                    next,
                    &next_compilation_context,
                    global_compilation_context,
                )?;
                result_external_constants_name_clustered_indices
                    .append(&mut compiled_next.external_constants_name_clustered_indices);
                resolve_type(
                    &compiled_next.node.r#type,
                    &compiled_starting_with.node.r#type,
                    &next_compilation_context,
                )?;
                let mut while_compilation_context = compilation_context.clone();
                while_compilation_context
                    .path
                    .0
                    .extend([PathSegment::While.into()]);
                self.define_constant(
                    r#as.clone(),
                    ConstantMetadata {
                        r#type: compiled_starting_with.node.r#type.clone(),
                        is_computable: compiled_starting_with.is_computable,
                    },
                    &mut while_compilation_context,
                    global_compilation_context,
                );
                let starting_with_type = compiled_starting_with.node.r#type.clone();
                NodeAndMetadata {
                    node: Node {
                        content: Content::Sequence(Arc::new(
                            intermediate_representation::Sequence {
                                starting_with: Box::new(compiled_starting_with.node),
                                current_constant_name_clustered_index,
                                next: Arc::new(compiled_next.node),
                            },
                        )),
                        r#type: Type::Array(Box::new(starting_with_type)),
                    },
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    is_pure: compiled_starting_with.is_pure & compiled_next.is_pure,
                    is_computable: compiled_starting_with.is_computable
                        & compiled_next.is_computable,
                }
            }
            Program::Object(object) => {
                match object.len() {
                    0 => {
                        return Ok(NodeAndMetadata {
                            external_constants_name_clustered_indices: BTreeSet::new(),
                            node: Node {
                                content: Content::Object(BTreeMap::new()),
                                r#type: Type::Object(BTreeMap::new()),
                            },
                            is_pure: true,
                            is_computable: true,
                        });
                    }
                    1 => {
                        let (function_name, function_argument) = object.iter().next().unwrap();
                        if function_name.ends_with(":") {
                            if let Some(function_body) =
                                compilation_context.available_functions.get(function_name)
                            {
                                let mut arguments_is_pure = true;
                                let mut arguments_is_computable = true;
                                let mut body_compilation_context = compilation_context.clone();
                                body_compilation_context.path.0.extend([
                                    PathSegment::UserFunctionCall(function_name.clone()).into(),
                                ]);
                                let arguments_iterator = match &**function_argument {
                                    Program::Object(function_arguments) => {
                                        let arguments_iterator: Box<
                                            dyn Iterator<Item = (Arc<String>, Arc<Program>)>,
                                        > =
                                            if function_arguments.len() > 1 {
                                                Box::new(function_arguments.iter().map(
                                                    |(key, value)| (key.clone(), value.clone()),
                                                ))
                                            } else {
                                                Box::new(
                                                    [(
                                                        DEFAULT_ARGUMENT_NAME.to_string().into(),
                                                        function_argument.clone(),
                                                    )]
                                                    .into_iter(),
                                                )
                                            };
                                        arguments_iterator
                                    }
                                    _ => Box::new(
                                        [(
                                            DEFAULT_ARGUMENT_NAME.to_string().into(),
                                            function_argument.clone(),
                                        )]
                                        .into_iter(),
                                    ),
                                };
                                let mut new_constants_definitions = Vec::new();
                                let mut result_external_constants_name_clustered_indices =
                                    BTreeSet::new();
                                for (function_argument_name, function_argument_body) in
                                    arguments_iterator
                                {
                                    if function_argument_name.ends_with(":") {
                                        body_compilation_context.available_functions.extend(
                                            [(function_argument_name, function_argument_body)]
                                                .into_iter(),
                                        );
                                    } else {
                                        let mut argument_compilation_context =
                                            compilation_context.clone();
                                        argument_compilation_context.path.0.extend([
                                            PathSegment::UserFunctionCall(function_name.clone())
                                                .into(),
                                            PathSegment::Argument(function_argument_name.clone())
                                                .into(),
                                        ]);
                                        let mut compiled_constant = self.compile_with_context(
                                            &function_argument_body,
                                            &argument_compilation_context,
                                            global_compilation_context,
                                        )?;
                                        result_external_constants_name_clustered_indices.append(
                                            &mut compiled_constant
                                                .external_constants_name_clustered_indices,
                                        );
                                        let constant_definition = self.define_constant(
                                            function_argument_name.clone(),
                                            ConstantMetadata {
                                                r#type: compiled_constant.node.r#type.clone(),
                                                is_computable: compiled_constant.is_computable,
                                            },
                                            &mut body_compilation_context,
                                            global_compilation_context,
                                        );
                                        new_constants_definitions.push(
                                            intermediate_representation::ConstantDefinition {
                                                name_clustered_index: constant_definition
                                                    .name_clustered_index,
                                                node: Arc::new(compiled_constant.node),
                                            },
                                        );
                                        arguments_is_pure &= compiled_constant.is_pure;
                                        arguments_is_computable &= compiled_constant.is_computable;
                                    }
                                }
                                let function_body_as_maybe_compiled_program =
                                    MaybeCompiledProgram::from(function_body);
                                if compilation_context
                                    .entered_user_functions
                                    .contains(function_body)
                                {
                                    let (function_index, function_type) =
                                        global_compilation_context
                                            .user_function_to_index_and_type_option
                                            .get(&function_body_as_maybe_compiled_program)
                                            .unwrap(); // in different contexts the same function return type may be not the same, here this would mean polymorphic recursion which is not supported
                                    return Ok(NodeAndMetadata {
                                        external_constants_name_clustered_indices: BTreeSet::new(),
                                        node: Node {
                                            content: Content::UserFunctionCall {
                                                arguments: new_constants_definitions,
                                                body: *function_index,
                                            },
                                            r#type: function_type.clone(),
                                        },
                                        is_pure: arguments_is_pure,
                                        is_computable: arguments_is_computable,
                                    });
                                } else {
                                    body_compilation_context
                                        .entered_user_functions
                                        .extend([function_body.clone()]);
                                    let function_index =
                                        global_compilation_context.user_functions_definitions.len();
                                    global_compilation_context.user_functions_definitions.push(
                                        UserFunctionCallDefinition {
                                            external_constants_name_clustered_indices: Vec::new(),
                                            body: function_body_as_maybe_compiled_program.clone(),
                                            is_pure: arguments_is_pure,
                                        },
                                    );
                                    global_compilation_context
                                        .user_function_to_index_and_type_option
                                        .insert(
                                            function_body_as_maybe_compiled_program.clone(),
                                            (function_index, Type::Unknown(MaybeType::default())),
                                        );
                                    let mut compiled_function = self.compile_with_context(
                                        function_body,
                                        &body_compilation_context,
                                        global_compilation_context,
                                    )?;
                                    global_compilation_context.user_functions_definitions
                                        [function_index] = UserFunctionCallDefinition {
                                        external_constants_name_clustered_indices: Vec::from_iter(
                                            compiled_function
                                                .external_constants_name_clustered_indices
                                                .iter()
                                                .cloned(),
                                        ),
                                        body: MaybeCompiledProgram {
                                            program: function_body_as_maybe_compiled_program
                                                .program,
                                            node: Some(compiled_function.node.clone()),
                                        },
                                        is_pure: compiled_function.is_pure,
                                    };
                                    result_external_constants_name_clustered_indices.append(
                                        &mut compiled_function
                                            .external_constants_name_clustered_indices,
                                    );
                                    return Ok(NodeAndMetadata {
                                        external_constants_name_clustered_indices:
                                            result_external_constants_name_clustered_indices.clone(),
                                        node: Node {
                                            content: Content::UserFunctionCall {
                                                arguments: new_constants_definitions,
                                                body: function_index,
                                            },
                                            r#type: compiled_function.node.r#type,
                                        },
                                        is_pure: arguments_is_pure && compiled_function.is_pure,
                                        is_computable: arguments_is_computable
                                            && compiled_function.is_computable,
                                    });
                                }
                            } else {
                                return Err(anyhow!(
                                    "expected one of available functions {:#?}, found function \
                                     {function_name:?} at {:#?}",
                                    compilation_context
                                        .available_functions
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
                let mut is_computable = true;
                for (object_key, object_value) in object.iter() {
                    let mut object_value_compilation_context = compilation_context.clone();
                    object_value_compilation_context
                        .path
                        .0
                        .extend([PathSegment::ObjectKey(object_key.clone()).into()]);
                    let mut compiled_object_value = self.compile_with_context(
                        object_value,
                        &object_value_compilation_context,
                        global_compilation_context,
                    )?;
                    result_external_constants_name_clustered_indices.append(
                        &mut compiled_object_value.external_constants_name_clustered_indices,
                    );
                    result_inner_types.insert(
                        object_key.clone(),
                        compiled_object_value.node.r#type.clone(),
                    );
                    result_content.insert(object_key.clone(), Arc::new(compiled_object_value.node));
                    is_pure &= compiled_object_value.is_pure;
                    is_computable &= compiled_object_value.is_computable;
                }
                NodeAndMetadata {
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::Object(result_content),
                        r#type: Type::Object(result_inner_types),
                    },
                    is_pure,
                    is_computable,
                }
            }
            Program::Value(value_option) => NodeAndMetadata {
                external_constants_name_clustered_indices: BTreeSet::new(),
                node: Node {
                    content: Content::Value(unsafe {
                        std::mem::transmute::<
                            Arc<Option<Value>>,
                            Arc<Option<intermediate_representation::Value>>,
                        >(value_option.clone())
                    }),
                    r#type: Value::r#type(value_option),
                },
                is_pure: true,
                is_computable: true,
            },
        })
    }
}
