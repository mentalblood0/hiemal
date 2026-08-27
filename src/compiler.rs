use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use anyhow::{Context, Error, Result, anyhow};
use enumset::EnumSet;
use gxhash::HashMap;
use regex::Regex;
use serde::Serialize;

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
        self, AtSegment, Condition, EmbeddedFunction, EmbeddedFunctionCall, Match, Path,
        PathSegment, Program, RangeBound,
    },
    r#type::{Capability, Constructed, MaybeType, Type, TypeAtResult, TypeKind, TypeProperties},
    value::Value,
};

#[derive(Clone, Default)]
struct CompilationContext {
    path: Path,
    available_functions: Object<String, Program>,
    available_constants: HashMap<Arc<String>, usize>,
    entered_user_functions: Set<Program>,
}

#[derive(Serialize)]
struct CompilationTypeError<'a, G, E>
where
    G: Serialize,
    E: Serialize,
{
    got: &'a G,
    expected: &'a E,
    r#at: &'a Path,
}

impl CompilationContext {
    fn error<'a, G, E>(&self, got: &'a G, expected: &'a E) -> Error
    where
        G: Serialize,
        E: Serialize,
    {
        anyhow!(
            "{}",
            serde_saphyr::to_string(&CompilationTypeError {
                got,
                expected,
                r#at: &self.path
            })
            .unwrap()
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
    expected_type_kind: &TypeKind,
    compilation_context: &CompilationContext,
) -> Result<Type> {
    if expected_type_kind == &TypeKind::Any || expected_type_kind.contains(&got_type.kind) {
        Ok(got_type.clone())
    } else {
        match (&got_type.kind, expected_type_kind) {
            (TypeKind::Unknown(unknown_type), expected_type_kind)
            | (expected_type_kind, TypeKind::Unknown(unknown_type)) => {
                let mut unknown_type_write_guard = unknown_type.lockable_internals.write();
                match &*unknown_type_write_guard {
                    Some(got_type) => {
                        resolve_type(got_type, expected_type_kind, compilation_context)
                    }
                    None => {
                        let result_type = Type {
                            kind: expected_type_kind.clone(),
                            properties: got_type.properties.clone(),
                        };
                        *unknown_type_write_guard = Some(result_type.clone());
                        Ok(result_type)
                    }
                }
            }
            (
                TypeKind::Constructed(got_constructed),
                TypeKind::Constructed(expected_constructed),
            ) => resolve_type(
                got_constructed.inner(),
                &expected_constructed.inner().kind,
                compilation_context,
            ),
            (TypeKind::Constructed(got_constructed), _) => resolve_type(
                got_constructed.inner(),
                expected_type_kind,
                compilation_context,
            ),
            (_, TypeKind::Constructed(expected_constructed)) => resolve_type(
                got_type,
                &expected_constructed.inner().kind,
                compilation_context,
            ),
            (TypeKind::Array(got_element_type), TypeKind::Array(expected_element_type)) => {
                let mut inner_compilation_context = compilation_context.clone();
                inner_compilation_context
                    .path
                    .0
                    .push(PathSegment::ArrayIndex(0));
                Ok(got_type.with_kind(TypeKind::Array(Box::new(resolve_type(
                    got_element_type,
                    &expected_element_type.kind,
                    &inner_compilation_context,
                )?))))
            }
            (TypeKind::Array(got_element_type), TypeKind::Tuple(expected_elements_types)) => {
                let mut result_union_types = BTreeSet::new();
                for expected_element_type in expected_elements_types.iter() {
                    result_union_types.insert(resolve_type(
                        got_element_type,
                        &expected_element_type.kind,
                        compilation_context,
                    )?);
                }
                Ok(got_type.with_kind(TypeKind::Array(Box::new(Type::from(result_union_types)))))
            }
            (TypeKind::Tuple(got_elements_types), TypeKind::Array(expected_element_type)) => {
                let mut result_tuple_types = Vec::with_capacity(got_elements_types.len());
                for got_element_type in got_elements_types.iter() {
                    result_tuple_types.push(resolve_type(
                        got_element_type,
                        &expected_element_type.kind,
                        compilation_context,
                    )?);
                }
                Ok(got_type.with_kind(TypeKind::Tuple(result_tuple_types.into())))
            }
            (TypeKind::Object(got_inner_types), TypeKind::Object(expected_inner_types)) => {
                let mut result_inner_types = BTreeMap::new();
                for (expected_value_key, expected_value_type) in expected_inner_types.iter() {
                    if let Some(got_value_type) = got_inner_types.get(expected_value_key) {
                        result_inner_types.insert(
                            expected_value_key.clone(),
                            resolve_type(
                                got_value_type,
                                &expected_value_type.kind,
                                compilation_context,
                            )?,
                        );
                    } else {
                        return Err(compilation_context.error(got_type, expected_type_kind));
                    }
                }
                Ok(got_type.with_kind(TypeKind::Object(result_inner_types.into())))
            }
            (TypeKind::Object(got_inner_types), TypeKind::GenericObject(expected_value_type)) => {
                let mut result_inner_types = BTreeMap::new();
                for (got_value_key, got_value_type) in got_inner_types.iter() {
                    result_inner_types.insert(
                        got_value_key.clone(),
                        resolve_type(
                            got_value_type,
                            &expected_value_type.kind,
                            compilation_context,
                        )?,
                    );
                }
                Ok(got_type.with_kind(TypeKind::Object(result_inner_types.into())))
            }
            (
                TypeKind::GenericObject(got_value_type),
                TypeKind::GenericObject(expected_value_type),
            ) => {
                let mut inner_compilation_context = compilation_context.clone();
                inner_compilation_context
                    .path
                    .0
                    .push(PathSegment::ArrayIndex(0));
                Ok(got_type.with_kind(TypeKind::Array(Box::new(resolve_type(
                    got_value_type,
                    &expected_value_type.kind,
                    &inner_compilation_context,
                )?))))
            }
            (TypeKind::Union(got_union_types), TypeKind::Union(expected_union_types)) => {
                let mut result_union_types = BTreeSet::new();
                if !got_union_types.is_subset(expected_union_types) {
                    for one_of_got_types in got_union_types.iter() {
                        let mut found = false;
                        for one_of_expected_types in expected_union_types.iter() {
                            if let Ok(result_union_type) = resolve_type(
                                one_of_got_types,
                                &one_of_expected_types.kind,
                                compilation_context,
                            ) {
                                result_union_types.insert(result_union_type);
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            return Err(compilation_context.error(got_type, expected_type_kind));
                        }
                    }
                }
                Ok(Type::from(result_union_types))
            }
            (TypeKind::Union(got_union_types), _) => {
                let mut result_union_types = BTreeSet::new();
                for one_of_got_types in got_union_types.iter() {
                    result_union_types.insert(resolve_type(
                        one_of_got_types,
                        expected_type_kind,
                        compilation_context,
                    )?);
                }
                Ok(got_type.with_kind(TypeKind::Union(result_union_types.into())))
            }
            (_, TypeKind::Union(expected_union_types)) => {
                if !expected_union_types.contains(&expected_type_kind.clone().into()) {
                    for one_of_expected_types in expected_union_types.iter() {
                        if let Ok(result_type) =
                            resolve_type(got_type, &one_of_expected_types.kind, compilation_context)
                        {
                            return Ok(result_type);
                        }
                    }
                    return Err(compilation_context.error(got_type, expected_type_kind));
                }
                Ok(got_type.clone())
            }
            (TypeKind::LiteralString(_), TypeKind::String)
            | (TypeKind::GenericLiteralString, TypeKind::String)
            | (TypeKind::LiteralString(_), TypeKind::GenericLiteralString) => Ok(got_type.clone()),
            _ => Err(compilation_context.error(got_type, expected_type_kind)),
        }
    }
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct NodeAndMetadata {
    node: Arc<Node>,
    external_constants_name_clustered_indices: BTreeSet<usize>,
}

#[derive(Clone, Debug, Eq, Default)]
struct MaybeCompiledProgram {
    program: Arc<Program>,
    node: Option<Arc<Node>>,
}

impl Hash for MaybeCompiledProgram {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.program.hash(state);
    }
}

impl PartialEq for MaybeCompiledProgram {
    fn eq(&self, other: &Self) -> bool {
        self.program == other.program
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
}

#[derive(Default, Clone, Hash, Debug)]
struct ConstantMetadata {
    r#type: Type,
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
                        result = tuple.get(*tuple_index).unwrap().clone();
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
                            allow: _,
                            forbid: _,
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
                            allow: _,
                            forbid: _,
                        },
                        PathSegment::Compute,
                    ) => {
                        result = compute.clone();
                    }
                    (
                        Program::FromAt {
                            from: inner_from,
                            at: inner_at,
                            default: _,
                        },
                        _,
                    ) => {
                        let result_and_inner_path_segment_index =
                            process_from_at_program_path_part(
                                inner_from,
                                inner_at,
                                includes_cache,
                            )?;
                        if result_and_inner_path_segment_index.1 != Some(inner_at.len()) {
                            return Err(anyhow!(
                                "Can not get program from {:#?} at {:#?}: stuck at path segment \
                                 {}: {current_path_segment:#?}",
                                from,
                                at,
                                current_path_segment_index + 1
                            ))
                            .context(
                                "expected only program segments path in inner from-at clause, got \
                                 {inner_at:#?}",
                            )?;
                        }
                        result = result_and_inner_path_segment_index.0;
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
        let compilation_context = CompilationContext::default();
        let compiled_program = self.compile_with_context(
            program,
            &compilation_context,
            &mut global_compilation_context,
        )?;
        if !compiled_program.node.r#type.properties.is_computable {
            Err(compilation_context.error(&"non-computible program", &"computible program"))
        } else {
            Ok(Arc::new(IntermediateRepresentation {
                root: compiled_program.node.clone(),
                user_functions: global_compilation_context
                    .user_functions_definitions
                    .into_iter()
                    .map(|user_function_definition| UserFunction {
                        external_constants_name_clustered_indices: user_function_definition
                            .external_constants_name_clustered_indices,
                        node: user_function_definition.body.node.unwrap().clone(),
                    })
                    .collect(),
                unique_constants_names_count: global_compilation_context
                    .constants_names_to_name_clustered_constants_indices
                    .len(),
            }))
        }
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
            .insert(name, result.index);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_embedded_function_call<F>(
        &self,
        compilation_context: &CompilationContext,
        global_compilation_context: &mut GlobalCompilationContext,
        embedded_function_call: &EmbeddedFunctionCall,
        argument_type_kind: &TypeKind,
        get_result_type_from_argument_resolved_type: &F,
    ) -> Result<Arc<NodeAndMetadata>>
    where
        F: Fn(&Type) -> Result<Type>,
    {
        let mut argument_compilation_context = compilation_context.clone();
        argument_compilation_context
            .path
            .0
            .extend([PathSegment::EmbeddedFunctionCall(
                embedded_function_call.embedded_function,
            )]);
        let compiled_argument = self.compile_with_context(
            &embedded_function_call.argument,
            &argument_compilation_context,
            global_compilation_context,
        )?;
        let compiled_argument_resolved_type = resolve_type(
            &compiled_argument.node.r#type,
            argument_type_kind,
            &argument_compilation_context,
        )?;
        if !compiled_argument_resolved_type.properties.is_computable {
            return Err(argument_compilation_context.error(
                &"non-computible embedded function argument",
                &"computible embedded function argument",
            ));
        }
        let result_type =
            get_result_type_from_argument_resolved_type(&compiled_argument_resolved_type)?;
        Ok(NodeAndMetadata {
            external_constants_name_clustered_indices: compiled_argument
                .external_constants_name_clustered_indices
                .clone(),
            node: Node {
                content: Content::EmbeddedFunctionCall {
                    path_option: if result_type
                        .properties
                        .capabilities
                        .contains(Capability::Error)
                    {
                        Some(argument_compilation_context.path.clone())
                    } else {
                        None
                    },
                    embedded_function_call: intermediate_representation::EmbeddedFunctionCall {
                        embedded_function: embedded_function_call.embedded_function,
                        argument: compiled_argument.node.clone(),
                    },
                },
                r#type: result_type,
            }
            .into(),
        }
        .into())
    }

    fn compile_with_context(
        &self,
        program: &Program,
        compilation_context: &CompilationContext,
        global_compilation_context: &mut GlobalCompilationContext,
    ) -> Result<Arc<NodeAndMetadata>> {
        Ok(match program {
            Program::BytesValue { bytes: hex_string } => NodeAndMetadata {
                external_constants_name_clustered_indices: BTreeSet::new(),
                node: Node {
                    content: Content::Value(
                        Some(intermediate_representation::Value::Bytes(
                            hex::decode(hex_string)
                                .with_context(|| {
                                    compilation_context.error(hex_string, &"hexadecimal string")
                                })?
                                .into(),
                        ))
                        .into(),
                    ),
                    r#type: TypeKind::Bytes.into(),
                }
                .into(),
            }
            .into(),
            Program::Tuple(tuple) => {
                if tuple.is_empty() {
                    return Ok(NodeAndMetadata {
                        external_constants_name_clustered_indices: BTreeSet::new(),
                        node: Node {
                            content: Content::Value(
                                Some(intermediate_representation::Value::Tuple(Vector::default()))
                                    .into(),
                            ),
                            r#type: TypeKind::Tuple(vec![].into()).into(),
                        }
                        .into(),
                    }
                    .into());
                }
                let mut result_content = Vec::with_capacity(tuple.len());
                let mut result_external_constants_name_clustered_indices = BTreeSet::new();
                let mut result_elements_types = Vec::with_capacity(tuple.len());
                for (element_index, element) in tuple.iter().enumerate() {
                    let mut element_compilation_context = compilation_context.clone();
                    element_compilation_context
                        .path
                        .0
                        .extend([PathSegment::ArrayIndex(element_index)]);
                    let compiled_element = self.compile_with_context(
                        element,
                        &element_compilation_context,
                        global_compilation_context,
                    )?;
                    result_elements_types.push(compiled_element.node.r#type.clone());
                    result_content.push(compiled_element.node.clone());
                    result_external_constants_name_clustered_indices.append(
                        &mut compiled_element
                            .external_constants_name_clustered_indices
                            .clone(),
                    );
                }
                NodeAndMetadata {
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::Tuple(result_content),
                        r#type: result_elements_types.into(),
                    }
                    .into(),
                }
                .into()
            }
            Program::Scope {
                functions,
                constants,
                compute,
                allow,
                forbid,
            } => {
                if allow.is_some() && forbid.is_some() {
                    return Err(compilation_context.error(
                        &BTreeMap::from_iter([("allow", &allow), ("forbid", &forbid)]),
                        &"either `allow` or `forbid`",
                    ));
                }
                let mut compute_compilation_context = compilation_context.clone();
                compute_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Compute]);
                let mut new_constants = Vec::with_capacity(constants.len());
                let mut result_external_constants_name_clustered_indices = BTreeSet::new();
                let mut constants_name_clustered_indices = Vec::with_capacity(constants.len());
                let mut result_type_properties = TypeProperties::default();
                for (constant_name, constant_compute_body) in constants.iter() {
                    let mut constant_compilation_context = compilation_context.clone();
                    constant_compilation_context.path.0.extend([
                        PathSegment::Constants,
                        PathSegment::Constant(constant_name.clone()),
                    ]);
                    let compiled_constant = self.compile_with_context(
                        constant_compute_body,
                        &constant_compilation_context,
                        global_compilation_context,
                    )?;
                    result_external_constants_name_clustered_indices.append(
                        &mut compiled_constant
                            .external_constants_name_clustered_indices
                            .clone(),
                    );
                    let constant_definition = self.define_constant(
                        constant_name.clone(),
                        ConstantMetadata {
                            r#type: compiled_constant.node.r#type.clone(),
                        },
                        &mut compute_compilation_context,
                        global_compilation_context,
                    );
                    constants_name_clustered_indices.push(constant_definition.name_clustered_index);
                    new_constants.push(intermediate_representation::ConstantDefinition {
                        name_clustered_index: constant_definition.name_clustered_index,
                        node: compiled_constant.node.clone(),
                    });
                }
                for (function_name, function_body) in functions.iter() {
                    if !function_name.ends_with(":") {
                        let mut function_compilation_context = compilation_context.clone();
                        function_compilation_context.path.0.extend([
                            PathSegment::Functions,
                            PathSegment::Function(function_name.clone()),
                        ]);
                        return Err(function_compilation_context.error(
                            &format!("function named {function_name:?}"),
                            &format!("function named {:?}", format!("{function_name}:")),
                        ));
                    }
                    compute_compilation_context
                        .available_functions
                        .insert(function_name.clone(), function_body.clone());
                }
                let compiled_compute = self.compile_with_context(
                    compute,
                    &compute_compilation_context,
                    global_compilation_context,
                )?;
                result_type_properties.unify(&compiled_compute.node.r#type.properties);
                if let Some(allow) = allow {
                    let used_not_allowed_capabilities =
                        compiled_compute.node.r#type.properties.capabilities - *allow;
                    if !used_not_allowed_capabilities.is_empty() {
                        return Err(compute_compilation_context
                            .error(&used_not_allowed_capabilities, allow));
                    }
                } else if let Some(forbid) = forbid {
                    let used_forbidden_capabilities =
                        compiled_compute.node.r#type.properties.capabilities & *forbid;
                    if !used_forbidden_capabilities.is_empty() {
                        return Err(compute_compilation_context.error(
                            &used_forbidden_capabilities,
                            &EnumSet::default().difference(*forbid),
                        ));
                    }
                }
                let mut compiled_compute_external_constants_name_clustered_indices =
                    compiled_compute
                        .external_constants_name_clustered_indices
                        .clone();
                for constant_name_clustered_index in constants_name_clustered_indices {
                    compiled_compute_external_constants_name_clustered_indices
                        .remove(&constant_name_clustered_index);
                }
                result_external_constants_name_clustered_indices
                    .append(&mut compiled_compute_external_constants_name_clustered_indices);
                let result_type_kind = compiled_compute.node.r#type.kind.clone();
                NodeAndMetadata {
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::Scope {
                            constants: new_constants,
                            compute: compiled_compute.node.clone(),
                        },
                        r#type: Type {
                            kind: result_type_kind,
                            properties: result_type_properties,
                        },
                    }
                    .into(),
                }
                .into()
            }
            Program::Constant {
                constant: constant_name,
            } => {
                if let Some(constant_index) =
                    compilation_context.available_constants.get(constant_name)
                {
                    let constant_metadata =
                        global_compilation_context.constants[*constant_index].clone();
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
                        }
                        .into(),
                    }
                    .into()
                } else {
                    return Err(compilation_context.error(
                        constant_name,
                        &BTreeMap::from_iter([(
                            &"one of available constants names",
                            &compilation_context
                                .available_functions
                                .keys()
                                .collect::<Vec<_>>(),
                        )]),
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
            Program::FromAt { from, at, default } => {
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
                let compiled_extracted_from = self.compile_with_context(
                    &extracted_from,
                    &from_program_compilation_context,
                    global_compilation_context,
                )?;
                let mut external_constants_name_clustered_indices = compiled_extracted_from
                    .external_constants_name_clustered_indices
                    .clone();
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
                                                let compiled_from_dynamic = self
                                                    .compile_with_context(
                                                        from_dynamic,
                                                        compilation_context,
                                                        global_compilation_context,
                                                    )?;
                                                resolve_type(
                                                    &compiled_from_dynamic.node.r#type,
                                                    &TypeKind::Number,
                                                    compilation_context,
                                                )?;
                                                external_constants_name_clustered_indices.append(
                                                    &mut compiled_from_dynamic
                                                        .external_constants_name_clustered_indices
                                                        .clone(),
                                                );
                                                intermediate_representation::RangeBound::Dynamic(
                                                    compiled_from_dynamic.node.clone(),
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
                                                let compiled_to_dynamic = self
                                                    .compile_with_context(
                                                        to_dynamic,
                                                        compilation_context,
                                                        global_compilation_context,
                                                    )?;
                                                resolve_type(
                                                    &compiled_to_dynamic.node.r#type,
                                                    &TypeKind::Number,
                                                    compilation_context,
                                                )?;
                                                external_constants_name_clustered_indices.append(
                                                    &mut compiled_to_dynamic
                                                        .external_constants_name_clustered_indices
                                                        .clone(),
                                                );
                                                intermediate_representation::RangeBound::Dynamic(
                                                    compiled_to_dynamic.node.clone(),
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
                let mut default_compilation_context = compilation_context.clone();
                default_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Default]);
                let compiled_default = self.compile_with_context(
                    default,
                    &default_compilation_context,
                    global_compilation_context,
                )?;
                let (compiled_extracted_from_type_at_result, runtime_type_error_is_possible) =
                    compiled_extracted_from
                        .node
                        .r#type
                        .clone()
                        .at_path(&value_path)
                        .with_context(|| {
                            compilation_context.error(
                                &compiled_extracted_from.node.r#type.kind,
                                &BTreeMap::from_iter([("value with path", &value_path)]),
                            )
                        })?;
                let compiled_extracted_from_type_at_result_as_type =
                    match compiled_extracted_from_type_at_result {
                        TypeAtResult::Single(r#type) => r#type,
                        TypeAtResult::Multiple(union_types) => union_types.into(),
                    };
                let r#type = if runtime_type_error_is_possible {
                    Type::from(BTreeSet::from_iter([
                        compiled_extracted_from_type_at_result_as_type,
                        compiled_default.node.r#type.clone(),
                    ]))
                } else {
                    compiled_extracted_from_type_at_result_as_type
                };
                NodeAndMetadata {
                    node: Node {
                        content: Content::FromAt {
                            from: compiled_extracted_from.node.clone(),
                            value_path_segments: value_path,
                            default: compiled_default.node.clone(),
                        },
                        r#type,
                    }
                    .into(),
                    external_constants_name_clustered_indices,
                }
                .into()
            }
            Program::EmbeddedFunctionCall(embedded_function_call) => {
                match embedded_function_call.embedded_function {
                    EmbeddedFunction::Sum => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::Array(Box::new(TypeKind::Number.into())),
                        &|compiled_argument_resolved_type| {
                            Ok(compiled_argument_resolved_type.with_kind(TypeKind::Number))
                        },
                    )?,
                    EmbeddedFunction::Mod => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::Tuple(
                            vec![TypeKind::Number.into(), TypeKind::Number.into()].into(),
                        ),
                        &|compiled_argument_resolved_type| {
                            Ok(Type {
                                kind: TypeKind::Number,
                                properties: TypeProperties {
                                    capabilities: Capability::Error.into(),
                                    is_computable: true,
                                }
                                .unified(&compiled_argument_resolved_type.properties),
                            })
                        },
                    )?,
                    EmbeddedFunction::Concat => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::Array(Box::new(TypeKind::String.into())),
                        &|compiled_argument_resolved_type| {
                            Ok(compiled_argument_resolved_type.with_kind(TypeKind::String))
                        },
                    )?,
                    EmbeddedFunction::IsSorted => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::Array(Box::new(TypeKind::Any.into())),
                        &|compiled_argument_resolved_type| {
                            Ok(compiled_argument_resolved_type.with_kind(TypeKind::Bool))
                        },
                    )?,
                    EmbeddedFunction::ReadBytesFromStandardInput => self
                        .compile_embedded_function_call(
                            compilation_context,
                            global_compilation_context,
                            embedded_function_call,
                            &TypeKind::Union(
                                BTreeSet::from_iter([
                                    TypeKind::Number.into(),
                                    TypeKind::LiteralString("all".into()).into(),
                                ])
                                .into(),
                            ),
                            &|compiled_argument_resolved_type| {
                                Ok(Type {
                                    kind: TypeKind::Bytes,
                                    properties: TypeProperties {
                                        capabilities: Capability::ReadStandardInput
                                            | Capability::Error,
                                        is_computable: true,
                                    }
                                    .unified(&compiled_argument_resolved_type.properties),
                                })
                            },
                        )?,
                    EmbeddedFunction::ParseYaml => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::String,
                        &|compiled_argument_resolved_type| {
                            Ok(Type {
                                kind: TypeKind::Any,
                                properties: TypeProperties {
                                    capabilities: Capability::Error.into(),
                                    is_computable: true,
                                }
                                .unified(&compiled_argument_resolved_type.properties),
                            })
                        },
                    )?,
                    EmbeddedFunction::KeyValuePairs => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::GenericObject(Box::new(TypeKind::Any.into())),
                        &|compiled_argument_resolved_type| {
                            if let TypeKind::Object(argument_object_values_types) =
                                &compiled_argument_resolved_type.kind
                            {
                                Ok(compiled_argument_resolved_type.with_kind(TypeKind::Tuple(
                                    argument_object_values_types
                                        .values()
                                        .map(|value| {
                                            TypeKind::Tuple(
                                                vec![TypeKind::String.into(), value.clone()].into(),
                                            )
                                            .into()
                                        })
                                        .collect::<Vec<_>>()
                                        .into(),
                                )))
                            } else {
                                Err(compilation_context
                                    .error(compiled_argument_resolved_type, &"object"))
                            }
                        },
                    )?,
                    EmbeddedFunction::MatchRegex => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::Object(
                            BTreeMap::from_iter([
                                ("string".to_string().into(), TypeKind::String.into()),
                                (
                                    "regex".to_string().into(),
                                    TypeKind::Constructed(Constructed::Regex).into(),
                                ),
                            ])
                            .into(),
                        ),
                        &|compiled_argument_resolved_type| {
                            if let TypeKind::LiteralString(regex_literal_string) =
                                &compiled_argument_resolved_type.kind
                            {
                                Regex::new(&regex_literal_string.to_string()).with_context(
                                    || {
                                        compilation_context.error(
                                            &regex_literal_string.to_string(),
                                            &"correct regex",
                                        )
                                    },
                                )?;
                            };
                            Ok(compiled_argument_resolved_type.with_kind(TypeKind::Union(
                                BTreeSet::from_iter([
                                    compiled_argument_resolved_type.with_kind(TypeKind::Object(
                                        BTreeMap::from_iter([
                                            (
                                                "groups".to_string().into(),
                                                compiled_argument_resolved_type.with_kind(
                                                    TypeKind::GenericObject(Box::new(
                                                        compiled_argument_resolved_type.with_kind(
                                                            TypeKind::Union(
                                                                BTreeSet::from_iter(vec![
                                                                    compiled_argument_resolved_type
                                                                        .with_kind(
                                                                            TypeKind::String,
                                                                        ),
                                                                    compiled_argument_resolved_type
                                                                        .with_kind(
                                                                            TypeKind::Number,
                                                                        ),
                                                                ])
                                                                .into(),
                                                            ),
                                                        ),
                                                    )),
                                                ),
                                            ),
                                            (
                                                "start".to_string().into(),
                                                compiled_argument_resolved_type
                                                    .with_kind(TypeKind::Number),
                                            ),
                                            (
                                                "end".to_string().into(),
                                                compiled_argument_resolved_type
                                                    .with_kind(TypeKind::Number),
                                            ),
                                        ])
                                        .into(),
                                    )),
                                    compiled_argument_resolved_type.with_kind(TypeKind::Null),
                                ])
                                .into(),
                            )))
                        },
                    )?,
                    EmbeddedFunction::ReadBytesFromFile => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::String,
                        &|compiled_argument_resolved_type| {
                            Ok(Type {
                                kind: TypeKind::Bytes,
                                properties: TypeProperties {
                                    capabilities: Capability::ReadFile | Capability::Error,
                                    is_computable: true,
                                }
                                .unified(&compiled_argument_resolved_type.properties),
                            })
                        },
                    )?,
                    EmbeddedFunction::StringFromBytes => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::Bytes,
                        &|compiled_argument_resolved_type| {
                            Ok(Type {
                                kind: TypeKind::String,
                                properties: TypeProperties {
                                    capabilities: Capability::Error.into(),
                                    is_computable: true,
                                }
                                .unified(&compiled_argument_resolved_type.properties),
                            })
                        },
                    )?,
                    EmbeddedFunction::CreateFile => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::Object(
                            BTreeMap::from_iter([
                                (
                                    "content".to_string().into(),
                                    TypeKind::Union(
                                        BTreeSet::from_iter([
                                            TypeKind::Bytes.into(),
                                            TypeKind::String.into(),
                                        ])
                                        .into(),
                                    )
                                    .into(),
                                ),
                                ("path".to_string().into(), TypeKind::String.into()),
                            ])
                            .into(),
                        ),
                        &|compiled_argument_resolved_type| {
                            Ok(Type {
                                kind: TypeKind::Null,
                                properties: TypeProperties {
                                    capabilities: Capability::CreateFile | Capability::Error,
                                    is_computable: true,
                                }
                                .unified(&compiled_argument_resolved_type.properties),
                            })
                        },
                    )?,
                    EmbeddedFunction::OverwriteFile => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::Object(
                            BTreeMap::from_iter([
                                (
                                    "content".to_string().into(),
                                    TypeKind::Union(
                                        BTreeSet::from_iter([
                                            TypeKind::Bytes.into(),
                                            TypeKind::String.into(),
                                        ])
                                        .into(),
                                    )
                                    .into(),
                                ),
                                ("path".to_string().into(), TypeKind::String.into()),
                            ])
                            .into(),
                        ),
                        &|compiled_argument_resolved_type| {
                            Ok(Type {
                                kind: TypeKind::Null,
                                properties: TypeProperties {
                                    capabilities: Capability::OverwriteFile | Capability::Error,
                                    is_computable: true,
                                }
                                .unified(&compiled_argument_resolved_type.properties),
                            })
                        },
                    )?,
                    EmbeddedFunction::RemoveFile => self.compile_embedded_function_call(
                        compilation_context,
                        global_compilation_context,
                        embedded_function_call,
                        &TypeKind::String,
                        &|compiled_argument_resolved_type| {
                            Ok(Type {
                                kind: TypeKind::Null,
                                properties: TypeProperties {
                                    capabilities: Capability::RemoveFile | Capability::Error,
                                    is_computable: true,
                                }
                                .unified(&compiled_argument_resolved_type.properties),
                            })
                        },
                    )?,
                }
            }
            Program::Match {
                r#match,
                r#as,
                cases,
            } => {
                let (r#match, match_constant_name) = match r#match {
                    Match::Constant(constant_name) => (
                        &Program::Constant {
                            constant: constant_name.clone(),
                        },
                        Some(constant_name),
                    ),
                    Match::Program(program) => (&**program, None),
                };
                let mut match_compilation_context = compilation_context.clone();
                match_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Match]);
                let compiled_match = self.compile_with_context(
                    r#match,
                    &match_compilation_context,
                    global_compilation_context,
                )?;
                if !compiled_match.node.r#type.kind.is_known() {
                    return Err(match_compilation_context
                        .error(&compiled_match.node.r#type, &"match of known type"));
                }
                let mut result_cases = Vec::new();
                let mut result_types = BTreeSet::new();
                let mut result_external_constants_name_clustered_indices = compiled_match
                    .external_constants_name_clustered_indices
                    .clone();
                let mut current_match_type_kind_option =
                    Some(compiled_match.node.r#type.kind.clone());
                let mut covered_types_kinds = BTreeSet::new();
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
                let mut has_effective_values_conditions = false;
                for (case_index, (case_condition, case)) in cases.iter().enumerate() {
                    let mut case_compilation_context = compilation_context.clone();
                    case_compilation_context
                        .path
                        .0
                        .extend([PathSegment::Cases, PathSegment::Case(case_index)]);
                    if let Some(ref current_match_type_kind) = current_match_type_kind_option {
                        match case_condition {
                            Condition::Type(case_match_type) => {
                                if let Some(refined_match_type_kind) =
                                    current_match_type_kind.intersection(&case_match_type.kind)
                                {
                                    if let Some(match_constant_name) =
                                        r#as.as_ref().or(match_constant_name)
                                    {
                                        self.define_constant(
                                            match_constant_name.clone(),
                                            ConstantMetadata {
                                                r#type: compiled_match
                                                    .node
                                                    .r#type
                                                    .with_kind(refined_match_type_kind),
                                            },
                                            &mut case_compilation_context,
                                            global_compilation_context,
                                        );
                                    };
                                    let compiled_case = self.compile_with_context(
                                        case,
                                        &case_compilation_context,
                                        global_compilation_context,
                                    )?;
                                    result_types.insert(compiled_case.node.r#type.clone());
                                    result_external_constants_name_clustered_indices.append(
                                        &mut compiled_case
                                            .external_constants_name_clustered_indices
                                            .clone(),
                                    );
                                    covered_types_kinds.insert(case_match_type.kind.clone());
                                    result_cases.push(Case {
                                        condition: intermediate_representation::Condition::Type(
                                            case_match_type.clone(),
                                        ),
                                        node: compiled_case.node.clone(),
                                    });
                                    current_match_type_kind_option =
                                        current_match_type_kind.substraction(&case_match_type.kind);
                                    if current_match_type_kind_option.is_none() {
                                        break;
                                    }
                                }
                            }
                            Condition::Value(condition) => {
                                let compiled_condition = self.compile_with_context(
                                    condition,
                                    &case_compilation_context,
                                    global_compilation_context,
                                )?;
                                if let Some(refined_match_type_kind) = compiled_match
                                    .node
                                    .r#type
                                    .kind
                                    .intersection(&compiled_condition.node.r#type.kind)
                                {
                                    if let Some(match_constant_name) =
                                        r#as.as_ref().or(match_constant_name)
                                    {
                                        self.define_constant(
                                            match_constant_name.clone(),
                                            ConstantMetadata {
                                                r#type: refined_match_type_kind.clone().into(),
                                            },
                                            &mut case_compilation_context,
                                            global_compilation_context,
                                        );
                                    }
                                    let compiled_case = self.compile_with_context(
                                        case,
                                        &case_compilation_context,
                                        global_compilation_context,
                                    )?;
                                    result_types.insert(compiled_case.node.r#type.clone());
                                    result_external_constants_name_clustered_indices.append(
                                        &mut compiled_case
                                            .external_constants_name_clustered_indices
                                            .clone(),
                                    );
                                    result_cases.push(Case {
                                        condition: intermediate_representation::Condition::Value(
                                            compiled_condition.node.clone(),
                                        ),
                                        node: compiled_case.node.clone(),
                                    });
                                    if matches!(
                                        refined_match_type_kind,
                                        TypeKind::LiteralTrue
                                            | TypeKind::LiteralFalse
                                            | TypeKind::LiteralString(_)
                                            | TypeKind::Null
                                    ) {
                                        current_match_type_kind_option = current_match_type_kind
                                            .substraction(&refined_match_type_kind);
                                        covered_types_kinds.insert(refined_match_type_kind);
                                        if current_match_type_kind_option.is_none() {
                                            break;
                                        }
                                        if !compiled_condition.node.r#type.properties.is_computable
                                        {
                                            return Err(case_compilation_context.error(
                                                &"non-computible case condition value",
                                                &"computible case condition value",
                                            ));
                                        }
                                        has_effective_values_conditions = true;
                                    }
                                }
                            }
                        }
                    }
                }
                if has_effective_values_conditions
                    && !compiled_match.node.r#type.properties.is_computable
                {
                    return Err(match_compilation_context
                        .error(&"non-computable match", &"computable match"));
                }
                let covered = Type::from(
                    covered_types_kinds
                        .iter()
                        .map(|kind| kind.clone().into())
                        .collect::<BTreeSet<_>>(),
                );
                if !covered.kind.contains(&compiled_match.node.r#type.kind) {
                    return Err(compilation_context.error(&covered, &compiled_match.node.r#type));
                }
                match result_cases.len() {
                    0 => {
                        return Err(match_compilation_context.error(
                            &Option::<String>::None,
                            &BTreeMap::from_iter([(
                                "at least one valid case for match",
                                &compiled_match.node.r#type,
                            )]),
                        ));
                    }
                    _ => NodeAndMetadata {
                        node: Node {
                            content: Content::Match {
                                r#match: compiled_match.node.clone(),
                                cases: result_cases,
                                match_constant_name_clustered_index_option,
                            },
                            r#type: Type::from(result_types),
                        }
                        .into(),
                        external_constants_name_clustered_indices:
                            result_external_constants_name_clustered_indices,
                    }
                    .into(),
                }
            }
            Program::Map { map, r#as, through } => {
                let mut map_compilation_context = compilation_context.clone();
                map_compilation_context.path.0.extend([PathSegment::Map]);
                let compiled_map = self.compile_with_context(
                    map,
                    &map_compilation_context,
                    global_compilation_context,
                )?;
                let mut result_external_constants_name_clustered_indices = compiled_map
                    .external_constants_name_clustered_indices
                    .clone();
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
                        match &map_concrete_type.kind {
                            TypeKind::Tuple(map_tuple_elements_types) => {
                                let mut result_elements_types =
                                    Vec::with_capacity(map_tuple_elements_types.len());
                                let mut result_throughs_nodes_indexes =
                                    Vec::with_capacity(map_tuple_elements_types.len());
                                let mut compiled_throughs: indexmap::IndexSet<
                                    Arc<NodeAndMetadata>,
                                > = indexmap::IndexSet::new();
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
                                        through_compilation_context
                                            .path
                                            .0
                                            .extend([PathSegment::Through(element_type_index)]);
                                        self.define_constant(
                                            r#as.clone(),
                                            ConstantMetadata {
                                                r#type: element_type.clone(),
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
                                                compiled_throughs.insert(compiled_through);
                                                compiled_throughs.len() - 1
                                            };
                                        element_type_to_compiled_through_index
                                            .insert(element_type.clone(), compiled_through_index);
                                        result_throughs_nodes_indexes.push(compiled_through_index);
                                    }
                                }
                                result_union_types.insert(Type::from(result_elements_types));
                                Throughs::Tuple {
                                    nodes_indexes: result_throughs_nodes_indexes,
                                    nodes: compiled_throughs
                                        .into_iter()
                                        .map(|compiled_through| compiled_through.node.clone())
                                        .collect(),
                                }
                            }
                            TypeKind::Array(map_array_element_type) => {
                                let mut through_compilation_context = compilation_context.clone();
                                through_compilation_context
                                    .path
                                    .0
                                    .extend([PathSegment::Through(0)]);
                                self.define_constant(
                                    r#as.clone(),
                                    ConstantMetadata {
                                        r#type: *map_array_element_type.clone(),
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
                                    compiled_through
                                        .external_constants_name_clustered_indices
                                        .clone(),
                                );
                                result_union_types.insert(map_concrete_type.with_kind(
                                    TypeKind::Array(Box::new(compiled_through.node.r#type.clone())),
                                ));
                                Throughs::Array(compiled_through.node.clone())
                            }
                            _ => {
                                return Err(map_compilation_context
                                    .error(map_concrete_type, &"tuple or array"));
                            }
                        },
                    ));
                }
                NodeAndMetadata {
                    node: Node {
                        content: Content::Map(Arc::new(intermediate_representation::Map {
                            map: compiled_map.node.clone(),
                            map_concrete_type_and_throughs,
                            map_constant_name_clustered_index,
                        })),
                        r#type: Type::from(result_union_types)
                            .with_unified_properties_from(&compiled_map.node.r#type),
                    }
                    .into(),
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                }
                .into()
            }
            Program::Flatten { flatten } => {
                let mut flatten_compilation_context = compilation_context.clone();
                flatten_compilation_context
                    .path
                    .0
                    .push(PathSegment::Flatten);
                let compiled_flatten = self.compile_with_context(
                    flatten,
                    &flatten_compilation_context,
                    global_compilation_context,
                )?;
                let flatten_resolved_type = resolve_type(
                    &compiled_flatten.node.r#type,
                    &TypeKind::Array(Box::new(
                        TypeKind::Array(Box::new(TypeKind::Any.into())).into(),
                    )),
                    compilation_context,
                )?;
                NodeAndMetadata {
                    node: Node {
                        content: Content::Flatten(compiled_flatten.node.clone()),
                        r#type: flatten_resolved_type.flatten()?,
                    }
                    .into(),
                    external_constants_name_clustered_indices: compiled_flatten
                        .external_constants_name_clustered_indices
                        .clone(),
                }
                .into()
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
                    .extend([PathSegment::Filter]);
                let compiled_filter = self.compile_with_context(
                    filter,
                    &filter_compilation_context,
                    global_compilation_context,
                )?;
                let mut result_external_constants_name_clustered_indices = compiled_filter
                    .external_constants_name_clustered_indices
                    .clone();
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
                        match &filter_concrete_type.kind {
                            TypeKind::Tuple(filter_tuple_elements_types) => {
                                let mut result_throughs_nodes_indexes =
                                    Vec::with_capacity(filter_tuple_elements_types.len());
                                let mut compiled_throughs: indexmap::IndexSet<
                                    Arc<NodeAndMetadata>,
                                > = indexmap::IndexSet::new();
                                for (element_type_index, element_type) in
                                    filter_tuple_elements_types.iter().enumerate()
                                {
                                    let mut through_compilation_context =
                                        compilation_context.clone();
                                    through_compilation_context
                                        .path
                                        .0
                                        .extend([PathSegment::Through(element_type_index)]);
                                    self.define_constant(
                                        r#as.clone(),
                                        ConstantMetadata {
                                            r#type: element_type.clone(),
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
                                        &TypeKind::Bool,
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
                                            compiled_throughs.insert(compiled_through);
                                            compiled_throughs.len() - 1
                                        };
                                    result_throughs_nodes_indexes.push(compiled_through_index);
                                }
                                result_union_types.insert(filter_concrete_type.with_kind(
                                    TypeKind::Array(Box::new(Type::from(BTreeSet::from_iter(
                                        filter_tuple_elements_types.iter().cloned(),
                                    )))),
                                ));
                                Throughs::Tuple {
                                    nodes_indexes: result_throughs_nodes_indexes,
                                    nodes: compiled_throughs
                                        .into_iter()
                                        .map(|compiled_through| compiled_through.node.clone())
                                        .collect(),
                                }
                            }
                            TypeKind::Array(filter_array_element_type) => {
                                let mut through_compilation_context = compilation_context.clone();
                                through_compilation_context
                                    .path
                                    .0
                                    .extend([PathSegment::Through(0)]);
                                self.define_constant(
                                    r#as.clone(),
                                    ConstantMetadata {
                                        r#type: *filter_array_element_type.clone(),
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
                                    &TypeKind::Bool,
                                    compilation_context,
                                )?;
                                result_external_constants_name_clustered_indices.extend(
                                    compiled_through
                                        .external_constants_name_clustered_indices
                                        .clone(),
                                );
                                result_union_types.insert(filter_concrete_type.clone());
                                Throughs::Array(compiled_through.node.clone())
                            }
                            _ => {
                                return Err(filter_compilation_context
                                    .error(filter_concrete_type, &"tuple or array"));
                            }
                        },
                    ));
                }
                NodeAndMetadata {
                    node: Node {
                        content: Content::Filter(Arc::new(intermediate_representation::Filter {
                            filter: compiled_filter.node.clone(),
                            filter_concrete_type_and_throughs,
                            filter_constant_name_clustered_index,
                        })),
                        r#type: Type::from(result_union_types)
                            .with_unified_properties_from(&compiled_filter.node.r#type),
                    }
                    .into(),
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                }
                .into()
            }
            Program::Fold {
                fold,
                r#as,
                starting_with,
                accumulating_in,
                through,
            } => {
                let mut fold_compilation_context = compilation_context.clone();
                fold_compilation_context.path.0.extend([PathSegment::Fold]);
                let compiled_fold = self.compile_with_context(
                    fold,
                    &fold_compilation_context,
                    global_compilation_context,
                )?;
                let mut result_external_constants_name_clustered_indices = compiled_fold
                    .external_constants_name_clustered_indices
                    .clone();
                let mut starting_with_compilation_context = compilation_context.clone();
                starting_with_compilation_context
                    .path
                    .0
                    .extend([PathSegment::StartingWith]);
                let compiled_starting_with = self.compile_with_context(
                    starting_with,
                    &starting_with_compilation_context,
                    global_compilation_context,
                )?;
                result_external_constants_name_clustered_indices.extend(
                    compiled_starting_with
                        .external_constants_name_clustered_indices
                        .clone(),
                );
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
                        match &fold_concrete_type.kind {
                            TypeKind::Tuple(fold_tuple_elements_types) => {
                                let mut result_type = compiled_starting_with.node.r#type.clone();
                                let mut result_throughs_nodes_indexes =
                                    Vec::with_capacity(fold_tuple_elements_types.len());
                                let mut compiled_throughs: indexmap::IndexSet<Arc<NodeAndMetadata>> =
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
                                            .extend([PathSegment::Through(current_type_index)]);
                                        self.define_constant(
                                            r#as.clone(),
                                            ConstantMetadata {
                                                r#type: current_type.clone(),
                                            },
                                            &mut through_compilation_context,
                                            global_compilation_context,
                                        );
                                        self.define_constant(
                                            accumulating_in.clone(),
                                            ConstantMetadata {
                                                r#type: result_type.clone(),
                                            },
                                            &mut through_compilation_context,
                                            global_compilation_context,
                                        );
                                        let compiled_through = self.compile_with_context(
                                            through,
                                            &through_compilation_context,
                                            global_compilation_context,
                                        )?;
                                        result_type = compiled_through.node.r#type.clone();
                                        let compiled_through_index = if let Some(compiled_through_index) =
                                            compiled_throughs.get_index_of(&compiled_through)
                                        {
                                            compiled_through_index
                                        } else {
                                            if (current_type_index == fold_tuple_elements_types.len() - 1) && !compiled_through.node.r#type.properties.is_computable {
                                                return Err(through_compilation_context.error(&"non-computible through", &"computible through"));
                                            }
                                            result_external_constants_name_clustered_indices.extend(
                                                compiled_through
                                                    .external_constants_name_clustered_indices
                                                    .clone(),
                                            );
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
                                        .map(|compiled_through| compiled_through.node.clone())
                                        .collect(),
                                }
                            }
                            TypeKind::Array(fold_array_element_type) => {
                                let mut through_compilation_context = compilation_context.clone();
                                through_compilation_context
                                    .path
                                    .0
                                    .extend([PathSegment::Through(0)]);
                                let starting_with_type = compiled_starting_with.node.r#type.clone();
                                self.define_constant(
                                    r#as.clone(),
                                    ConstantMetadata {
                                        r#type: *fold_array_element_type.clone(),
                                    },
                                    &mut through_compilation_context,
                                    global_compilation_context,
                                );
                                self.define_constant(
                                    accumulating_in.clone(),
                                    ConstantMetadata {
                                        r#type: starting_with_type.clone(),
                                    },
                                    &mut through_compilation_context,
                                    global_compilation_context,
                                );
                                let compiled_through = self.compile_with_context(
                                    through,
                                    &through_compilation_context,
                                    global_compilation_context,
                                )?;
                                let compiled_through_resolved_type = resolve_type(
                                    &compiled_through.node.r#type,
                                    &starting_with_type.kind,
                                    &through_compilation_context,
                                )?;
                                result_external_constants_name_clustered_indices
                                    .extend(compiled_through.external_constants_name_clustered_indices.clone());
                                result_union_types.insert(compiled_through_resolved_type);
                                Throughs::Array(compiled_through.node.clone())
                            }
                            _ => {
                                return Err(fold_compilation_context.error(
                                    fold_concrete_type,
                                    &"tuple or array"
                                ));
                            }
                        })
                    );
                }
                NodeAndMetadata {
                    node: Node {
                        content: Content::Fold {
                            fold: compiled_fold.node.clone(),
                            fold_constant_name_clustered_index,
                            starting_with: compiled_starting_with.node.clone(),
                            accumulating_in_constant_name_clustered_index,
                            fold_concrete_type_and_throughs,
                        },
                        r#type: Type::from(result_union_types)
                            .with_unified_properties_from(&compiled_fold.node.r#type)
                            .with_unified_properties_from(&compiled_starting_with.node.r#type),
                    }
                    .into(),
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                }
                .into()
            }
            Program::Metaprogram { metaprogram } => {
                let mut metaprogram_compilation_context = compilation_context.clone();
                metaprogram_compilation_context
                    .path
                    .0
                    .extend([PathSegment::Metaprogram]);
                let compiled_metaprogram =
                    Arc::new(self.compile(metaprogram).with_context(|| {
                        metaprogram_compilation_context.error(&"", &"valid metaprogram")
                    })?);
                self.compile_with_context(
                    &serde_saphyr::from_str(&serde_saphyr::to_string(
                        &self
                            .metaprograms_computer
                            .compute(&compiled_metaprogram)
                            .with_context(|| {
                                metaprogram_compilation_context
                                    .error(&"", &"computable metaprogram")
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
                r#while,
            } => {
                let mut starting_with_compilation_context = compilation_context.clone();
                starting_with_compilation_context
                    .path
                    .0
                    .extend([PathSegment::StartingWith]);
                let compiled_starting_with = self.compile_with_context(
                    starting_with,
                    &starting_with_compilation_context,
                    global_compilation_context,
                )?;
                let mut result_external_constants_name_clustered_indices = compiled_starting_with
                    .external_constants_name_clustered_indices
                    .clone();
                let mut current_concrete_type = compiled_starting_with.node.r#type.clone();
                let mut current_concrete_type_to_next = BTreeMap::new();
                let mut current_constant_name_clustered_index;
                loop {
                    let mut next_compilation_context = compilation_context.clone();
                    next_compilation_context.path.0.extend([PathSegment::Next(
                        compiled_starting_with.node.r#type.clone(),
                    )]);
                    current_constant_name_clustered_index = self
                        .define_constant(
                            r#as.clone(),
                            ConstantMetadata {
                                r#type: current_concrete_type.clone(),
                            },
                            &mut next_compilation_context,
                            global_compilation_context,
                        )
                        .name_clustered_index;
                    let compiled_next = self.compile_with_context(
                        next,
                        &next_compilation_context,
                        global_compilation_context,
                    )?;
                    if current_concrete_type_to_next
                        .insert(
                            std::mem::take(&mut current_concrete_type),
                            compiled_next.node.clone(),
                        )
                        .is_some()
                    {
                        break;
                    }
                    current_concrete_type = compiled_next.node.r#type.clone();
                    result_external_constants_name_clustered_indices.append(
                        &mut compiled_next
                            .external_constants_name_clustered_indices
                            .clone(),
                    );
                }
                let element_type = Type::from(BTreeSet::from_iter(
                    [compiled_starting_with.node.r#type.clone()]
                        .into_iter()
                        .chain(current_concrete_type_to_next.keys().cloned()),
                ));
                let compiled_while_node = if let Some(r#while) = r#while {
                    let mut while_compilation_context = compilation_context.clone();
                    while_compilation_context.path.0.push(PathSegment::While);
                    self.define_constant(
                        r#as.clone(),
                        ConstantMetadata {
                            r#type: element_type.clone(),
                        },
                        &mut while_compilation_context,
                        global_compilation_context,
                    );
                    let compiled_while = self.compile_with_context(
                        r#while,
                        &while_compilation_context,
                        global_compilation_context,
                    )?;
                    resolve_type(
                        &compiled_while.node.r#type,
                        &TypeKind::Bool,
                        compilation_context,
                    )?;
                    Some(compiled_while.node.clone())
                } else {
                    None
                };
                let r#type = Type {
                    kind: TypeKind::Array(Box::new(element_type)),
                    properties: TypeProperties {
                        capabilities: EnumSet::default(),
                        is_computable: compiled_while_node.is_some(),
                    }
                    .unified(&compiled_starting_with.node.r#type.properties)
                    .unified_all(
                        current_concrete_type_to_next
                            .keys()
                            .map(|r#type| &r#type.properties),
                    ),
                };
                NodeAndMetadata {
                    node: Node {
                        content: Content::Sequence(Arc::new(
                            intermediate_representation::Sequence {
                                starting_with: compiled_starting_with.node.clone(),
                                current_constant_name_clustered_index,
                                current_concrete_type_kind_and_next: current_concrete_type_to_next
                                    .into_iter()
                                    .map(|(r#type, node)| (r#type.kind, node))
                                    .collect::<Vec<_>>()
                                    .into(),
                                r#while: compiled_while_node,
                            },
                        )),
                        r#type,
                    }
                    .into(),
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                }
                .into()
            }
            Program::Pipe { pipe, r#as } => {
                if pipe.is_empty() {
                    return Err(compilation_context.error(&[0; 0], &"non-empty pipe"));
                }
                let mut compiled_pipe_elements = Vec::with_capacity(pipe.len());
                let mut compiled_previous_pipe_element_option: Option<Arc<NodeAndMetadata>> = None;
                let mut as_constant_name_clustered_index_option = None;
                let mut r#type = None;
                for (pipe_element_index, pipe_element) in pipe.iter().enumerate() {
                    let mut pipe_element_compilation_context = compilation_context.clone();
                    pipe_element_compilation_context.path.0.extend([
                        PathSegment::Pipe,
                        PathSegment::ArrayIndex(pipe_element_index),
                    ]);
                    if let Some(r#as) = r#as
                        && let Some(compiled_previous_pipe_element) =
                            compiled_previous_pipe_element_option
                    {
                        as_constant_name_clustered_index_option = Some(
                            self.define_constant(
                                r#as.clone(),
                                ConstantMetadata {
                                    r#type: compiled_previous_pipe_element.node.r#type.clone(),
                                },
                                &mut pipe_element_compilation_context,
                                global_compilation_context,
                            )
                            .name_clustered_index,
                        );
                    }
                    let compiled_pipe_element = self.compile_with_context(
                        pipe_element,
                        &pipe_element_compilation_context,
                        global_compilation_context,
                    )?;
                    compiled_previous_pipe_element_option = Some(compiled_pipe_element.clone());
                    if r#type.is_none() {
                        r#type = Some(compiled_pipe_element.node.r#type.clone());
                    } else {
                        r#type = Some(
                            compiled_pipe_element
                                .node
                                .r#type
                                .clone()
                                .with_unified_properties_from(&r#type.unwrap()),
                        )
                    }
                    compiled_pipe_elements.push(compiled_pipe_element);
                }
                NodeAndMetadata {
                    node: Node {
                        content: Content::Pipe {
                            pipe: compiled_pipe_elements
                                .iter()
                                .map(|compiled_pipe_element| compiled_pipe_element.node.clone())
                                .collect(),
                            as_constant_name_clustered_index_option,
                        },
                        r#type: r#type.unwrap(),
                    }
                    .into(),
                    external_constants_name_clustered_indices: compiled_pipe_elements
                        .iter()
                        .flat_map(|compiled_pipe_element| {
                            compiled_pipe_element
                                .external_constants_name_clustered_indices
                                .iter()
                                .cloned()
                        })
                        .collect(),
                }
                .into()
            }
            Program::Try { r#try, or, r#as } => {
                let mut try_compilation_context = compilation_context.clone();
                try_compilation_context.path.0.push(PathSegment::Try);
                let compiled_try = self.compile_with_context(
                    r#try,
                    &try_compilation_context,
                    global_compilation_context,
                )?;
                if !compiled_try.node.r#type.properties.is_computable {
                    return Err(
                        try_compilation_context.error(&"non-computible try", &"computible try")
                    );
                }
                let mut or_compilation_context = compilation_context.clone();
                or_compilation_context.path.0.push(PathSegment::Or);
                let as_constant_name_clustered_index = self
                    .define_constant(
                        r#as.clone(),
                        ConstantMetadata {
                            r#type: compiled_try.node.r#type.with_kind(TypeKind::String),
                        },
                        &mut or_compilation_context,
                        global_compilation_context,
                    )
                    .name_clustered_index;
                let compiled_or = self.compile_with_context(
                    or,
                    &or_compilation_context,
                    global_compilation_context,
                )?;
                NodeAndMetadata {
                    node: Node {
                        content: Content::Try {
                            r#try: compiled_try.node.clone(),
                            or: compiled_or.node.clone(),
                            as_constant_name_clustered_index,
                        },
                        r#type: Type::from(BTreeSet::from_iter([
                            compiled_try.node.r#type.clone(),
                            compiled_or.node.r#type.clone(),
                        ])),
                    }
                    .into(),
                    external_constants_name_clustered_indices: BTreeSet::from_iter(
                        compiled_try
                            .external_constants_name_clustered_indices
                            .iter()
                            .cloned()
                            .chain(
                                compiled_or
                                    .external_constants_name_clustered_indices
                                    .iter()
                                    .cloned(),
                            ),
                    ),
                }
                .into()
            }
            Program::Object(object) => {
                match object.len() {
                    0 => {
                        return Ok(NodeAndMetadata {
                            external_constants_name_clustered_indices: BTreeSet::new(),
                            node: Node {
                                content: Content::Object(BTreeMap::new()),
                                r#type: TypeKind::Object(BTreeMap::new().into()).into(),
                            }
                            .into(),
                        }
                        .into());
                    }
                    1 => {
                        let (function_name, function_argument) = object.iter().next().unwrap();
                        if function_name.ends_with(":") {
                            if let Some(function_body) =
                                compilation_context.available_functions.get(function_name)
                            {
                                let mut body_compilation_context = compilation_context.clone();
                                body_compilation_context
                                    .path
                                    .0
                                    .extend([PathSegment::UserFunctionCall(function_name.clone())]);
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
                                            PathSegment::UserFunctionCall(function_name.clone()),
                                            PathSegment::Argument(function_argument_name.clone()),
                                        ]);
                                        let compiled_constant = self.compile_with_context(
                                            &function_argument_body,
                                            &argument_compilation_context,
                                            global_compilation_context,
                                        )?;
                                        result_external_constants_name_clustered_indices.append(
                                            &mut compiled_constant
                                                .external_constants_name_clustered_indices
                                                .clone(),
                                        );
                                        let constant_definition = self.define_constant(
                                            function_argument_name.clone(),
                                            ConstantMetadata {
                                                r#type: compiled_constant.node.r#type.clone(),
                                            },
                                            &mut body_compilation_context,
                                            global_compilation_context,
                                        );
                                        new_constants_definitions.push(
                                            intermediate_representation::ConstantDefinition {
                                                name_clustered_index: constant_definition
                                                    .name_clustered_index,
                                                node: compiled_constant.node.clone(),
                                            },
                                        );
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
                                                arguments_is_pure: function_type
                                                    .properties
                                                    .capabilities
                                                    .is_empty(),
                                            },
                                            r#type: function_type.clone(),
                                        }
                                        .into(),
                                    }
                                    .into());
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
                                        },
                                    );
                                    global_compilation_context
                                        .user_function_to_index_and_type_option
                                        .insert(
                                            function_body_as_maybe_compiled_program.clone(),
                                            (
                                                function_index,
                                                TypeKind::Unknown(MaybeType::default()).into(),
                                            ),
                                        );
                                    let compiled_function = self.compile_with_context(
                                        function_body,
                                        &body_compilation_context,
                                        global_compilation_context,
                                    )?;
                                    global_compilation_context
                                        .user_function_to_index_and_type_option
                                        .get_mut(&function_body_as_maybe_compiled_program)
                                        .unwrap()
                                        .1 = compiled_function.node.r#type.clone();
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
                                    };
                                    result_external_constants_name_clustered_indices.append(
                                        &mut compiled_function
                                            .external_constants_name_clustered_indices
                                            .clone(),
                                    );
                                    return Ok(Arc::new(NodeAndMetadata {
                                        external_constants_name_clustered_indices:
                                            result_external_constants_name_clustered_indices.clone(),
                                        node: Node {
                                            content: Content::UserFunctionCall {
                                                arguments: new_constants_definitions,
                                                body: function_index,
                                                arguments_is_pure: compiled_function
                                                    .node
                                                    .r#type
                                                    .properties
                                                    .capabilities
                                                    .is_empty(),
                                            },
                                            r#type: compiled_function.node.r#type.clone(),
                                        }
                                        .into(),
                                    }));
                                }
                            } else {
                                return Err(compilation_context.error(
                                    function_name,
                                    &BTreeMap::from_iter([(
                                        &"one of available functions names",
                                        &compilation_context
                                            .available_functions
                                            .keys()
                                            .collect::<Vec<_>>(),
                                    )]),
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
                    let compiled_object_value = self.compile_with_context(
                        object_value,
                        &object_value_compilation_context,
                        global_compilation_context,
                    )?;
                    result_external_constants_name_clustered_indices.append(
                        &mut compiled_object_value
                            .external_constants_name_clustered_indices
                            .clone(),
                    );
                    result_inner_types.insert(
                        object_key.clone(),
                        compiled_object_value.node.r#type.clone(),
                    );
                    result_content.insert(object_key.clone(), compiled_object_value.node.clone());
                }
                let result_inner_types_arc = Arc::new(result_inner_types);
                NodeAndMetadata {
                    external_constants_name_clustered_indices:
                        result_external_constants_name_clustered_indices,
                    node: Node {
                        content: Content::Object(result_content),
                        r#type: (
                            TypeKind::Object(result_inner_types_arc.clone()),
                            result_inner_types_arc.values(),
                        )
                            .into(),
                    }
                    .into(),
                }
                .into()
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
                    r#type: Value::type_kind(value_option).into(),
                }
                .into(),
            }
            .into(),
        })
    }
}
