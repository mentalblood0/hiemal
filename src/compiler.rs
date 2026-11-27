use std::{
    collections::{BTreeMap, BTreeSet},
    hash::Hash,
    sync::Arc,
};

use anyhow::{Context, Error, Result, anyhow};
use enumset::EnumSet;
use gxhash::HashMap;
use regex::Regex;
use serde::Serialize;

use crate::{
    computer::compute,
    containers::{Object, SetRaw, Vector},
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    embedded_functions::{EMBEDDED_FUNCTIONS, EmbeddedFunction},
    intermediate_representation::{self, Case, Content, IntermediateRepresentation, Node, Throughs},
    program::{
        Condition, Constant, DefinedName, Function, FunctionCall, Match, Module, Path, PathSegment, Program, ProgramAndMetadata, UserFunctionPath, UserFunctionTag,
    },
    r#type::{Capability, Embedded, Type, TypeKind, TypeProperties},
    value::Value,
};

#[derive(Clone, Default)]
struct CompilationContext {
    path: Path,
    available_functions: Object<UserFunctionPath, ProgramAndMetadata>,
    available_constants: Object<String, Type>,
    entered_user_functions: SetRaw<u128>,
}

impl CompilationContext {
    fn with_additional_path<A: IntoIterator<Item = PathSegment>>(&self, addition: A) -> Self {
        Self {
            path: self.path.extended(addition),
            available_functions: self.available_functions.clone(),
            available_constants: self.available_constants.clone(),
            entered_user_functions: self.entered_user_functions.clone(),
        }
    }
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
        anyhow!("\n{}", serde_saphyr::to_string(&CompilationTypeError { got, expected, r#at: &self.path }).unwrap())
    }
}

fn assert_type(got_type: &Type, expected_type_kind: &TypeKind, compilation_context: &CompilationContext) -> Result<()> {
    if &got_type.kind == expected_type_kind {
        Ok(())
    } else {
        match (&got_type.kind, expected_type_kind) {
            (TypeKind::LiteralString(_), TypeKind::String | TypeKind::GenericLiteralString)
            | (TypeKind::GenericLiteralString, TypeKind::String)
            | (TypeKind::LiteralNumber(_), TypeKind::Number)
            | (TypeKind::LiteralTrue | TypeKind::LiteralFalse, TypeKind::Bool) => Ok(()),
            (_, TypeKind::Any) => Ok(()),
            (TypeKind::Constructed(got_constructed), TypeKind::Constructed(expected_constructed)) => {
                assert_type(got_constructed.inner(), &expected_constructed.inner().kind, compilation_context)
            }
            (TypeKind::Constructed(got_constructed), _) => assert_type(got_constructed.inner(), expected_type_kind, compilation_context),
            (_, TypeKind::Constructed(expected_constructed)) => assert_type(got_type, &expected_constructed.inner().kind, compilation_context),
            (TypeKind::Tuple(self_elements_types), TypeKind::Tuple(other_elements_types)) => self_elements_types
                .iter()
                .zip(other_elements_types.iter())
                .all(|(self_element_type, other_element_type)| assert_type(self_element_type, &other_element_type.kind, compilation_context).is_ok())
                .ok_or_else(|| compilation_context.error(got_type, expected_type_kind)),
            (TypeKind::Array(got_element_type), TypeKind::Array(expected_element_type)) => {
                assert_type(got_element_type, &expected_element_type.kind, compilation_context)
            }
            (TypeKind::Array(got_element_type), TypeKind::Tuple(expected_elements_types)) => {
                for expected_element_type in expected_elements_types.iter() {
                    assert_type(got_element_type, &expected_element_type.kind, compilation_context)?;
                }
                Ok(())
            }
            (TypeKind::Tuple(got_elements_types), TypeKind::Array(expected_element_type)) => {
                for got_element_type in got_elements_types.iter() {
                    assert_type(got_element_type, &expected_element_type.kind, compilation_context)?;
                }
                Ok(())
            }
            (TypeKind::Object(got_inner_types), TypeKind::Object(expected_inner_types)) => {
                for (expected_value_key, expected_value_type) in expected_inner_types.iter() {
                    if let Some(got_value_type) = got_inner_types.get(expected_value_key) {
                        assert_type(got_value_type, &expected_value_type.kind, compilation_context)?;
                    } else {
                        return Err(compilation_context.error(got_type, expected_type_kind));
                    }
                }
                Ok(())
            }
            (TypeKind::Object(got_inner_types), TypeKind::GenericObject(expected_value_type)) => {
                for got_value_type in got_inner_types.values() {
                    assert_type(got_value_type, &expected_value_type.kind, compilation_context)?;
                }
                Ok(())
            }
            (TypeKind::GenericObject(got_value_type), TypeKind::GenericObject(expected_value_type)) => {
                let mut inner_compilation_context = compilation_context.clone();
                inner_compilation_context.path.0.push(PathSegment::ArrayIndex(0));
                assert_type(got_value_type, &expected_value_type.kind, &inner_compilation_context)
            }
            (TypeKind::Union(got_union_types), TypeKind::Union(expected_union_types)) => {
                if !got_union_types.is_subset(expected_union_types) {
                    for one_of_got_types in got_union_types.iter() {
                        let mut found = false;
                        for one_of_expected_types in expected_union_types.iter() {
                            if assert_type(one_of_got_types, &one_of_expected_types.kind, compilation_context).is_ok() {
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            return Err(compilation_context.error(got_type, expected_type_kind));
                        }
                    }
                }
                Ok(())
            }
            (TypeKind::Union(got_union_types), _) => {
                for one_of_got_types in got_union_types.iter() {
                    assert_type(one_of_got_types, expected_type_kind, compilation_context)?;
                }
                Ok(())
            }
            (_, TypeKind::Union(expected_union_types)) => {
                if !expected_union_types.contains(&expected_type_kind.clone().into()) {
                    for one_of_expected_types in expected_union_types.iter() {
                        if let Ok(result_type) = assert_type(got_type, &one_of_expected_types.kind, compilation_context) {
                            return Ok(result_type);
                        }
                    }
                    return Err(compilation_context.error(got_type, expected_type_kind));
                }
                Ok(())
            }
            _ => Err(compilation_context.error(got_type, expected_type_kind)),
        }
    }
}

struct ExportedFunction {
    compilation_context: CompilationContext,
    body: Arc<ProgramAndMetadata>,
}

#[derive(Default)]
struct GlobalCompilationContext {
    user_functions_calls: Vec<Arc<Node>>,
    exported_functions: HashMap<Arc<UserFunctionPath>, ExportedFunction>,
    constants_names_to_name_clustered_constants_indices: HashMap<Arc<String>, usize>,
    compiled_programs_cache: HashMap<u128, (Arc<Node>, std::time::Duration)>,
    compiled_programs_cache_misses: usize,
    compiled_programs_cache_hits: usize,
    time_spent_on_hashing: std::time::Duration,
    saved_compilation_time: std::time::Duration,
}

fn compile_module(module: &Module, global_compilation_context: &mut GlobalCompilationContext) -> Result<HashMap<Arc<UserFunctionPath>, ExportedFunction>> {
    let mut result = HashMap::default();
    let mut current_compilation_context = CompilationContext::default();
    for (define_index, define) in module.defines.iter().enumerate() {
        current_compilation_context.path.0 = vec![PathSegment::Define(define_index)];
        for (user_function_tags_and_path, body) in define.functions() {
            current_compilation_context.available_functions.insert(user_function_tags_and_path.path.clone(), body.clone());
            if user_function_tags_and_path.tags.contains(UserFunctionTag::Export) {
                result.insert(
                    user_function_tags_and_path.path.clone(),
                    ExportedFunction {
                        compilation_context: current_compilation_context.clone(),
                        body: body.clone(),
                    },
                );
            }
        }
        {
            let mut compiled_constants = Vec::new();
            for (constant_name, program_and_metadata) in define.constants() {
                compiled_constants.push((
                    constant_name,
                    compile_with_context(program_and_metadata, &current_compilation_context, global_compilation_context)?,
                ));
            }
            for (name, compiled_constant) in compiled_constants {
                define_constant(
                    name.clone(),
                    compiled_constant.r#type.clone(),
                    &mut current_compilation_context,
                    global_compilation_context,
                );
            }
        }
    }
    Ok(result)
}

pub fn compile_program(program_and_metadata: &ProgramAndMetadata) -> Result<Arc<IntermediateRepresentation>> {
    let mut global_compilation_context = GlobalCompilationContext::default();
    let compilation_context = CompilationContext::default();
    let compiled_program = compile_with_context(program_and_metadata, &compilation_context, &mut global_compilation_context)?;
    if !compiled_program.r#type.properties.is_computable {
        Err(compilation_context.error(&"non-computable program", &"computable program"))
    } else if !compiled_program.r#type.properties.is_serializable {
        Err(compilation_context.error(
            &"program with non-serializable result (it includes embedded, runtime-only types)",
            &"program with serializable result",
        ))
    } else {
        eprintln!(
            "compiled subprograms cache:\n  hits: {}\n  misses: {}\n  time spent on hashing: {:?}\n  saved compilation time: {:?}",
            global_compilation_context.compiled_programs_cache_hits,
            global_compilation_context.compiled_programs_cache_misses,
            global_compilation_context.time_spent_on_hashing,
            global_compilation_context.saved_compilation_time
        );
        Ok(Arc::new(IntermediateRepresentation {
            root: compiled_program,
            user_functions: global_compilation_context.user_functions_calls,
            unique_constants_names_count: global_compilation_context.constants_names_to_name_clustered_constants_indices.len(),
        }))
    }
}

fn define_constant(name: Arc<String>, r#type: Type, compilation_context: &mut CompilationContext, global_compilation_context: &mut GlobalCompilationContext) -> usize {
    let result = if let Some(constant_name_clustered_index) = global_compilation_context.constants_names_to_name_clustered_constants_indices.get(&name) {
        *constant_name_clustered_index
    } else {
        let result = global_compilation_context.constants_names_to_name_clustered_constants_indices.len();
        global_compilation_context.constants_names_to_name_clustered_constants_indices.insert(name.clone(), result);
        result
    };
    compilation_context.available_constants.insert(name, r#type.into());
    result
}

#[allow(clippy::too_many_arguments)]
fn compile_embedded_function_call(
    compilation_context: &CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
    embedded_function_call: &FunctionCall,
    get_result_type_from_argument_resolved_type_option: Option<&dyn Fn(&Type, u8) -> Result<Type>>,
) -> Result<Arc<Node>> {
    let argument_compilation_context =
        compilation_context.with_additional_path([PathSegment::EmbeddedFunctionCall(*embedded_function_call.function.as_embedded().unwrap())]);
    let compiled_argument = compile_with_context(&embedded_function_call.argument, &argument_compilation_context, global_compilation_context)?;
    if embedded_function_call.function.as_embedded().unwrap() == &EmbeddedFunction::SerializeTo && !compiled_argument.r#type.properties.is_serializable {
        return Err(compilation_context.error(&compiled_argument.r#type, &"serializable value"));
    }
    let (signature, variant_index, _) = EMBEDDED_FUNCTIONS[*embedded_function_call.function.as_embedded().unwrap() as usize]
        .iter()
        .enumerate()
        .find_map(|(signature_index, signature)| {
            if assert_type(&compiled_argument.r#type, &signature.argument, &argument_compilation_context).is_ok() {
                Some((signature, signature_index as u8, signature.argument_is_computable))
            } else {
                None
            }
        })
        .ok_or_else(|| {
            argument_compilation_context.error(
                &compiled_argument.r#type,
                &Type::from(BTreeSet::from_iter(
                    EMBEDDED_FUNCTIONS[*embedded_function_call.function.as_embedded().unwrap() as usize].iter().map(|signature| signature.argument.clone().into()),
                )),
            )
        })?;
    // if argument_is_computable && !compiled_argument.node.r#type.properties.is_computable {
    //     return Err(argument_compilation_context.error(
    //         &"non-computable embedded function argument",
    //         &"computable embedded function argument",
    //     ));
    // }
    let static_properties = &match &signature.r#return.properties {
        Some(static_properties) => static_properties.clone(),
        Option::None => TypeProperties::default(),
    };
    let static_and_argument_properties = TypeProperties {
        capabilities: static_properties.capabilities | compiled_argument.r#type.properties.capabilities,
        is_computable: true,
        is_serializable: static_properties.is_serializable,
        is_pure: static_properties.is_pure && compiled_argument.r#type.properties.is_pure,
    };
    let result_type = if let Some(get_result_type_kind_from_argument_resolved_type) = get_result_type_from_argument_resolved_type_option {
        get_result_type_kind_from_argument_resolved_type(&compiled_argument.r#type, variant_index)?
    } else {
        match signature.r#return.r#type {
            Some(ref return_type) => return_type.clone().into(),
            Option::None => panic!(),
        }
    }
    .with_recursively_updated_properties(&|properties| {
        let mut result = properties.unified(&static_and_argument_properties);
        result.is_computable = true;
        // result.is_serializable = signature
        //     .r#return
        //     .properties
        //     .as_ref()
        //     .map(|properties| properties.is_serializable)
        //     .unwrap_or(true);
        result
    });
    Ok(Node {
        content: Content::EmbeddedFunctionCall {
            path_option: if result_type.properties.capabilities.contains(Capability::Error) {
                Some(argument_compilation_context.path.clone())
            } else {
                None
            },
            embedded_function_call: intermediate_representation::EmbeddedFunctionCall {
                embedded_function: *embedded_function_call.function.as_embedded().unwrap(),
                variant_index,
                argument: compiled_argument,
            },
        },
        r#type: result_type,
    }
    .into())
}

fn hash_with_context(
    program_with_hash: &ProgramAndMetadata,
    compilation_context: &CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
) -> Option<u128> {
    if program_with_hash.value_only {
        Some(program_with_hash.hash)
    } else {
        let hashing_start = std::time::Instant::now();
        let mut hasher = gxhash::GxHasher::default();
        program_with_hash.hash.hash(&mut hasher);
        for constant_name in program_with_hash.external_constants_names.iter() {
            constant_name.hash(&mut hasher);
            compilation_context.available_constants.get(constant_name)?.hash(&mut hasher);
        }
        for function_name in program_with_hash.external_functions_names.iter() {
            function_name.hash(&mut hasher);
            compilation_context.available_functions.get(function_name)?.hash.hash(&mut hasher);
        }
        let result = hasher.finish_u128();
        let time_spent_on_hashing = hashing_start.elapsed();
        global_compilation_context.time_spent_on_hashing += time_spent_on_hashing;
        Some(result)
    }
}

fn compile_with_context(
    program_and_metadata: &ProgramAndMetadata,
    compilation_context: &CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
) -> Result<Arc<Node>> {
    if program_and_metadata.compiles_easily && !program_and_metadata.value_only {
        compile_with_context_without_cache(program_and_metadata, compilation_context, global_compilation_context)
    } else {
        let instantiated_program_hash_option = hash_with_context(program_and_metadata, compilation_context, global_compilation_context);
        if let Some(instantiated_program_hash) = instantiated_program_hash_option
            && let Some((cached_compilation_result, saved_compilation_time)) = global_compilation_context.compiled_programs_cache.get(&instantiated_program_hash)
        {
            global_compilation_context.compiled_programs_cache_hits += 1;
            global_compilation_context.saved_compilation_time += *saved_compilation_time;
            Ok(cached_compilation_result.clone())
        } else {
            global_compilation_context.compiled_programs_cache_misses += 1;
            let compilation_start = std::time::Instant::now();
            let result = compile_with_context_without_cache(program_and_metadata, compilation_context, global_compilation_context)?;
            let compilation_duration = compilation_start.elapsed();
            if let Some(instantiated_program_hash) = instantiated_program_hash_option {
                global_compilation_context.compiled_programs_cache.insert(instantiated_program_hash, (result.clone(), compilation_duration));
            } else {
                return Err(compilation_context.error(
                    &"no error",
                    &"error about usage of undefined constant or function, because hash of subprogram and context were not computed because some external names this \
                      subprogram uses were not found in the context",
                ));
            }
            Ok(result)
        }
    }
}

fn compile_with_context_without_cache(
    program_with_hash: &ProgramAndMetadata,
    compilation_context: &CompilationContext,
    global_compilation_context: &mut GlobalCompilationContext,
) -> Result<Arc<Node>> {
    Ok(match &program_with_hash.program {
        Program::Bytes { bytes } => Node {
            content: Content::Value(Some(intermediate_representation::Value::Bytes { bytes: bytes.clone() }).into()),
            r#type: TypeKind::Bytes.into(),
        }
        .into(),
        Program::Time { time } => Node {
            content: Content::Value(Some(intermediate_representation::Value::Time { time: *time }).into()),
            r#type: TypeKind::Time.into(),
        }
        .into(),
        Program::Duration { duration } => Node {
            content: Content::Value(Some(intermediate_representation::Value::Duration { duration: *duration }).into()),
            r#type: TypeKind::Duration.into(),
        }
        .into(),
        Program::Value(value_option) => Node {
            content: Content::Value(unsafe { std::mem::transmute::<Arc<Option<Value>>, Arc<Option<intermediate_representation::Value>>>(value_option.clone()) }),
            r#type: Value::type_kind(value_option).into(),
        }
        .into(),
        _ => {
            let result = match &program_with_hash.program {
                Program::Tuple(tuple) => {
                    if tuple.is_empty() {
                        return Ok(Node {
                            content: Content::Value(Some(intermediate_representation::Value::Tuple(Vector::default())).into()),
                            r#type: TypeKind::Tuple(vec![].into()).into(),
                        }
                        .into());
                    }
                    let mut result_content = Vec::with_capacity(tuple.len());
                    let mut result_elements_types = Vec::with_capacity(tuple.len());
                    for (element_index, element) in tuple.iter().enumerate() {
                        let element_compilation_context = compilation_context.with_additional_path([PathSegment::ArrayIndex(element_index)]);
                        let compiled_element = compile_with_context(element, &element_compilation_context, global_compilation_context)?;
                        result_elements_types.push(compiled_element.r#type.clone());
                        result_content.push(compiled_element);
                    }
                    Node {
                        content: Content::Tuple(result_content),
                        r#type: result_elements_types.into(),
                    }
                    .into()
                }
                Program::Scope { define, compute, allow, forbid } => {
                    if allow.is_some() && forbid.is_some() {
                        return Err(compilation_context.error(&BTreeMap::from_iter([("allow", &allow), ("forbid", &forbid)]), &"either `allow` or `forbid`"));
                    }
                    let mut compute_compilation_context = compilation_context.with_additional_path([PathSegment::Compute]);
                    let mut new_constants = Vec::new();
                    let mut constants_name_clustered_indices = Vec::new();
                    let mut result_type_properties = TypeProperties::default();
                    for (constant_name, constant_compute_body) in define.constants() {
                        let constant_compilation_context =
                            compilation_context.with_additional_path([PathSegment::Constants, PathSegment::Constant(constant_name.clone())]);
                        let compiled_constant = compile_with_context(constant_compute_body, &constant_compilation_context, global_compilation_context)?;
                        let constant_definition = define_constant(
                            constant_name.clone(),
                            compiled_constant.r#type.clone(),
                            &mut compute_compilation_context,
                            global_compilation_context,
                        );
                        constants_name_clustered_indices.push(constant_definition);
                        new_constants.push(intermediate_representation::ConstantDefinition {
                            name_clustered_index: constant_definition,
                            node: compiled_constant,
                        });
                    }
                    for (function_tags_and_path, function_body) in define.functions() {
                        if function_tags_and_path.tags.contains(UserFunctionTag::Export) {
                            global_compilation_context.exported_functions.insert(
                                function_tags_and_path.path.clone(),
                                ExportedFunction {
                                    compilation_context: compilation_context.clone(),
                                    body: function_body.clone(),
                                },
                            );
                        }
                        compute_compilation_context.available_functions.insert(function_tags_and_path.path.clone(), function_body.clone());
                    }
                    let compiled_compute = compile_with_context(compute, &compute_compilation_context, global_compilation_context)?;
                    result_type_properties.unify(&compiled_compute.r#type.properties);
                    if let Some(allow) = allow {
                        let used_not_allowed_capabilities = compiled_compute.r#type.properties.capabilities - *allow;
                        if !used_not_allowed_capabilities.is_empty() {
                            return Err(compute_compilation_context.error(&used_not_allowed_capabilities, allow));
                        }
                    } else if let Some(forbid) = forbid {
                        let used_forbidden_capabilities = compiled_compute.r#type.properties.capabilities & *forbid;
                        if !used_forbidden_capabilities.is_empty() {
                            return Err(compute_compilation_context.error(&used_forbidden_capabilities, &EnumSet::default().difference(*forbid)));
                        }
                    }
                    let result_type_kind = compiled_compute.r#type.kind.clone();
                    Node {
                        content: Content::Scope {
                            constants: new_constants,
                            compute: compiled_compute,
                        },
                        r#type: Type {
                            kind: result_type_kind,
                            properties: result_type_properties,
                        },
                    }
                    .into()
                }
                Program::Constant(Constant { constant: constant_name }) => {
                    if let Some(constant_type) = compilation_context.available_constants.get(constant_name) {
                        let name_clustered_constant_index = *global_compilation_context.constants_names_to_name_clustered_constants_indices.get(constant_name).unwrap();
                        Node {
                            content: Content::Constant(name_clustered_constant_index),
                            r#type: (*constant_type).clone(),
                        }
                        .into()
                    } else {
                        return Err(compilation_context.error(
                            constant_name,
                            &BTreeMap::from_iter([(
                                &"one of available constants names",
                                &compilation_context.available_constants.keys().collect::<Vec<_>>(),
                            )]),
                        ));
                    }
                }
                Program::Match { r#match, r#as, cases } => {
                    let (r#match, match_constant_name) = match r#match {
                        Match::Constant(constant_name) => (&Program::Constant(Constant { constant: constant_name.clone() }).into(), Some(constant_name)),
                        Match::Program(program_with_hash) => (&**program_with_hash, None),
                    };
                    let match_compilation_context = compilation_context.with_additional_path([PathSegment::Match]);
                    let compiled_match = compile_with_context(r#match, &match_compilation_context, global_compilation_context)?;
                    let mut result_cases = Vec::new();
                    let mut result_types = BTreeSet::new();
                    let mut current_match_type_kind_option = Some(compiled_match.r#type.kind.clone());
                    let mut covered_types_kinds = BTreeSet::new();
                    let match_constant_name_clustered_index_option = if let Some(match_constant_name) = r#as {
                        Some(
                            if let Some(result) = global_compilation_context.constants_names_to_name_clustered_constants_indices.get(match_constant_name) {
                                *result
                            } else {
                                global_compilation_context.constants_names_to_name_clustered_constants_indices.len()
                            },
                        )
                    } else {
                        None
                    };
                    let mut has_effective_values_conditions = false;
                    for (case_index, (case_condition, case)) in cases.iter().enumerate() {
                        if let Some(ref current_match_type_kind) = current_match_type_kind_option {
                            match case_condition {
                                Condition::Type(case_match_type) => {
                                    if let Some(refined_match_type_kind) = current_match_type_kind.intersection(&case_match_type.kind) {
                                        let mut case_compilation_context = compilation_context.with_additional_path([PathSegment::Cases, PathSegment::Case(case_index)]);
                                        if let Some(match_constant_name) = r#as.as_ref().or(match_constant_name) {
                                            define_constant(
                                                match_constant_name.clone(),
                                                compiled_match.r#type.with_kind(refined_match_type_kind),
                                                &mut case_compilation_context,
                                                global_compilation_context,
                                            );
                                        }
                                        let compiled_case = compile_with_context(case, &case_compilation_context, global_compilation_context)?;
                                        result_types.insert(compiled_case.r#type.clone());
                                        covered_types_kinds.insert(case_match_type.kind.clone());
                                        result_cases.push(Case {
                                            condition: intermediate_representation::Condition::Type(case_match_type.clone()),
                                            node: compiled_case,
                                        });
                                        current_match_type_kind_option = current_match_type_kind.substraction(&case_match_type.kind);
                                        if current_match_type_kind_option.is_none() {
                                            break;
                                        }
                                    }
                                }
                                Condition::Value(condition) => {
                                    let mut case_compilation_context = compilation_context.with_additional_path([PathSegment::Cases, PathSegment::Case(case_index)]);
                                    let compiled_condition = compile_with_context(&condition, &case_compilation_context, global_compilation_context)?;
                                    if let Some(refined_match_type_kind) = compiled_match.r#type.kind.intersection(&compiled_condition.r#type.kind) {
                                        if let Some(match_constant_name) = r#as.as_ref().or(match_constant_name) {
                                            define_constant(
                                                match_constant_name.clone(),
                                                refined_match_type_kind.clone().into(),
                                                &mut case_compilation_context,
                                                global_compilation_context,
                                            );
                                        }
                                        let compiled_case = compile_with_context(case, &case_compilation_context, global_compilation_context)?;
                                        result_types.insert(compiled_case.r#type.clone());
                                        result_cases.push(Case {
                                            condition: intermediate_representation::Condition::Value(compiled_condition.clone()),
                                            node: compiled_case,
                                        });
                                        if matches!(
                                            refined_match_type_kind,
                                            TypeKind::LiteralTrue | TypeKind::LiteralFalse | TypeKind::LiteralString(_) | TypeKind::LiteralNumber(_) | TypeKind::Null
                                        ) {
                                            current_match_type_kind_option = current_match_type_kind.substraction(&refined_match_type_kind);
                                            covered_types_kinds.insert(refined_match_type_kind);
                                            if current_match_type_kind_option.is_none() {
                                                break;
                                            }
                                            if !compiled_condition.r#type.properties.is_computable {
                                                return Err(case_compilation_context.error(&"non-computable case condition value", &"computable case condition value"));
                                            }
                                            has_effective_values_conditions = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if has_effective_values_conditions && !compiled_match.r#type.properties.is_computable {
                        return Err(match_compilation_context.error(&"non-computable match", &"computable match"));
                    }
                    let covered = Type::from(covered_types_kinds.iter().map(|kind| kind.clone().into()).collect::<BTreeSet<_>>());
                    if !covered.kind.contains(&compiled_match.r#type.kind) {
                        return Err(compilation_context.error(
                            &BTreeMap::from_iter([("coverage for", covered)]),
                            &BTreeMap::from_iter([("coverage for", &compiled_match.r#type)]),
                        ));
                    }
                    if result_cases.len() == 1 && r#as.as_ref().or(match_constant_name).is_none() {
                        result_cases.into_iter().next().unwrap().node
                    } else if result_cases.is_empty() {
                        return Err(match_compilation_context.error(
                            &Option::<String>::None,
                            &BTreeMap::from_iter([("at least one valid case for match", &compiled_match.r#type)]),
                        ));
                    } else {
                        Node {
                            content: Content::Match {
                                r#match: compiled_match,
                                cases: result_cases,
                                match_constant_name_clustered_index_option,
                            },
                            r#type: Type::from(result_types),
                        }
                        .into()
                    }
                }
                Program::Map { map, r#as, through } => {
                    let map_compilation_context = compilation_context.with_additional_path([PathSegment::Map]);
                    let compiled_map = compile_with_context(map, &map_compilation_context, global_compilation_context)?;
                    let map_constant_name_clustered_index = if let Some(result) = global_compilation_context.constants_names_to_name_clustered_constants_indices.get(r#as)
                    {
                        *result
                    } else {
                        global_compilation_context.constants_names_to_name_clustered_constants_indices.len()
                    };
                    let mut map_concrete_type_and_throughs = Vec::with_capacity(compiled_map.r#type.union_types_len());
                    let mut result_union_types = BTreeSet::new();
                    for map_concrete_type in compiled_map.r#type.union_types() {
                        map_concrete_type_and_throughs.push((
                            map_concrete_type.clone(),
                            match &map_concrete_type.kind {
                                TypeKind::Tuple(map_tuple_elements_types) => {
                                    let mut result_elements_types = Vec::with_capacity(map_tuple_elements_types.len());
                                    let mut result_throughs_nodes_indexes = Vec::with_capacity(map_tuple_elements_types.len());
                                    let mut compiled_throughs: indexmap::IndexSet<Arc<Node>> = indexmap::IndexSet::new();
                                    let mut element_type_to_compiled_through_index: BTreeMap<Type, usize> = BTreeMap::new();
                                    for (element_type_index, element_type) in map_tuple_elements_types.iter().enumerate() {
                                        if let Some(element_through_index) = element_type_to_compiled_through_index.get(element_type) {
                                            result_elements_types.push(compiled_throughs[*element_through_index].r#type.clone());
                                            result_throughs_nodes_indexes.push(*element_through_index);
                                        } else {
                                            let mut through_compilation_context = compilation_context.with_additional_path([PathSegment::Through(element_type_index)]);
                                            define_constant(r#as.clone(), element_type.clone(), &mut through_compilation_context, global_compilation_context);
                                            let compiled_through = compile_with_context(through, &through_compilation_context, global_compilation_context)?;
                                            result_elements_types.push(compiled_through.r#type.clone());
                                            let compiled_through_index = if let Some(compiled_through_index) = compiled_throughs.get_index_of(&compiled_through) {
                                                compiled_through_index
                                            } else {
                                                compiled_throughs.insert(compiled_through);
                                                compiled_throughs.len() - 1
                                            };
                                            element_type_to_compiled_through_index.insert(element_type.clone(), compiled_through_index);
                                            result_throughs_nodes_indexes.push(compiled_through_index);
                                        }
                                    }
                                    result_union_types.insert(Type::from(result_elements_types));
                                    Throughs::Tuple {
                                        nodes_indexes: result_throughs_nodes_indexes,
                                        nodes: compiled_throughs.into_iter().collect(),
                                    }
                                }
                                TypeKind::Array(map_array_element_type) => {
                                    let mut through_compilation_context = compilation_context.with_additional_path([PathSegment::Through(0)]);
                                    define_constant(
                                        r#as.clone(),
                                        *map_array_element_type.clone(),
                                        &mut through_compilation_context,
                                        global_compilation_context,
                                    );
                                    let compiled_through = compile_with_context(through, &through_compilation_context, global_compilation_context)?;
                                    result_union_types.insert(map_concrete_type.with_kind(TypeKind::Array(Box::new(compiled_through.r#type.clone()))));
                                    Throughs::Array(compiled_through)
                                }
                                _ => {
                                    return Err(map_compilation_context.error(map_concrete_type, &"tuple or array"));
                                }
                            },
                        ));
                    }
                    let r#type = Type::from(result_union_types).with_unified_properties_from(&compiled_map.r#type);
                    Node {
                        content: Content::Map(Arc::new(intermediate_representation::Map {
                            map: compiled_map,
                            map_concrete_type_and_throughs,
                            map_constant_name_clustered_index,
                        })),
                        r#type,
                    }
                    .into()
                }
                Program::Flatten { flatten } => {
                    let mut flatten_compilation_context = compilation_context.clone();
                    flatten_compilation_context.path.0.push(PathSegment::Flatten);
                    let compiled_flatten = compile_with_context(flatten, &flatten_compilation_context, global_compilation_context)?;
                    assert_type(
                        &compiled_flatten.r#type,
                        &TypeKind::Array(Box::new(TypeKind::Array(Box::new(TypeKind::Any.into())).into())),
                        compilation_context,
                    )?;
                    let r#type = compiled_flatten.r#type.flatten()?;
                    Node {
                        content: Content::Flatten(compiled_flatten),
                        r#type,
                    }
                    .into()
                }
                Program::Filter { filter, r#as, through } => {
                    let filter_compilation_context = compilation_context.with_additional_path([PathSegment::Filter]);
                    let compiled_filter = compile_with_context(filter, &filter_compilation_context, global_compilation_context)?;
                    let filter_constant_name_clustered_index =
                        if let Some(result) = global_compilation_context.constants_names_to_name_clustered_constants_indices.get(r#as) {
                            *result
                        } else {
                            global_compilation_context.constants_names_to_name_clustered_constants_indices.len()
                        };
                    let mut filter_concrete_type_and_throughs = Vec::with_capacity(compiled_filter.r#type.union_types_len());
                    let mut result_union_types = BTreeSet::new();
                    for filter_concrete_type in compiled_filter.r#type.union_types() {
                        filter_concrete_type_and_throughs.push((
                            filter_concrete_type.clone(),
                            match &filter_concrete_type.kind {
                                TypeKind::Tuple(filter_tuple_elements_types) => {
                                    let mut result_throughs_nodes_indexes = Vec::with_capacity(filter_tuple_elements_types.len());
                                    let mut compiled_throughs: indexmap::IndexSet<Arc<Node>> = indexmap::IndexSet::new();
                                    for (element_type_index, element_type) in filter_tuple_elements_types.iter().enumerate() {
                                        let mut through_compilation_context = compilation_context.with_additional_path([PathSegment::Through(element_type_index)]);
                                        define_constant(r#as.clone(), element_type.clone(), &mut through_compilation_context, global_compilation_context);
                                        let compiled_through = compile_with_context(through, &through_compilation_context, global_compilation_context)?;
                                        assert_type(&compiled_through.r#type, &TypeKind::Bool, compilation_context)?;
                                        let compiled_through_index = if let Some(compiled_through_index) = compiled_throughs.get_index_of(&compiled_through) {
                                            compiled_through_index
                                        } else {
                                            compiled_throughs.insert(compiled_through);
                                            compiled_throughs.len() - 1
                                        };
                                        result_throughs_nodes_indexes.push(compiled_through_index);
                                    }
                                    result_union_types.insert(filter_concrete_type.with_kind(TypeKind::Array(Box::new(Type::from(BTreeSet::from_iter(
                                        filter_tuple_elements_types.iter().cloned(),
                                    ))))));
                                    Throughs::Tuple {
                                        nodes_indexes: result_throughs_nodes_indexes,
                                        nodes: compiled_throughs.into_iter().collect(),
                                    }
                                }
                                TypeKind::Array(filter_array_element_type) => {
                                    let mut through_compilation_context = compilation_context.with_additional_path([PathSegment::Through(0)]);
                                    define_constant(
                                        r#as.clone(),
                                        *filter_array_element_type.clone(),
                                        &mut through_compilation_context,
                                        global_compilation_context,
                                    );
                                    let compiled_through = compile_with_context(through, &through_compilation_context, global_compilation_context)?;
                                    assert_type(&compiled_through.r#type, &TypeKind::Bool, compilation_context)?;
                                    result_union_types.insert(filter_concrete_type.clone());
                                    Throughs::Array(compiled_through)
                                }
                                _ => {
                                    return Err(filter_compilation_context.error(filter_concrete_type, &"tuple or array"));
                                }
                            },
                        ));
                    }
                    let r#type = Type::from(result_union_types).with_unified_properties_from(&compiled_filter.r#type);
                    Node {
                        content: Content::Filter(Arc::new(intermediate_representation::Filter {
                            filter: compiled_filter,
                            filter_concrete_type_and_throughs,
                            filter_constant_name_clustered_index,
                        })),
                        r#type,
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
                    let fold_compilation_context = compilation_context.with_additional_path([PathSegment::Fold]);
                    let compiled_fold = compile_with_context(fold, &fold_compilation_context, global_compilation_context)?;
                    let starting_with_compilation_context = compilation_context.with_additional_path([PathSegment::StartingWith]);
                    let compiled_starting_with = compile_with_context(starting_with, &starting_with_compilation_context, global_compilation_context)?;
                    let mut fold_constant_name_clustered_index = None;
                    let mut accumulating_in_constant_name_clustered_index = None;
                    let mut throughs = Vec::new();
                    let mut current_types_to_through_and_index: BTreeMap<(Type, Type), (Arc<Node>, usize)> = BTreeMap::new();
                    let (throughs_cycle_start_index, result_type) = match &compiled_fold.r#type.kind {
                        TypeKind::Tuple(fold_tuple_elements_types) => {
                            let mut current_accumulator_type = compiled_starting_with.r#type.clone();
                            for (current_index, current_type) in fold_tuple_elements_types.iter().enumerate() {
                                if let Some((through, _)) = current_types_to_through_and_index.get(&(current_type.clone(), current_accumulator_type.clone())) {
                                    current_accumulator_type = through.r#type.clone();
                                    throughs.push(through.clone());
                                } else {
                                    let mut through_compilation_context = compilation_context.with_additional_path([PathSegment::Through(current_index)]);
                                    fold_constant_name_clustered_index = Some(define_constant(
                                        r#as.clone(),
                                        current_type.clone(),
                                        &mut through_compilation_context,
                                        global_compilation_context,
                                    ));
                                    accumulating_in_constant_name_clustered_index = Some(define_constant(
                                        accumulating_in.clone(),
                                        current_accumulator_type.clone(),
                                        &mut through_compilation_context,
                                        global_compilation_context,
                                    ));
                                    let compiled_through = compile_with_context(through, &through_compilation_context, global_compilation_context)?;
                                    current_types_to_through_and_index
                                        .insert((current_type.clone(), current_accumulator_type.clone()), (compiled_through.clone(), current_index));
                                    current_accumulator_type = compiled_through.r#type.clone();
                                    throughs.push(compiled_through);
                                }
                            }
                            (0usize, current_accumulator_type)
                        }
                        TypeKind::Array(fold_array_element_type) => {
                            let current_type = *fold_array_element_type.clone();
                            let mut current_accumulator_type = compiled_starting_with.r#type.clone();
                            let mut current_index = 0usize;
                            let mut result_union_types = BTreeSet::new();
                            (
                                loop {
                                    if let Some((through, index)) = current_types_to_through_and_index.get(&(current_type.clone(), current_accumulator_type.clone())) {
                                        throughs.push(through.clone());
                                        break *index;
                                    } else {
                                        let mut through_compilation_context = compilation_context.with_additional_path([PathSegment::Through(current_index)]);
                                        let starting_with_type = compiled_starting_with.r#type.clone();
                                        fold_constant_name_clustered_index = Some(define_constant(
                                            r#as.clone(),
                                            *fold_array_element_type.clone(),
                                            &mut through_compilation_context,
                                            global_compilation_context,
                                        ));
                                        accumulating_in_constant_name_clustered_index = Some(define_constant(
                                            accumulating_in.clone(),
                                            starting_with_type.clone(),
                                            &mut through_compilation_context,
                                            global_compilation_context,
                                        ));
                                        let compiled_through = compile_with_context(through, &through_compilation_context, global_compilation_context)?;
                                        result_union_types.insert(compiled_through.r#type.clone());
                                        assert_type(&compiled_through.r#type, &starting_with_type.kind, &through_compilation_context)?;
                                        current_types_to_through_and_index
                                            .insert((current_type.clone(), current_accumulator_type.clone()), (compiled_through.clone(), current_index));
                                        result_union_types.insert(compiled_through.r#type.clone());
                                        current_accumulator_type = compiled_through.r#type.clone();
                                        throughs.push(compiled_through);
                                    }
                                    current_index += 1;
                                },
                                Type::from(result_union_types),
                            )
                        }
                        _ => {
                            return Err(fold_compilation_context.error(&compiled_fold.r#type, &"tuple or array"));
                        }
                    };
                    Node {
                        content: Content::Fold {
                            fold: compiled_fold,
                            fold_constant_name_clustered_index: fold_constant_name_clustered_index.unwrap(),
                            accumulating_in_constant_name_clustered_index: accumulating_in_constant_name_clustered_index.unwrap(),
                            starting_with: compiled_starting_with,
                            throughs,
                            throughs_cycle_start_index,
                        },
                        r#type: result_type,
                    }
                    .into()
                }
                Program::Metaprogram { metaprogram } => {
                    let metaprogram_compilation_context = compilation_context.with_additional_path([PathSegment::Metaprogram]);
                    let compiled_metaprogram = Arc::new(compile_program(metaprogram).with_context(|| metaprogram_compilation_context.error(&"", &"valid metaprogram"))?);
                    compile_with_context(
                        &serde_saphyr::from_str(&serde_saphyr::to_string(
                            &compute(&compiled_metaprogram).with_context(|| metaprogram_compilation_context.error(&"", &"computable metaprogram"))?,
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
                    let starting_with_compilation_context = compilation_context.with_additional_path([PathSegment::StartingWith]);
                    let compiled_starting_with = compile_with_context(starting_with, &starting_with_compilation_context, global_compilation_context)?;
                    let mut current_type = compiled_starting_with.r#type.clone();
                    let mut nexts = Vec::new();
                    let mut current_concrete_type_to_next_and_index = BTreeMap::new();
                    let mut current_constant_name_clustered_index;
                    let mut current_index = 0usize;
                    let mut result_union_types = BTreeSet::from_iter([compiled_starting_with.r#type.clone()]);
                    let nexts_cycle_start_index = loop {
                        let mut next_compilation_context = compilation_context.with_additional_path([PathSegment::Next(current_type.clone())]);
                        current_constant_name_clustered_index =
                            define_constant(r#as.clone(), current_type.clone(), &mut next_compilation_context, global_compilation_context);
                        let compiled_next = compile_with_context(next, &next_compilation_context, global_compilation_context)?;
                        let compiled_next_node = if compiled_next.r#type.kind.have_large_tuples(128) {
                            Node {
                                content: compiled_next.content.clone(),
                                r#type: compiled_next.r#type.with_kind(compiled_next.r#type.kind.clone().with_tuples_weakened_to_arrays(128)),
                            }
                            .into()
                        } else {
                            compiled_next
                        };
                        if let Some((_, nexts_cycle_start_index)) =
                            current_concrete_type_to_next_and_index.insert(std::mem::take(&mut current_type), (compiled_next_node.clone(), current_index))
                        {
                            break nexts_cycle_start_index;
                        } else {
                            nexts.push(compiled_next_node.clone());
                        }
                        result_union_types.insert(compiled_next_node.r#type.clone());
                        current_type = compiled_next_node.r#type.clone();
                        current_index += 1;
                    };
                    let element_type = Box::new(Type::from(result_union_types));
                    let intermediate_result_type = Type::from(TypeKind::Array(element_type.clone())).with_unified_properties(
                        &compiled_starting_with.r#type.properties.unified_all(current_concrete_type_to_next_and_index.keys().map(|r#type| &r#type.properties)),
                    );
                    let (compiled_while_node, result_type) = if let Some(r#while) = r#while {
                        let mut while_compilation_context = compilation_context.clone();
                        while_compilation_context.path.0.push(PathSegment::While);
                        define_constant(r#as.clone(), *element_type.clone(), &mut while_compilation_context, global_compilation_context);
                        let compiled_while = compile_with_context(r#while, &while_compilation_context, global_compilation_context)?;
                        assert_type(&compiled_while.r#type, &TypeKind::Bool, compilation_context)?;
                        let result_type = intermediate_result_type.with_unified_properties_from(&compiled_while.r#type);
                        (Some(compiled_while), result_type)
                    } else {
                        let mut result_type = intermediate_result_type;
                        result_type.properties.is_computable = false;
                        (None, result_type)
                    };
                    Node {
                        content: Content::Sequence(
                            intermediate_representation::Sequence {
                                starting_with: compiled_starting_with,
                                current_constant_name_clustered_index,
                                nexts,
                                nexts_cycle_start_index,
                                r#while: compiled_while_node,
                            }
                            .into(),
                        ),
                        r#type: result_type,
                    }
                    .into()
                }
                Program::Loop {
                    starting_with,
                    r#as,
                    r#loop,
                    r#while,
                } => {
                    let starting_with_compilation_context = compilation_context.with_additional_path([PathSegment::StartingWith]);
                    let compiled_starting_with = compile_with_context(starting_with, &starting_with_compilation_context, global_compilation_context)?;
                    let mut current_concrete_type = compiled_starting_with.r#type.clone();
                    let mut current_constant_name_clustered_index;
                    let mut nexts = Vec::new();
                    let mut current_concrete_type_to_next_and_index = BTreeMap::new();
                    let mut current_index = 0usize;
                    let nexts_cycle_start_index = loop {
                        let mut next_compilation_context = compilation_context.with_additional_path([PathSegment::Loop(current_concrete_type.clone())]);
                        current_constant_name_clustered_index =
                            define_constant(r#as.clone(), current_concrete_type.clone(), &mut next_compilation_context, global_compilation_context);
                        let compiled_next = compile_with_context(r#loop, &next_compilation_context, global_compilation_context)?;
                        let compiled_next_node = if compiled_next.r#type.kind.have_large_tuples(128) {
                            Node {
                                content: compiled_next.content.clone(),
                                r#type: compiled_next.r#type.with_kind(compiled_next.r#type.kind.clone().with_tuples_weakened_to_arrays(128)),
                            }
                            .into()
                        } else {
                            compiled_next
                        };
                        if let Some((_, nexts_cycle_start_index)) =
                            current_concrete_type_to_next_and_index.insert(std::mem::take(&mut current_concrete_type), (compiled_next_node.clone(), current_index))
                        {
                            break nexts_cycle_start_index;
                        } else {
                            nexts.push(compiled_next_node.clone());
                        }
                        current_concrete_type = compiled_next_node.r#type.clone();
                        current_index += 1;
                    };
                    let intermediate_result_type = Type::from(BTreeSet::from_iter(
                        [compiled_starting_with.r#type.clone()].into_iter().chain(current_concrete_type_to_next_and_index.keys().cloned()),
                    ))
                    .with_unified_properties(
                        &TypeProperties {
                            capabilities: EnumSet::default(),
                            is_computable: true,
                            is_serializable: true,
                            is_pure: true,
                        }
                        .unified(&compiled_starting_with.r#type.properties)
                        .unified_all(current_concrete_type_to_next_and_index.keys().map(|r#type| &r#type.properties)),
                    );
                    let (compiled_while_node, result_type) = if let Some(r#while) = r#while {
                        let mut while_compilation_context = compilation_context.clone();
                        while_compilation_context.path.0.push(PathSegment::While);
                        define_constant(
                            r#as.clone(),
                            intermediate_result_type.clone(),
                            &mut while_compilation_context,
                            global_compilation_context,
                        );
                        let compiled_while = compile_with_context(r#while, &while_compilation_context, global_compilation_context)?;
                        assert_type(&compiled_while.r#type, &TypeKind::Bool, compilation_context)?;
                        (Some(compiled_while), intermediate_result_type)
                    } else {
                        let mut result_type = intermediate_result_type.with_kind(TypeKind::Never);
                        result_type.properties.is_computable = false;
                        (None, result_type)
                    };
                    Node {
                        content: Content::Loop {
                            starting_with: compiled_starting_with,
                            current_constant_name_clustered_index,
                            nexts,
                            nexts_cycle_start_index,
                            r#while: compiled_while_node,
                        },
                        r#type: result_type,
                    }
                    .into()
                }
                Program::Pipe { pipe, r#as } => {
                    if pipe.is_empty() {
                        return Err(compilation_context.error(&[0; 0], &"non-empty pipe"));
                    }
                    let mut compiled_pipe_elements = Vec::with_capacity(pipe.len());
                    let mut compiled_previous_pipe_element_option: Option<Arc<Node>> = None;
                    let mut as_constant_name_clustered_index_option = None;
                    let mut r#type = None;
                    for (pipe_element_index, pipe_element) in pipe.iter().enumerate() {
                        let mut pipe_element_compilation_context =
                            compilation_context.with_additional_path([PathSegment::Pipe, PathSegment::ArrayIndex(pipe_element_index)]);
                        if let Some(r#as) = r#as
                            && let Some(compiled_previous_pipe_element) = compiled_previous_pipe_element_option
                        {
                            as_constant_name_clustered_index_option = Some(define_constant(
                                r#as.clone(),
                                compiled_previous_pipe_element.r#type.clone(),
                                &mut pipe_element_compilation_context,
                                global_compilation_context,
                            ));
                        }
                        let compiled_pipe_element = compile_with_context(pipe_element, &pipe_element_compilation_context, global_compilation_context)?;
                        compiled_previous_pipe_element_option = Some(compiled_pipe_element.clone());
                        if r#type.is_none() {
                            r#type = Some(compiled_pipe_element.r#type.clone());
                        } else {
                            r#type = Some(compiled_pipe_element.r#type.clone().with_unified_properties_from(&r#type.unwrap()))
                        }
                        compiled_pipe_elements.push(compiled_pipe_element);
                    }
                    Node {
                        content: Content::Pipe {
                            pipe: compiled_pipe_elements,
                            as_constant_name_clustered_index_option,
                        },
                        r#type: r#type.unwrap(),
                    }
                    .into()
                }
                Program::Try { r#try, or, r#as } => {
                    let mut try_compilation_context = compilation_context.clone();
                    try_compilation_context.path.0.push(PathSegment::Try);
                    let compiled_try = compile_with_context(r#try, &try_compilation_context, global_compilation_context)?;
                    if !compiled_try.r#type.properties.is_computable {
                        return Err(try_compilation_context.error(&"non-computable try", &"computable try"));
                    }
                    let mut or_compilation_context = compilation_context.clone();
                    or_compilation_context.path.0.push(PathSegment::Or);
                    let as_constant_name_clustered_index = define_constant(
                        r#as.clone(),
                        compiled_try.r#type.with_kind(TypeKind::String),
                        &mut or_compilation_context,
                        global_compilation_context,
                    );
                    let compiled_or = compile_with_context(or, &or_compilation_context, global_compilation_context)?;
                    let result_type = Type::from(BTreeSet::from_iter([compiled_try.r#type.clone(), compiled_or.r#type.clone()]));
                    Node {
                        content: Content::Try {
                            r#try: compiled_try,
                            or: compiled_or,
                            as_constant_name_clustered_index,
                        },
                        r#type: result_type,
                    }
                    .into()
                }
                Program::FunctionCall(function_call) => match &function_call.function {
                    Function::Embedded((embedded_function,)) => match embedded_function {
                        EmbeddedFunction::Sort => compile_embedded_function_call(
                            compilation_context,
                            global_compilation_context,
                            function_call,
                            Some(&|compiled_argument_resolved_type, _| {
                                if let TypeKind::Tuple(argument_elements_types) = &compiled_argument_resolved_type.kind {
                                    Ok(TypeKind::Tuple(
                                        std::iter::repeat_n(
                                            TypeKind::Union(BTreeSet::from_iter(argument_elements_types.iter().cloned()).into()).into(),
                                            argument_elements_types.len(),
                                        )
                                        .collect::<Vec<_>>()
                                        .into(),
                                    )
                                    .into())
                                } else {
                                    Ok(compiled_argument_resolved_type.clone())
                                }
                            }),
                        )?,
                        EmbeddedFunction::KeyValuePairs => compile_embedded_function_call(
                            compilation_context,
                            global_compilation_context,
                            function_call,
                            Some(&|compiled_argument_resolved_type, _| {
                                if let TypeKind::Object(argument_object_values_types) = &compiled_argument_resolved_type.kind {
                                    Ok(TypeKind::Tuple(
                                        argument_object_values_types
                                            .values()
                                            .map(|value| TypeKind::Tuple(vec![TypeKind::String.into(), value.clone()].into()).into())
                                            .collect::<Vec<_>>()
                                            .into(),
                                    )
                                    .into())
                                } else {
                                    Err(compilation_context.error(&compiled_argument_resolved_type, &"object"))
                                }
                            }),
                        )?,
                        EmbeddedFunction::ObjectFrom => compile_embedded_function_call(
                            compilation_context,
                            global_compilation_context,
                            function_call,
                            Some(&|compiled_argument_resolved_type, _| {
                                let argument_type_variants: Box<dyn Iterator<Item = &Type>> =
                                    if let TypeKind::Union(union_elements) = &compiled_argument_resolved_type.kind {
                                        Box::new(union_elements.iter())
                                    } else {
                                        Box::new(std::iter::once(compiled_argument_resolved_type))
                                    };
                                let mut result_union_types = BTreeSet::new();
                                for argument_type_variant in argument_type_variants {
                                    match &argument_type_variant.kind {
                                        TypeKind::Tuple(argument_elements_types) => {
                                            if argument_elements_types.iter().all(|key_and_value_tuple_type| {
                                                matches!(key_and_value_tuple_type.kind.as_tuple().unwrap()[0].kind, TypeKind::LiteralString(_))
                                            }) {
                                                result_union_types.insert(
                                                    TypeKind::Object(
                                                        BTreeMap::from_iter(argument_elements_types.iter().map(|key_and_value_tuple_type| {
                                                            (
                                                                key_and_value_tuple_type.kind.as_tuple().unwrap()[0].kind.as_literal_string().unwrap().to_string().into(),
                                                                key_and_value_tuple_type.kind.as_tuple().unwrap()[1].clone(),
                                                            )
                                                        }))
                                                        .into(),
                                                    )
                                                    .into(),
                                                );
                                            } else {
                                                result_union_types.insert(
                                                    TypeKind::GenericObject(Box::new(Type::from(BTreeSet::from_iter(
                                                        argument_elements_types
                                                            .iter()
                                                            .map(|key_and_value_tuple_type| key_and_value_tuple_type.kind.as_tuple().unwrap()[1].clone()),
                                                    ))))
                                                    .into(),
                                                );
                                            }
                                        }
                                        TypeKind::Array(argument_element_type) => {
                                            let argument_element_type_variants: Box<dyn Iterator<Item = &Type>> =
                                                if let TypeKind::Union(union_elements) = &argument_element_type.kind {
                                                    Box::new(union_elements.iter())
                                                } else {
                                                    Box::new(std::iter::once(&**argument_element_type))
                                                };
                                            for argument_element_type_variant in argument_element_type_variants {
                                                result_union_types
                                                    .insert(TypeKind::GenericObject(argument_element_type_variant.kind.as_tuple().unwrap()[1].clone().into()).into());
                                            }
                                        }
                                        _ => panic!(),
                                    };
                                }
                                Ok(Type::from(result_union_types))
                            }),
                        )?,
                        EmbeddedFunction::CompileRegex => compile_embedded_function_call(
                            compilation_context,
                            global_compilation_context,
                            function_call,
                            Some(&|compiled_argument_resolved_type, _| {
                                if let TypeKind::LiteralString(regex_literal_string) = &compiled_argument_resolved_type.kind {
                                    Regex::new(&regex_literal_string.to_string())
                                        .with_context(|| compilation_context.error(&regex_literal_string.to_string(), &"correct regex"))?;
                                };
                                Ok(TypeKind::Embedded(Embedded::CompiledRegex).into())
                            }),
                        )?,
                        EmbeddedFunction::Get => compile_embedded_function_call(
                            compilation_context,
                            global_compilation_context,
                            function_call,
                            Some(&|compiled_argument_resolved_type, variant_index| {
                                let at_type_kind = &compiled_argument_resolved_type.kind.as_object().unwrap().get(&"at".to_string()).unwrap().kind;
                                let from_type_kind = &compiled_argument_resolved_type.kind.as_object().unwrap().get(&"from".to_string()).unwrap().kind;
                                let error_type_properties = TypeProperties {
                                    capabilities: Capability::Error.into(),
                                    is_computable: true,
                                    is_serializable: true,
                                    is_pure: true,
                                };
                                match variant_index {
                                    0 => {
                                        let from_type_kind_variants: Vec<&TypeKind> = match &from_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&from_type_kind],
                                        };

                                        let at_type_kind_variants: Vec<&TypeKind> = match &at_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&at_type_kind],
                                        };
                                        let combinations =
                                            from_type_kind_variants.iter().flat_map(|&from| at_type_kind_variants.iter().map(move |&at| (from, at))).collect::<Vec<_>>();
                                        let mut result = BTreeSet::new();
                                        for (from_type_kind, at_type_kind) in combinations {
                                            match (from_type_kind, at_type_kind) {
                                                (TypeKind::Array(element_type), _) => {
                                                    result.insert(error_type_properties.unified_recursively(*element_type.clone()));
                                                }
                                                (TypeKind::Tuple(elements_types), TypeKind::Number) => {
                                                    result.insert(
                                                        error_type_properties.unified_recursively(Type::from(BTreeSet::from_iter(elements_types.iter().cloned()))),
                                                    );
                                                }
                                                (TypeKind::Tuple(elements_types), TypeKind::LiteralNumber(index_rational)) => {
                                                    let index = index_rational.trunc().to_f64().value() as usize;
                                                    if let Some(element_type) = elements_types.get(index) {
                                                        result.insert(Type::from(element_type.kind.clone()));
                                                    } else {
                                                        return Err(compilation_context.error(&elements_types, &format!("tuple with at least {} elements", index + 1)));
                                                    }
                                                }
                                                _ => panic!(),
                                            }
                                        }
                                        Ok(Type::from(result))
                                    }
                                    1 => {
                                        let at_range_from_type_kind = &at_type_kind.as_tuple().unwrap()[0].kind;
                                        let at_range_to_type_kind = &at_type_kind.as_tuple().unwrap()[1].kind;
                                        let from_type_kind_variants: Vec<&TypeKind> = match &from_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&from_type_kind],
                                        };

                                        let at_range_from_type_kind_variants: Vec<&TypeKind> = match &at_range_from_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&at_range_from_type_kind],
                                        };
                                        let at_range_to_type_kind_variants: Vec<&TypeKind> = match &at_range_to_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&at_range_to_type_kind],
                                        };
                                        let mut combinations = Vec::with_capacity(
                                            from_type_kind_variants.len() * at_range_from_type_kind_variants.len() * at_range_to_type_kind_variants.len(),
                                        );
                                        for from_type_kind_variant in &from_type_kind_variants {
                                            for at_range_from_type_kind_variant in &at_range_from_type_kind_variants {
                                                for at_range_to_type_kind_variant in &at_range_to_type_kind_variants {
                                                    combinations.push((*from_type_kind_variant, *at_range_from_type_kind_variant, *at_range_to_type_kind_variant));
                                                }
                                            }
                                        }
                                        let mut result = BTreeSet::new();
                                        for (from_type_kind, at_range_from_type_kind, at_range_to_type_kind) in combinations {
                                            match (from_type_kind, at_range_from_type_kind, at_range_to_type_kind) {
                                                (TypeKind::Array(element_type), TypeKind::Number, _) | (TypeKind::Array(element_type), _, TypeKind::Number) => {
                                                    result.insert(error_type_properties.unified_recursively(Type::from(TypeKind::Array(element_type.clone()))));
                                                }
                                                (
                                                    TypeKind::Array(element_type),
                                                    TypeKind::LiteralNumber(from_index_rational),
                                                    TypeKind::LiteralNumber(to_index_rational),
                                                ) => {
                                                    let from_index = from_index_rational.trunc().to_f64().value() as usize;
                                                    let to_index = to_index_rational.trunc().to_f64().value() as usize;
                                                    result.insert(
                                                        error_type_properties.unified_recursively(Type::from(vec![*element_type.clone(); to_index - from_index])),
                                                    );
                                                }
                                                (
                                                    TypeKind::Tuple(elements_types),
                                                    TypeKind::Number | TypeKind::LiteralNumber(_),
                                                    TypeKind::Number | TypeKind::LiteralNumber(_),
                                                ) => {
                                                    let from_index = at_range_from_type_kind
                                                        .as_literal_number()
                                                        .map(|from_index_rational| from_index_rational.trunc().to_f64().value() as usize)
                                                        .unwrap_or(0);
                                                    let to_index = at_range_to_type_kind
                                                        .as_literal_number()
                                                        .map(|to_index_rational| to_index_rational.trunc().to_f64().value() as usize)
                                                        .unwrap_or(elements_types.len());
                                                    if (from_index <= to_index) && (to_index <= elements_types.len()) {
                                                        result.insert(Type::from(BTreeSet::from_iter(elements_types[from_index..to_index].iter().cloned())));
                                                    } else {
                                                        return Err(compilation_context.error(&from_type_kind, &format!("tuple with range {from_index}..{to_index}",)));
                                                    }
                                                }
                                                _ => panic!(),
                                            }
                                        }
                                        Ok(Type::from(result))
                                    }
                                    2 => {
                                        let from_type_kind_variants: Vec<&TypeKind> = match &from_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&from_type_kind],
                                        };

                                        let at_type_kind_variants: Vec<&TypeKind> = match &at_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&at_type_kind],
                                        };
                                        let combinations =
                                            from_type_kind_variants.iter().flat_map(|&from| at_type_kind_variants.iter().map(move |&at| (from, at))).collect::<Vec<_>>();
                                        let mut result = BTreeSet::new();
                                        for (from_type_kind, at_type_kind) in combinations {
                                            match (from_type_kind, at_type_kind) {
                                                (TypeKind::GenericObject(element_type), _) => {
                                                    result.insert(error_type_properties.unified_recursively(*element_type.clone()));
                                                }
                                                (TypeKind::Object(inner_types), TypeKind::String | TypeKind::GenericLiteralString) => {
                                                    result.insert(
                                                        error_type_properties.unified_recursively(Type::from(BTreeSet::from_iter(inner_types.values().cloned()))),
                                                    );
                                                }
                                                (TypeKind::Object(inner_types), TypeKind::LiteralString(key_rope)) => {
                                                    let key = key_rope.to_string();
                                                    if let Some(element_type) = inner_types.get(&key) {
                                                        result.insert(Type::from(element_type.kind.clone()));
                                                    } else {
                                                        return Err(compilation_context.error(&inner_types, &format!("object with key {key:?}",)));
                                                    }
                                                }
                                                _ => panic!(),
                                            }
                                        }
                                        Ok(Type::from(result))
                                    }
                                    3 => {
                                        let at_range_from_type_kind = &at_type_kind.as_tuple().unwrap()[0].kind;
                                        let at_range_to_type_kind = &at_type_kind.as_tuple().unwrap()[1].kind;
                                        let from_type_kind_variants: Vec<&TypeKind> = match &from_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&from_type_kind],
                                        };

                                        let at_range_from_type_kind_variants: Vec<&TypeKind> = match &at_range_from_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&at_range_from_type_kind],
                                        };
                                        let at_range_to_type_kind_variants: Vec<&TypeKind> = match &at_range_to_type_kind {
                                            TypeKind::Union(variants) => variants.iter().map(|r#type| &r#type.kind).collect(),
                                            _ => vec![&at_range_to_type_kind],
                                        };
                                        let mut combinations = Vec::with_capacity(
                                            from_type_kind_variants.len() * at_range_from_type_kind_variants.len() * at_range_to_type_kind_variants.len(),
                                        );
                                        for from_type_kind_variant in &from_type_kind_variants {
                                            for at_range_from_type_kind_variant in &at_range_from_type_kind_variants {
                                                for at_range_to_type_kind_variant in &at_range_to_type_kind_variants {
                                                    combinations.push((*from_type_kind_variant, *at_range_from_type_kind_variant, *at_range_to_type_kind_variant));
                                                }
                                            }
                                        }
                                        let mut result = BTreeSet::new();
                                        for (from_type_kind, at_range_from_type_kind, at_range_to_type_kind) in combinations {
                                            match (from_type_kind, at_range_from_type_kind, at_range_to_type_kind) {
                                                (TypeKind::String, _, _)
                                                | (TypeKind::LiteralString(_), TypeKind::Number, _)
                                                | (TypeKind::LiteralString(_), _, TypeKind::Number) => {
                                                    result.insert(error_type_properties.unified_recursively(TypeKind::String.into()));
                                                }
                                                (
                                                    TypeKind::LiteralString(literal_string),
                                                    TypeKind::LiteralNumber(_) | TypeKind::Null,
                                                    TypeKind::LiteralNumber(_) | TypeKind::Null,
                                                ) => {
                                                    let from_index = at_range_from_type_kind
                                                        .as_literal_number()
                                                        .map(|from_index_rational| from_index_rational.trunc().to_f64().value() as usize)
                                                        .unwrap_or(0);
                                                    let to_index = at_range_to_type_kind
                                                        .as_literal_number()
                                                        .map(|to_index_rational| to_index_rational.trunc().to_f64().value() as usize)
                                                        .unwrap_or(literal_string.len_chars());
                                                    if (from_index <= to_index) && (to_index <= literal_string.len_chars()) {
                                                        result.insert(TypeKind::LiteralString(literal_string.slice(from_index..to_index).into()).into());
                                                    } else {
                                                        return Err(compilation_context.error(&from_type_kind, &format!("string with range {from_index}..{to_index}",)));
                                                    }
                                                }
                                                _ => panic!(),
                                            }
                                        }
                                        Ok(Type::from(result))
                                    }
                                    _ => panic!(),
                                }
                            }),
                        )?,
                        _ => compile_embedded_function_call(compilation_context, global_compilation_context, function_call, None)?,
                    },
                    Function::User(user_function_name) => {
                        if let Some(function_body_with_hash) = compilation_context.available_functions.get(user_function_name) {
                            if compilation_context.entered_user_functions.contains(&function_body_with_hash.hash) {
                                return Err(compilation_context.error(&user_function_name, &"non-recursive function call"));
                            }
                            let mut body_compilation_context = compilation_context.with_additional_path([PathSegment::UserFunctionCall(user_function_name.clone())]);
                            let arguments_iterator = match &function_call.argument.program {
                                Program::Object(function_arguments) => {
                                    let arguments_iterator: Box<dyn Iterator<Item = (Arc<String>, Arc<ProgramAndMetadata>)>> =
                                        Box::new(function_arguments.iter().map(|(key, value_with_hash)| (key.clone(), value_with_hash.clone())));
                                    arguments_iterator
                                }
                                _ => Box::new([(DEFAULT_ARGUMENT_NAME.clone(), function_call.argument.clone())].into_iter()),
                            };
                            let mut new_constants_definitions = Vec::new();
                            for (argument_name, argument) in arguments_iterator {
                                let argument_compilation_context = compilation_context.with_additional_path([
                                    PathSegment::UserFunctionCall(user_function_name.clone()),
                                    PathSegment::Argument(argument_name.clone()),
                                ]);
                                let compiled_constant = compile_with_context(&argument, &argument_compilation_context, global_compilation_context)?;
                                let constant_definition = define_constant(
                                    argument_name.clone(),
                                    compiled_constant.r#type.clone(),
                                    &mut body_compilation_context,
                                    global_compilation_context,
                                );
                                new_constants_definitions.push(intermediate_representation::ConstantDefinition {
                                    name_clustered_index: constant_definition,
                                    node: compiled_constant,
                                });
                            }
                            body_compilation_context.entered_user_functions.insert(function_body_with_hash.hash);
                            let compiled_function = compile_with_context(&function_body_with_hash, &body_compilation_context, global_compilation_context)?;
                            let function_index = global_compilation_context.user_functions_calls.len();
                            let r#type = compiled_function.r#type.clone();
                            global_compilation_context.user_functions_calls.push(compiled_function);
                            return Ok(Node {
                                content: Content::UserFunctionCall {
                                    arguments: new_constants_definitions,
                                    body: function_index,
                                },
                                r#type,
                            }
                            .into());
                        } else {
                            return Err(compilation_context.error(
                                &user_function_name,
                                &BTreeMap::from_iter([(
                                    &"one of available functions names",
                                    &compilation_context.available_functions.keys().collect::<Vec<_>>(),
                                )]),
                            ));
                        }
                    }
                },
                Program::Object(object) => match object.len() {
                    0 => {
                        return Ok(Node {
                            content: Content::Object(BTreeMap::new()),
                            r#type: TypeKind::Object(BTreeMap::new().into()).into(),
                        }
                        .into());
                    }
                    1.. => {
                        let mut result_inner_types = BTreeMap::new();
                        let mut result_content = BTreeMap::new();
                        for (object_key, object_value) in object.iter() {
                            let object_value_compilation_context = compilation_context.with_additional_path([PathSegment::ObjectKey(object_key.clone())]);
                            let compiled_object_value = compile_with_context(object_value, &object_value_compilation_context, global_compilation_context)?;
                            result_inner_types.insert(object_key.clone(), compiled_object_value.r#type.clone());
                            result_content.insert(object_key.clone(), compiled_object_value);
                        }
                        let result_inner_types_arc = Arc::new(result_inner_types);
                        Node {
                            content: Content::Object(result_content),
                            r#type: (TypeKind::Object(result_inner_types_arc.clone()), result_inner_types_arc.values()).into(),
                        }
                        .into()
                    }
                },
                _ => panic!(),
            };
            if result.r#type.kind.is_literal() && result.r#type.properties.is_pure {
                let literal_value = result.r#type.kind.literal_value();
                let r#type = Value::type_kind(&literal_value).into();
                Node {
                    content: Content::Value(unsafe { std::mem::transmute::<Arc<Option<Value>>, Arc<Option<intermediate_representation::Value>>>(literal_value.into()) }),
                    r#type,
                }
                .into()
            } else {
                result
            }
        }
    })
}
