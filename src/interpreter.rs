use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use anyhow::{Context, Error, Result, anyhow};

use crate::{
    clause::{AtSegment, Clause},
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    function::Function,
    includes_cache::IncludesCache,
    path::{Path, PathSegment},
    program::Program,
    program_with_includes::{IncludeFrom, ProgramWithIncludes},
    r#type::Type,
    value::{SmallMap, Value},
};

pub struct Interpreter {
    pub embedded_functions: BTreeMap<String, Function>,
}

#[derive(Debug, Clone)]
pub enum TypeOrValue {
    Type(Type),
    Value(Value),
}

#[derive(Debug)]
pub struct ListMap<V> {
    map: SmallMap<String, Vec<V>>,
}

impl<V> ListMap<V> {
    pub fn new() -> Self {
        Self {
            map: SmallMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.map
            .get(key)
            .and_then(|values_with_this_key| values_with_this_key.last())
    }

    pub fn push(&mut self, key: &str, value: V) {
        if let Some(values_with_this_key) = self.map.get_mut(key) {
            values_with_this_key.push(value);
        } else {
            self.map.insert(key.to_string(), vec![value]);
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.map.get_mut(key).unwrap().pop();
    }
}

#[derive(Debug)]
pub struct TypeCheckingContext {
    pub path: Path,
    pub functions: ListMap<Program>,
    pub constants: ListMap<Type>,
    pub entered_functions: BTreeSet<String>,
    pub recursed_functions_types: SmallMap<String, Type>,
}

impl TypeCheckingContext {
    pub fn error(&self, expected_type: &Type, got_type: &Type) -> Error {
        anyhow!(
            "Expected {expected_type:?} but got {got_type:?} at {:#?}",
            self.path,
        )
    }

    pub fn get_generic_arguments_values(
        &mut self,
        generic: &Type,
        actual: &Type,
    ) -> Result<[Option<Type>; 256]> {
        let mut result: [Option<Type>; 256] = std::array::from_fn(|_| None);
        self.get_generic_arguments_values_into_dict(generic, actual, &mut result)?;
        Ok(result)
    }

    pub fn get_generic_arguments_values_into_dict(
        &mut self,
        generic: &Type,
        actual: &Type,
        result: &mut [Option<Type>; 256],
    ) -> Result<()> {
        match (generic, actual) {
            (Type::GenericArgument(id), _) => {
                result[*id as usize] = Some(actual.clone());
            }
            (Type::RecursedFunction(recursed_function_name), actual) => {
                match self.recursed_functions_types[recursed_function_name].clone() {
                    Type::RecursedFunction(_) => {
                        self.recursed_functions_types
                            .insert(recursed_function_name.clone(), actual.clone());
                    }
                    inferred_recursed_function_type => {
                        if inferred_recursed_function_type != *actual {
                            return Err(self.error(&inferred_recursed_function_type, actual));
                        }
                    }
                }
            }
            (expected, Type::RecursedFunction(recursed_function_name)) => {
                match self.recursed_functions_types[recursed_function_name].clone() {
                    Type::RecursedFunction(_) => {
                        self.recursed_functions_types
                            .insert(recursed_function_name.clone(), expected.clone());
                    }
                    inferred_recursed_function_type => {
                        if inferred_recursed_function_type != *expected {
                            return Err(self.error(&inferred_recursed_function_type, expected));
                        }
                    }
                }
            }
            (Type::Object(generic_object_argument), Type::Object(actual_object_argument)) => {
                for (key, generic_value_type) in generic_object_argument {
                    self.get_generic_arguments_values_into_dict(
                        generic_value_type,
                        actual_object_argument
                            .get(key)
                            .ok_or_else(|| self.error(generic, actual))?,
                        result,
                    )
                    .with_context(|| self.error(generic, actual))?;
                }
            }
            (Type::Array(generic_array_argument), Type::Array(actual_array_argument)) => {
                self.get_generic_arguments_values_into_dict(
                    generic_array_argument,
                    actual_array_argument,
                    result,
                )
                .with_context(|| self.error(generic, actual))?;
            }
            (Type::Number, Type::Number) => {}
            (Type::String, Type::String) => {}
            (Type::Bool, Type::Bool) => {}
            (Type::Null, Type::Null) => {}
            (generic, actual) => return Err(self.error(generic, actual)),
        }
        Ok(())
    }

    pub fn substitute_generic_arguments_values(
        &self,
        generic: &mut Type,
        values: &[Option<Type>; 256],
    ) -> Result<()> {
        match generic {
            Type::GenericArgument(id) => {
                *generic = values.get(*id as usize).unwrap().clone().with_context(|| {
                    format!(
                        "Can not resolve generic argument {id:?} from other generic-actual types \
                         at {:?}",
                        self.path
                    )
                })?;
            }
            Type::Object(object) => {
                for value in object.values_mut() {
                    self.substitute_generic_arguments_values(value, values)?;
                }
            }
            Type::Array(element) => {
                self.substitute_generic_arguments_values(element, values)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn assert_equal(
        &mut self,
        expected_type: &Type,
        actual_type: &Type,
    ) -> Result<[Option<Type>; 256]> {
        let generic_values = self
            .get_generic_arguments_values(expected_type, actual_type)
            .with_context(|| {
                format!(
                    "Error while getting generic arguments values at {:?}",
                    self.path
                )
            })?;
        let concrete_expected_type = {
            let mut result = actual_type.clone();
            self.substitute_generic_arguments_values(&mut result, &generic_values)?;
            result
        };
        let concrete_actual_type = {
            let mut result = actual_type.clone();
            self.substitute_generic_arguments_values(&mut result, &generic_values)?;
            result
        };
        if concrete_actual_type != concrete_expected_type {
            Err(self.error(&concrete_expected_type, &concrete_actual_type))
        } else {
            Ok(generic_values)
        }
    }
}

#[derive(Clone, Debug)]
pub struct ComputationContext {
    pub path: Path,
    pub functions: rpds::RedBlackTreeMapSync<String, Program>,
    pub constants: rpds::RedBlackTreeMapSync<String, Value>,
}

impl ComputationContext {
    pub fn extended<P, F, C>(&self, path: P, functions: F, constants: C) -> Self
    where
        P: IntoIterator<Item = PathSegment>,
        F: IntoIterator<Item = (String, Program)>,
        C: IntoIterator<Item = (String, Value)>,
    {
        Self {
            path: {
                let mut result = self.path.clone();
                result.0.extend(path);
                result
            },
            functions: {
                let mut result = self.functions.clone();
                for function in functions {
                    result.insert_mut(function.0, function.1);
                }
                result
            },
            constants: {
                let mut result = self.constants.clone();
                for constant in constants {
                    result.insert_mut(constant.0, constant.1);
                }
                result
            },
        }
    }
}

impl Interpreter {
    pub fn compute(
        &self,
        program_with_includes: &ProgramWithIncludes,
        includes_cache: &mut IncludesCache,
    ) -> Result<Value> {
        let program =
            serde_json::from_value(self.process_includes(&program_with_includes, includes_cache)?)?;
        self.check_types(&program)?;
        Ok(self.compute_with_context(
            &program,
            ComputationContext {
                path: Path(rpds::VectorSync::new_sync()),
                functions: rpds::RedBlackTreeMapSync::new_sync(),
                constants: rpds::RedBlackTreeMapSync::new_sync(),
            },
        )?)
    }

    pub fn process_includes(
        &self,
        program_with_includes: &ProgramWithIncludes,
        includes_cache: &mut IncludesCache,
    ) -> Result<serde_json::Value> {
        match program_with_includes {
            ProgramWithIncludes::Include(include_clause) => {
                let mut result = self.process_includes(
                    &match &include_clause.include.from {
                        IncludeFrom::File(path) => match path.extension() {
                            Some(ext) if ext == "yaml" || ext == "yml" => {
                                serde_saphyr::from_reader(std::io::BufReader::new(
                                    std::fs::File::open(path.clone())?,
                                ))
                                .with_context(|| {
                                    format!("Can not parse included file at {path:?}")
                                })?
                            }
                            Some(ext) if ext == "json" => serde_json::from_reader(
                                std::io::BufReader::new(std::fs::File::open(path.clone())?),
                            )
                            .with_context(|| format!("Can not parse included file at {path:?}"))?,
                            extension => {
                                return Err(anyhow!(
                                    "Unsupported include file extension {extension:?} in file \
                                     path {path:?}"
                                ));
                            }
                        },
                        IncludeFrom::Url(url) => {
                            match std::path::Path::new(url.path())
                                .extension()
                                .and_then(std::ffi::OsStr::to_str)
                                .map(|extension| extension.to_lowercase())
                            {
                                Some(extension)
                                    if extension == "yaml"
                                        || extension == "yml"
                                        || extension == "json" =>
                                {
                                    let program_text = &includes_cache.get(url)?;
                                    match extension.as_str() {
                                        "yaml" | "yml" => serde_saphyr::from_str(program_text)
                                            .with_context(|| {
                                                format!(
                                                    "Can not parse included program downloaded \
                                                     from url {url:?}"
                                                )
                                            })?,
                                        "json" => serde_json::from_str(program_text).with_context(
                                            || {
                                                format!(
                                                    "Can not parse included program downloaded \
                                                     from url {url:?}"
                                                )
                                            },
                                        )?,
                                        _ => {
                                            return Err(anyhow!(
                                                "Unsupported extension {extension:?} for include \
                                                 file downloaded from url {url:?}"
                                            ));
                                        }
                                    }
                                }
                                extension => {
                                    return Err(anyhow!(
                                        "Unsupported include file extension {extension:?} in url \
                                         {url:?}"
                                    ));
                                }
                            }
                        }
                    },
                    includes_cache,
                )?;
                for path_segment in include_clause.include.at.iter() {
                    match path_segment {
                        AtSegment::ObjectKey(object_key) => {
                            if let Some(value) = result
                                .as_object_mut()
                                .with_context(|| {
                                    format!(
                                        "Can not get value by key {object_key:?} while processing \
                                         includes as it is not object"
                                    )
                                })?
                                .remove(object_key)
                            {
                                result = value;
                            } else {
                                return Err(anyhow!(
                                    "Can not get value by key {object_key:?} from {result:?} \
                                     while processing includes as it have no such key"
                                ));
                            }
                        }
                        AtSegment::ArrayIndex(array_index) => {
                            let vec = result.as_array_mut().with_context(|| {
                                format!(
                                    "Can not get element by index {array_index:?} while \
                                     processing includes as it is not object"
                                )
                            })?;
                            if vec.len() > *array_index {
                                result = vec.remove(*array_index);
                            } else {
                                return Err(anyhow!(
                                    "Can not get element by index {array_index:?} from {result:?} \
                                     while processing includes as it have no such index"
                                ));
                            }
                        }
                    }
                }
                Ok(result)
            }
            ProgramWithIncludes::Array(array) => {
                let mut result = vec![];
                for element in array {
                    result.push(self.process_includes(element, includes_cache)?);
                }
                Ok(serde_json::to_value(result)?)
            }
            ProgramWithIncludes::Object(object) => {
                let mut result = BTreeMap::new();
                for (key, value) in object {
                    result.insert(key, self.process_includes(value, includes_cache)?);
                }
                Ok(serde_json::to_value(result)?)
            }
            ProgramWithIncludes::Other(value) => Ok(value.clone()),
        }
    }

    fn compute_with_context(
        &self,
        program: &Program,
        context: ComputationContext,
    ) -> Result<Value> {
        Ok(match program {
            Program::Clause(clause) => match clause {
                Clause::DefaultArgument(_) => context
                    .constants
                    .get(DEFAULT_ARGUMENT_NAME)
                    .unwrap()
                    .clone(),
                Clause::Constant(constant_clause) => context
                    .constants
                    .get(&constant_clause.constant)
                    .unwrap()
                    .clone(),
                Clause::With(with_clause) => {
                    let constants_bodies_context =
                        context.extended([PathSegment::With, PathSegment::Constants], [], []);
                    let mut compute_context = context.extended(
                        [PathSegment::With, PathSegment::Compute],
                        with_clause
                            .with
                            .functions
                            .iter()
                            .map(|function_name_and_body| {
                                (
                                    function_name_and_body.0.clone(),
                                    function_name_and_body.1.clone(),
                                )
                            }),
                        [],
                    );
                    let complex_constants = with_clause
                        .with
                        .constants
                        .iter()
                        .filter(|keyvalue| match &keyvalue.1 {
                            Program::Value(value) => {
                                compute_context
                                    .constants
                                    .insert_mut(keyvalue.0.to_string(), value.clone());
                                false
                            }
                            _ => true,
                        })
                        .collect::<Vec<_>>();
                    self.compute_with_context(
                        &with_clause.compute,
                        match complex_constants.len() {
                            0 => compute_context,
                            1 => {
                                let key_and_value_compute_body =
                                    complex_constants.into_iter().next().unwrap();
                                compute_context.constants.insert_mut(
                                    key_and_value_compute_body.0.to_string(),
                                    self.compute_with_context(
                                        &key_and_value_compute_body.1,
                                        constants_bodies_context.extended(
                                            [PathSegment::Constant(
                                                key_and_value_compute_body.0.to_string(),
                                            )],
                                            [],
                                            [],
                                        ),
                                    )?,
                                );
                                compute_context
                            }
                            2.. => {
                                let compute_context_mutex = Mutex::new(compute_context.clone());
                                complex_constants
                                    .par_iter()
                                    .try_for_each(|key_and_value_compute_body| {
                                        self.compute_with_context(
                                            &key_and_value_compute_body.1,
                                            constants_bodies_context.extended(
                                                [PathSegment::Constant(
                                                    key_and_value_compute_body.0.to_string(),
                                                )],
                                                [],
                                                [],
                                            ),
                                        )
                                        .and_then(
                                            |result| {
                                                compute_context_mutex
                                                    .lock()
                                                    .unwrap()
                                                    .constants
                                                    .insert_mut(
                                                        key_and_value_compute_body.0.to_string(),
                                                        result,
                                                    );
                                                Ok(())
                                            },
                                        )
                                    })
                                    .and_then(|_| Ok(compute_context_mutex.into_inner().unwrap()))?
                            }
                        },
                    )?
                }
                Clause::Map(map_clause) => {
                    let precomputed_array = self
                        .compute_with_context(
                            &map_clause.map,
                            context.extended([PathSegment::Map], [], []),
                        )?
                        .as_array()
                        .unwrap()
                        .clone();
                    let map_context =
                        context.extended([PathSegment::Map, PathSegment::Through], [], []);
                    let result_mutex = Mutex::new(rpds::VectorSync::from_iter(
                        std::iter::repeat(Value::Null).take(precomputed_array.len()),
                    ));
                    precomputed_array
                        .into_iter()
                        .enumerate()
                        .par_bridge()
                        .try_for_each(|(element_index, precomputed_element)| {
                            self.compute_with_context(
                                &map_clause.through,
                                map_context.extended(
                                    [PathSegment::ArrayIndex(element_index)],
                                    [],
                                    [(map_clause.r#as.clone(), precomputed_element.clone())],
                                ),
                            )
                            .and_then(|result| {
                                result_mutex.lock().unwrap().set_mut(element_index, result);
                                Ok(())
                            })
                        })
                        .and_then(|_| Ok(Value::Array(result_mutex.into_inner().unwrap())))?
                }
                Clause::Filter(filter_clause) => {
                    let precomputed_array = self
                        .compute_with_context(
                            &filter_clause.filter,
                            context.extended([PathSegment::Filter], [], []),
                        )?
                        .as_array()
                        .unwrap()
                        .clone();
                    let filter_context =
                        context.extended([PathSegment::Filter, PathSegment::Through], [], []);
                    let result_mutex = Mutex::new(vec![None; precomputed_array.len()]);
                    precomputed_array
                        .into_iter()
                        .enumerate()
                        .par_bridge()
                        .try_for_each(|(element_index, precomputed_element)| {
                            self.compute_with_context(
                                &filter_clause.through,
                                filter_context.extended(
                                    [PathSegment::ArrayIndex(element_index)],
                                    [],
                                    [(filter_clause.r#as.clone(), precomputed_element.clone())],
                                ),
                            )
                            .and_then(|result| {
                                if result.as_bool().unwrap() {
                                    result_mutex.lock().unwrap()[element_index] =
                                        Some(precomputed_element.clone());
                                }
                                Ok(())
                            })
                        })
                        .and_then(|_| {
                            Ok(Value::Array(rpds::VectorSync::from_iter(
                                result_mutex
                                    .into_inner()
                                    .unwrap()
                                    .into_iter()
                                    .filter_map(|element| element),
                            )))
                        })?
                }
                Clause::Fold(fold_clause) => {
                    let array = self
                        .compute_with_context(&fold_clause.fold, context.clone())?
                        .as_array()
                        .unwrap()
                        .clone();
                    let mut result = self.compute_with_context(
                        &fold_clause.starting_with,
                        context.extended([PathSegment::StartingWith], [], []),
                    )?;
                    for (element_index, precomputed_element) in array.into_iter().enumerate() {
                        result = self.compute_with_context(
                            &fold_clause.through,
                            context.extended(
                                [
                                    PathSegment::Fold,
                                    PathSegment::ArrayIndex(element_index),
                                    PathSegment::Through,
                                ],
                                [],
                                [
                                    (fold_clause.r#as.clone(), precomputed_element.clone()),
                                    (fold_clause.accumulating_in.clone(), result),
                                ],
                            ),
                        )?;
                    }
                    result
                }
                Clause::Branching(branching_clause) => {
                    let if_result = self
                        .compute_with_context(
                            &branching_clause.r#if,
                            context.extended([PathSegment::If], [], []),
                        )?
                        .as_bool()
                        .unwrap();
                    let result = if if_result {
                        self.compute_with_context(
                            &branching_clause.then,
                            context.extended([PathSegment::Then], [], []),
                        )?
                    } else {
                        self.compute_with_context(
                            &branching_clause.r#else,
                            context.extended([PathSegment::Else], [], []),
                        )?
                    };
                    result
                }
                Clause::TryOr(try_or_clause) => {
                    let result = match self.compute_with_context(
                        &try_or_clause.r#try,
                        context.extended([PathSegment::Try], [], []),
                    ) {
                        Ok(result) => result,
                        Err(error) => self.compute_with_context(
                            &try_or_clause.or,
                            context.extended(
                                [PathSegment::Or],
                                [],
                                [(
                                    try_or_clause.with_error.clone(),
                                    Value::String(error.to_string()),
                                )],
                            ),
                        )?,
                    };
                    result
                }
                Clause::FromAt(from_at_clause) => {
                    let mut result = self.compute_with_context(
                        &from_at_clause.from,
                        context.extended([PathSegment::From], [], []),
                    )?;
                    for at_segment in from_at_clause.at.iter() {
                        result = match at_segment {
                            AtSegment::ObjectKey(object_key) => result
                                .as_object()
                                .unwrap()
                                .get(&*object_key)
                                .unwrap()
                                .clone(),
                            AtSegment::ArrayIndex(array_index) => {
                                let array = result.as_array().unwrap();
                                result
                                    .as_array()
                                    .unwrap()
                                    .get(*array_index)
                                    .with_context(|| {
                                        format!(
                                            "Can not get element at index {array_index} from \
                                             array of length {} at path segment {:?} of from-at \
                                             clause at {:?}",
                                            array.len(),
                                            array_index,
                                            context.path
                                        )
                                    })?
                                    .clone()
                            }
                        };
                    }
                    result
                }
            },
            Program::Object(object) => {
                if object.size() == 1 {
                    let (function_name, argument) = object.iter().next().unwrap();
                    if let Some(function_body) = context.functions.get(function_name).cloned() {
                        let arguments_bodies_context = context.extended(
                            [PathSegment::Function(function_name.clone())],
                            [],
                            [],
                        );
                        let mut compute_context = arguments_bodies_context.clone();
                        match argument {
                            Program::Object(arguments) if arguments.size() > 1 => {
                                for (argument_name, argument_compute_body) in arguments.iter() {
                                    compute_context.constants.insert_mut(
                                        argument_name.clone(),
                                        self.compute_with_context(
                                            argument_compute_body,
                                            arguments_bodies_context.extended(
                                                [PathSegment::Argument(argument_name.clone())],
                                                [],
                                                [],
                                            ),
                                        )?,
                                    );
                                }
                            }
                            Program::Value(value) => match value {
                                Value::Object(arguments) if arguments.size() > 1 => {
                                    for (argument_name, argument_value) in arguments.iter() {
                                        compute_context.constants.insert_mut(
                                            argument_name.clone(),
                                            argument_value.clone(),
                                        );
                                    }
                                }
                                _ => compute_context
                                    .constants
                                    .insert_mut(DEFAULT_ARGUMENT_NAME.to_string(), value.clone()),
                            },
                            _ => compute_context.constants.insert_mut(
                                DEFAULT_ARGUMENT_NAME.to_string(),
                                self.compute_with_context(argument, arguments_bodies_context)?,
                            ),
                        }
                        return self.compute_with_context(&function_body, compute_context);
                    } else if let Some(function) = self.embedded_functions.get(function_name) {
                        let function_argument = self.compute_with_context(
                            argument,
                            context.extended(
                                [PathSegment::Function(function_name.clone())],
                                [],
                                [],
                            ),
                        )?;
                        return (function.function)(function_argument);
                    }
                }
                let mut result = rpds::RedBlackTreeMapSync::new_sync();
                let complex_values = object
                    .iter()
                    .filter(|keyvalue| match &keyvalue.1 {
                        Program::Value(value) => {
                            result.insert_mut(keyvalue.0.to_string(), value.clone());
                            false
                        }
                        _ => true,
                    })
                    .collect::<Vec<_>>();
                match complex_values.len() {
                    0 => Value::Object(result),
                    1 => {
                        let (key, value_compute_body) = complex_values.into_iter().next().unwrap();
                        result.insert_mut(
                            key.to_string(),
                            self.compute_with_context(
                                value_compute_body,
                                context.extended([PathSegment::ObjectKey(key.to_string())], [], []),
                            )?,
                        );
                        Value::Object(result)
                    }
                    2.. => {
                        let result_mutex = Mutex::new(result);
                        complex_values
                            .par_iter()
                            .try_for_each(|(key, value_compute_body)| {
                                self.compute_with_context(
                                    value_compute_body,
                                    context.extended(
                                        [PathSegment::ObjectKey(key.to_string())],
                                        [],
                                        [],
                                    ),
                                )
                                .and_then(|result| {
                                    result_mutex
                                        .lock()
                                        .unwrap()
                                        .insert_mut(key.to_string(), result);
                                    Ok(())
                                })
                            })
                            .and_then(|_| Ok(Value::Object(result_mutex.into_inner().unwrap())))?
                    }
                }
            }
            Program::Array(array) => {
                let mut result =
                    rpds::VectorSync::from_iter(std::iter::repeat(Value::Null).take(array.len()));
                let complex_elements = array
                    .iter()
                    .enumerate()
                    .filter(|(element_index, element)| match &element {
                        Program::Value(value) => {
                            result.set_mut(*element_index, value.clone());
                            false
                        }
                        _ => true,
                    })
                    .collect::<Vec<_>>();
                match complex_elements.len() {
                    0 => Value::Array(result),
                    1 => {
                        let (element_index, element_compute_body) =
                            complex_elements.into_iter().next().unwrap();
                        result.set_mut(
                            element_index,
                            self.compute_with_context(
                                element_compute_body,
                                context.extended([PathSegment::ArrayIndex(element_index)], [], []),
                            )?,
                        );
                        Value::Array(result)
                    }
                    2.. => {
                        let result_mutex = Mutex::new(result);
                        complex_elements
                            .into_par_iter()
                            .try_for_each(|(element_index, element_compute_body)| {
                                self.compute_with_context(
                                    element_compute_body,
                                    context.extended(
                                        [PathSegment::ArrayIndex(element_index)],
                                        [],
                                        [],
                                    ),
                                )
                                .and_then(|result| {
                                    result_mutex.lock().unwrap().set_mut(element_index, result);
                                    Ok(())
                                })
                            })
                            .and_then(|_| Ok(Value::Array(result_mutex.into_inner().unwrap())))?
                    }
                }
            }
            Program::Value(value) => value.clone(),
        })
    }

    pub fn check_types(&self, program: &Program) -> Result<Type> {
        self.get_program_type(
            program,
            &mut TypeCheckingContext {
                path: Path(rpds::VectorSync::new_sync()),
                functions: ListMap::new(),
                constants: ListMap::new(),
                entered_functions: BTreeSet::new(),
                recursed_functions_types: SmallMap::new(),
            },
        )
    }

    fn get_value_type(&self, value: &Value, context: &mut TypeCheckingContext) -> Result<Type> {
        match value {
            Value::Array(array) => {
                if let Some(first_element) = array.first() {
                    context.path.0.push_back_mut(PathSegment::ArrayIndex(0));
                    let result = self.get_value_type(first_element, context)?;
                    context.path.0.drop_last_mut();
                    for (element_index, element) in array.iter().skip(1).enumerate() {
                        context
                            .path
                            .0
                            .push_back_mut(PathSegment::ArrayIndex(element_index));
                        let element_type = self.get_value_type(element, context)?;
                        if element_type != result {
                            return Err(context.error(&result, &element_type));
                        }
                        context.path.0.drop_last_mut();
                    }
                    Ok(result)
                } else {
                    Err(anyhow!("Expected non-empty array"))
                }
            }
            Value::Object(object) => {
                let mut result_map = BTreeMap::new();
                for (key, value) in object {
                    context
                        .path
                        .0
                        .push_back_mut(PathSegment::ObjectKey(key.clone()));
                    result_map.insert(key.clone(), self.get_value_type(value, context)?);
                    context.path.0.drop_last_mut();
                }
                Ok(Type::Object(result_map))
            }
            Value::String(_) => Ok(Type::String),
            Value::Number(_) => Ok(Type::Number),
            Value::Bool(_) => Ok(Type::Bool),
            Value::Null => Ok(Type::Null),
        }
    }

    fn get_program_type(
        &self,
        program: &Program,
        context: &mut TypeCheckingContext,
    ) -> Result<Type> {
        match program {
            Program::Clause(clause) => match clause {
                Clause::DefaultArgument(_) => context
                    .constants
                    .get(DEFAULT_ARGUMENT_NAME)
                    .cloned()
                    .with_context(|| {
                        format!(
                            "Unknown constant {:?} at {:#?}",
                            DEFAULT_ARGUMENT_NAME, context.path
                        )
                    }),
                Clause::Constant(constant_clause) => context
                    .constants
                    .get(&constant_clause.constant)
                    .cloned()
                    .with_context(|| {
                        format!(
                            "Unknown constant {:?} at {:#?}",
                            constant_clause.constant, context.path
                        )
                    }),
                Clause::With(with_clause) => {
                    for function_name_and_body in with_clause.with.functions.iter() {
                        context
                            .functions
                            .push(&function_name_and_body.0, function_name_and_body.1.clone());
                    }
                    context.path.0.push_back_mut(PathSegment::With);
                    context.path.0.push_back_mut(PathSegment::Constants);
                    for constant_name_and_compute_body in with_clause.with.constants.iter() {
                        context.path.0.push_back_mut(PathSegment::Constant(
                            constant_name_and_compute_body.0.clone(),
                        ));
                        let precomputed_constant_type = self
                            .get_program_type(&constant_name_and_compute_body.1.clone(), context)?;
                        context.path.0.drop_last_mut();
                        context
                            .constants
                            .push(&constant_name_and_compute_body.0, precomputed_constant_type);
                    }
                    context.path.0.drop_last_mut();
                    context.path.0.push_back_mut(PathSegment::Compute);
                    let result = self.get_program_type(&with_clause.compute.clone(), context)?;
                    context.path.0.drop_last_mut();
                    context.path.0.drop_last_mut();
                    for function_name_and_compute_body in with_clause.with.functions.iter() {
                        context.functions.remove(&function_name_and_compute_body.0);
                    }
                    for function_name_and_compute_body in with_clause.with.constants.iter() {
                        context.constants.remove(&function_name_and_compute_body.0);
                    }
                    Ok(result)
                }
                Clause::Map(map_clause) => {
                    context.path.0.push_back_mut(PathSegment::Map);
                    let actual_array_type =
                        self.get_program_type(&map_clause.map.clone(), context)?;
                    context.path.0.drop_last_mut();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        context
                            .constants
                            .push(&map_clause.r#as, *array_element_type.clone());
                        context.path.0.push_back_mut(PathSegment::Through);
                        let result = self.get_program_type(&map_clause.through, context)?;
                        context.path.0.drop_last_mut();
                        context.constants.remove(&map_clause.r#as);
                        Ok(Type::Array(Box::new(result)))
                    } else {
                        Err(anyhow!(
                            "Expected array for map clause at {:?}, got {actual_array_type:?}",
                            context.path
                        ))
                    }
                }
                Clause::Filter(filter_clause) => {
                    context.path.0.push_back_mut(PathSegment::Filter);
                    let actual_array_type =
                        self.get_program_type(&filter_clause.filter, context)?;
                    context.path.0.drop_last_mut();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        context
                            .constants
                            .push(&filter_clause.r#as, *array_element_type.clone());
                        context.path.0.push_back_mut(PathSegment::Through);
                        let through_type =
                            self.get_program_type(&filter_clause.through, context)?;
                        context.path.0.drop_last_mut();
                        context
                            .assert_equal(&through_type, &Type::Bool)
                            .with_context(|| {
                                anyhow!(
                                    "Expected filter at {:?} to use function which returns \
                                     boolean value, but it returns {through_type:?}",
                                    context.path
                                )
                            })?;
                        context.constants.remove(&filter_clause.r#as);
                        Ok(Type::Array(array_element_type.clone()))
                    } else {
                        Err(anyhow!(
                            "Expected array for filter clause at {:?}, got {actual_array_type:?}",
                            context.path
                        ))
                    }
                }
                Clause::Fold(fold_clause) => {
                    context.path.0.push_back_mut(PathSegment::Fold);
                    let actual_array_type = self.get_program_type(&fold_clause.fold, context)?;
                    context.path.0.drop_last_mut();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        let starting_with_type =
                            self.get_program_type(&fold_clause.starting_with, context)?;
                        context
                            .constants
                            .push(&fold_clause.r#as, *array_element_type.clone());
                        context
                            .constants
                            .push(&fold_clause.accumulating_in, starting_with_type.clone());
                        context.path.0.push_back_mut(PathSegment::Through);
                        let through_type = self.get_program_type(&fold_clause.through, context)?;
                        context.path.0.drop_last_mut();
                        context
                            .assert_equal(&through_type, &starting_with_type)
                            .with_context(|| {
                                anyhow!(
                                    "Expected fold at {:?} to use function which returns value \
                                     {starting_with_type:?} (as is starting value), but it \
                                     returns {through_type:?}",
                                    context.path
                                )
                            })?;
                        context.constants.remove(&fold_clause.r#as);
                        context.constants.remove(&fold_clause.accumulating_in);
                        Ok(through_type)
                    } else {
                        Err(anyhow!(
                            "Expected array for fold clause at {:?}, got {actual_array_type:?}",
                            context.path
                        ))
                    }
                }
                Clause::Branching(branching_clause) => {
                    context.path.0.push_back_mut(PathSegment::If);
                    let if_branch_type = self.get_program_type(&branching_clause.r#if, context)?;
                    context.assert_equal(&Type::Bool, &if_branch_type)?;
                    context.path.0.drop_last_mut();
                    context.path.0.push_back_mut(PathSegment::Then);
                    let then_branch_type =
                        self.get_program_type(&branching_clause.then, context)?;
                    context.path.0.drop_last_mut();
                    context.path.0.push_back_mut(PathSegment::Else);
                    let else_branch_type =
                        self.get_program_type(&branching_clause.r#else, context)?;
                    context.path.0.drop_last_mut();
                    context
                        .assert_equal(&then_branch_type, &else_branch_type)
                        .with_context(|| {
                            anyhow!(
                                "Expected 'then' and 'else' branches at {:?} to be of the same \
                                 type, but 'then' branch is {then_branch_type:?} and 'else' \
                                 branch is {else_branch_type:?}",
                                context.path
                            )
                        })?;
                    Ok(then_branch_type)
                }
                Clause::TryOr(try_or_clause) => {
                    context.path.0.push_back_mut(PathSegment::Try);
                    let try_branch_type = self.get_program_type(&try_or_clause.r#try, context)?;
                    context.path.0.drop_last_mut();
                    context.path.0.push_back_mut(PathSegment::Or);
                    context
                        .constants
                        .push(&try_or_clause.with_error, Type::String);
                    let or_branch_type = self.get_program_type(&try_or_clause.or, context)?;
                    context.path.0.drop_last_mut();
                    context.constants.remove(&try_or_clause.with_error);
                    context
                        .assert_equal(&try_branch_type, &or_branch_type)
                        .with_context(|| {
                            anyhow!(
                                "Expected 'try' and 'or' branches at {:?} to be of the same type, \
                                 but 'try' branch is {try_branch_type:?} and 'or' branch is \
                                 {or_branch_type:?}",
                                context.path
                            )
                        })?;
                    Ok(try_branch_type)
                }
                Clause::FromAt(from_at_clause) => {
                    context.path.0.push_back_mut(PathSegment::From);
                    let mut result = self.get_program_type(&from_at_clause.from, context)?;
                    context.path.0.drop_last_mut();
                    context.path.0.push_back_mut(PathSegment::At);
                    if from_at_clause.at.is_empty() {
                        return Err(anyhow!("Expected a non-empty list at {:?}", context.path));
                    }
                    for (at_segment_index, at_segment) in from_at_clause.at.iter().enumerate() {
                        context
                            .path
                            .0
                            .push_back_mut(PathSegment::AtIndex(at_segment_index));
                        match at_segment {
                            AtSegment::ObjectKey(object_key) => match result {
                                Type::Object(mut result_fields_types) => {
                                    if let Some(result_field_type) =
                                        result_fields_types.remove(object_key)
                                    {
                                        result = result_field_type;
                                    } else {
                                        return Err(anyhow!(
                                            "Expected to reach from 'from' to be an object with \
                                             key {object_key:?} at the point {:?}, but it has no \
                                             such key",
                                            context.path
                                        ));
                                    }
                                }
                                r#type => {
                                    return Err(anyhow!(
                                        "Expected to reach from 'from' to be an object at the \
                                         point {:?}, but it is {type:?}",
                                        context.path
                                    ));
                                }
                            },
                            AtSegment::ArrayIndex(_) => match result {
                                Type::Array(element_type) => {
                                    result = *element_type;
                                }
                                r#type => {
                                    return Err(anyhow!(
                                        "Expected to reach from 'from' to be an array at the \
                                         point {:?}, but it is {type:?}",
                                        context.path
                                    ));
                                }
                            },
                        }
                        context.path.0.drop_last_mut();
                    }
                    context.path.0.drop_last_mut();
                    Ok(result)
                }
            },
            Program::Object(object) => {
                if object.size() == 1 {
                    let (function_name, argument) = object.iter().next().unwrap();
                    if let Some(function_body) = context.functions.get(function_name).cloned() {
                        context
                            .path
                            .0
                            .push_back_mut(PathSegment::Function(function_name.clone()));
                        if context.entered_functions.contains(function_name) {
                            if let Some(this_recursed_function_type) =
                                context.recursed_functions_types.get(function_name)
                            {
                                return Ok(this_recursed_function_type.clone());
                            } else {
                                context.recursed_functions_types.insert(
                                    function_name.clone(),
                                    Type::RecursedFunction(function_name.clone()),
                                );
                            }
                        }
                        let mut arguments_names = vec![];
                        match argument {
                            Program::Object(arguments) if arguments.size() > 1 => {
                                for (argument_name, argument_compute_body) in arguments.iter() {
                                    context.path.0.push_back_mut(PathSegment::Argument(
                                        argument_name.clone(),
                                    ));
                                    let argument_type =
                                        self.get_program_type(&argument_compute_body, context)?;
                                    context.path.0.drop_last_mut();
                                    arguments_names.push(argument_name.clone());
                                    context.constants.push(&argument_name, argument_type);
                                }
                            }
                            Program::Value(value) => match value {
                                Value::Object(arguments) if arguments.size() > 1 => {
                                    for (argument_name, argument_value) in arguments.iter() {
                                        context.path.0.push_back_mut(PathSegment::Argument(
                                            argument_name.clone(),
                                        ));
                                        let argument_type =
                                            self.get_value_type(argument_value, context)?;
                                        context.path.0.drop_last_mut();
                                        arguments_names.push(argument_name.clone());
                                        context.constants.push(&argument_name, argument_type);
                                    }
                                }
                                _ => {
                                    let value_type = self.get_value_type(value, context)?;
                                    context.constants.push(DEFAULT_ARGUMENT_NAME, value_type);
                                }
                            },
                            argument_compute_body => {
                                let argument_type =
                                    self.get_program_type(argument_compute_body, context)?;
                                arguments_names.push(DEFAULT_ARGUMENT_NAME.to_string());
                                context.constants.push(DEFAULT_ARGUMENT_NAME, argument_type);
                            }
                        };
                        context.entered_functions.insert(function_name.clone());
                        let result = self.get_program_type(&function_body, context)?;
                        context.path.0.drop_last_mut();
                        context.entered_functions.remove(function_name);
                        for argument_name in arguments_names {
                            context.constants.remove(&argument_name);
                            context.recursed_functions_types.remove(&argument_name);
                        }
                        return Ok(result);
                    }
                    if let Some(function) = self.embedded_functions.get(function_name) {
                        context
                            .path
                            .0
                            .push_back_mut(PathSegment::EmbeddedFunction(function_name.clone()));
                        let arguments_type = self.get_program_type(argument, context)?;
                        let generic_values =
                            context.assert_equal(&function.argument_type, &arguments_type)?;
                        context.path.0.drop_last_mut();
                        let mut result = function.return_type.clone();
                        context
                            .substitute_generic_arguments_values(&mut result, &generic_values)?;
                        return Ok(result);
                    }
                }
                let mut result_map = BTreeMap::new();
                for (key, value) in object {
                    context
                        .path
                        .0
                        .push_back_mut(PathSegment::ObjectKey(key.clone()));
                    result_map.insert(key.clone(), self.get_program_type(value, context)?);
                    context.path.0.drop_last_mut();
                }
                Ok(Type::Object(result_map))
            }
            Program::Array(array) => {
                let mut non_recursed_elements_indexes_and_types = Vec::with_capacity(array.len());
                let mut recursed_elements_functions_names = vec![];
                for (element_index, element) in array.iter().enumerate() {
                    context
                        .path
                        .0
                        .push_back_mut(PathSegment::ArrayIndex(element_index));
                    match self.get_program_type(element, context)? {
                        Type::RecursedFunction(recursed_function_name) => {
                            recursed_elements_functions_names.push(recursed_function_name);
                        }
                        non_recursed_type => {
                            non_recursed_elements_indexes_and_types
                                .push((element_index, non_recursed_type));
                        }
                    }
                    context.path.0.drop_last_mut();
                }
                if let Some(first_non_recursed_element_type) =
                    non_recursed_elements_indexes_and_types
                        .first()
                        .and_then(|(_, element_type)| Some(element_type))
                {
                    if let Some((unexpected_type_element_index, unexpected_type)) =
                        non_recursed_elements_indexes_and_types
                            .iter()
                            .find(|(_, element_type)| {
                                element_type != first_non_recursed_element_type
                            })
                    {
                        context
                            .path
                            .0
                            .push_back_mut(PathSegment::ArrayIndex(*unexpected_type_element_index));
                        let result_error =
                            context.error(first_non_recursed_element_type, unexpected_type);
                        context.path.0.drop_last_mut();
                        return Err(result_error);
                    } else {
                        Ok(Type::Array(Box::new(
                            first_non_recursed_element_type.clone(),
                        )))
                    }
                } else if let Some(first_recursed_element_function_name) =
                    recursed_elements_functions_names.first()
                {
                    Ok(Type::Array(Box::new(Type::RecursedFunction(
                        first_recursed_element_function_name.clone(),
                    ))))
                } else {
                    Err(anyhow!("Expected non-empty array at {:?}", context.path))
                }
            }
            Program::Value(value) => self.get_value_type(value, context),
        }
    }
}
