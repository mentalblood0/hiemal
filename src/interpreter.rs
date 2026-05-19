use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use anyhow::{Context, Error, Result, anyhow};

use crate::{
    default_argument_name::DEFAULT_ARGUMENT_NAME,
    function::Function,
    includes_cache::IncludesCache,
    path::{Path, PathSegment},
    r#type::Type,
    value::{AtSegment, Include, RcOrValue, SmallMap, Value, ValueWithIncludes},
};

pub struct Interpreter {
    pub embedded_functions: BTreeMap<String, Function>,
}

#[derive(Debug, Clone)]
pub enum TypeOrRcOrValue {
    Type(Type),
    RcOrValue(RcOrValue),
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
    pub functions: ListMap<Rc<Value>>,
    pub constants: ListMap<TypeOrRcOrValue>,
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
    pub path: rpds::Vector<PathSegment>,
    pub functions: rpds::RedBlackTreeMap<String, Rc<Value>>,
    pub constants: rpds::RedBlackTreeMap<String, RcOrValue>,
}

impl ComputationContext {
    pub fn extended<P, F, C>(&self, path: P, functions: F, constants: C) -> Self
    where
        P: IntoIterator<Item = PathSegment>,
        F: IntoIterator<Item = (String, Rc<Value>)>,
        C: IntoIterator<Item = (String, RcOrValue)>,
    {
        Self {
            path: {
                let mut result = self.path.clone();
                result.extend(path);
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
        program_with_includes: &ValueWithIncludes,
        includes_cache: &mut IncludesCache,
    ) -> Result<Rc<Value>> {
        let program: Rc<Value> = Rc::new(serde_json::from_value(
            self.process_includes(&program_with_includes, includes_cache)?,
        )?);
        self.check_types(program.clone())?;
        Ok(
            match self.compute_with_context(
                &RcOrValue::Rc(program),
                &ComputationContext {
                    path: rpds::Vector::new(),
                    functions: rpds::RedBlackTreeMap::new(),
                    constants: rpds::RedBlackTreeMap::new(),
                },
            )? {
                RcOrValue::Rc(rc) => rc,
                RcOrValue::Value(value) => Rc::new(value),
            },
        )
    }

    pub fn process_includes(
        &self,
        program_with_includes: &ValueWithIncludes,
        includes_cache: &mut IncludesCache,
    ) -> Result<serde_json::Value> {
        match program_with_includes {
            ValueWithIncludes::Include(include_clause) => self.process_includes(
                &match include_clause {
                    Include::IncludeFile(path) => match path.extension() {
                        Some(ext) if ext == "yaml" || ext == "yml" => serde_saphyr::from_reader(
                            std::io::BufReader::new(std::fs::File::open(path.clone())?),
                        )
                        .with_context(|| format!("Can not parse included file at {path:?}"))?,
                        Some(ext) if ext == "json" => serde_json::from_reader(
                            std::io::BufReader::new(std::fs::File::open(path.clone())?),
                        )
                        .with_context(|| format!("Can not parse included file at {path:?}"))?,
                        extension => {
                            return Err(anyhow!(
                                "Unsupported include file extension {extension:?} in file path \
                                 {path:?}"
                            ));
                        }
                    },
                    Include::IncludeUrl(url) => {
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
                                                "Can not parse included program downloaded from \
                                                 url {url:?}"
                                            )
                                        })?,
                                    "json" => {
                                        serde_json::from_str(program_text).with_context(|| {
                                            format!(
                                                "Can not parse included program downloaded from \
                                                 url {url:?}"
                                            )
                                        })?
                                    }
                                    _ => {
                                        return Err(anyhow!(
                                            "Unsupported extension {extension:?} for include file \
                                             downloaded from url {url:?}"
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
            ),
            ValueWithIncludes::Array(array) => {
                let mut result = vec![];
                for element in array {
                    result.push(self.process_includes(element, includes_cache)?);
                }
                Ok(serde_json::to_value(result)?)
            }
            ValueWithIncludes::Object(object) => {
                let mut result = BTreeMap::new();
                for (key, value) in object {
                    result.insert(key, self.process_includes(value, includes_cache)?);
                }
                Ok(serde_json::to_value(result)?)
            }
            ValueWithIncludes::Other(value) => Ok(value.clone()),
        }
    }

    fn compute_with_context(
        &self,
        program: &RcOrValue,
        context: &ComputationContext,
    ) -> Result<RcOrValue> {
        Ok(match program.value() {
            Value::Constant(constant_clause) => context
                .constants
                .get(&constant_clause.constant)
                .unwrap()
                .clone(),
            Value::With(with_clause) => {
                let constants_bodies_context =
                    context.extended([PathSegment::With, PathSegment::Constants], [], []);
                let mut compute_context = context.extended(
                    [PathSegment::With, PathSegment::Compute],
                    with_clause
                        .with
                        .functions
                        .iter()
                        .map(|(function_name, function_body)| {
                            (function_name.clone(), function_body.clone())
                        }),
                    [],
                );
                for (constant_name, constant_compute_body) in with_clause.with.constants.iter() {
                    let precomputed_value = self.compute_with_context(
                        &constant_compute_body,
                        &constants_bodies_context.extended(
                            [PathSegment::Constant(constant_name.clone())],
                            [],
                            [],
                        ),
                    )?;
                    compute_context
                        .constants
                        .insert_mut(constant_name.clone(), precomputed_value);
                }
                let result = self.compute_with_context(&with_clause.compute, &compute_context)?;
                result
            }
            Value::Map(map_clause) => {
                let array = self
                    .compute_with_context(&map_clause.map, context)?
                    .as_array()
                    .unwrap()
                    .clone();
                let mut result = Vec::with_capacity(array.len());
                let elements_bodies_context = context.extended([PathSegment::Map], [], []);
                for (element_index, element_compute_body) in array.into_iter().enumerate() {
                    let element_body_context = elements_bodies_context.extended(
                        [PathSegment::ArrayIndex(element_index)],
                        [],
                        [],
                    );
                    let precomputed_element =
                        self.compute_with_context(&element_compute_body, &element_body_context)?;
                    result.push(
                        self.compute_with_context(
                            &map_clause.through,
                            &ComputationContext {
                                path: element_body_context.path.push_back(PathSegment::Through),
                                functions: element_body_context.functions,
                                constants: element_body_context
                                    .constants
                                    .insert(map_clause.r#as.clone(), precomputed_element),
                            },
                        )?,
                    );
                }
                RcOrValue::Value(Value::Array(result))
            }
            Value::Filter(filter_clause) => {
                let array = self
                    .compute_with_context(&filter_clause.filter, context)?
                    .as_array()
                    .unwrap()
                    .clone();
                let mut result = Vec::with_capacity(array.len());
                let elements_bodies_context = context.extended([PathSegment::Filter], [], []);
                for (element_index, element_compute_body) in array.into_iter().enumerate() {
                    let element_body_context = elements_bodies_context.extended(
                        [PathSegment::ArrayIndex(element_index)],
                        [],
                        [],
                    );
                    let precomputed_element =
                        self.compute_with_context(&element_compute_body, &element_body_context)?;
                    if self
                        .compute_with_context(
                            &filter_clause.through,
                            &ComputationContext {
                                path: element_body_context.path.push_back(PathSegment::Through),
                                functions: element_body_context.functions,
                                constants: element_body_context.constants.insert(
                                    filter_clause.r#as.clone(),
                                    precomputed_element.clone(),
                                ),
                            },
                        )?
                        .as_bool()
                        .unwrap()
                    {
                        result.push(precomputed_element);
                    };
                }
                RcOrValue::Value(Value::Array(result))
            }
            Value::Fold(fold_clause) => {
                let array = self
                    .compute_with_context(&fold_clause.fold, context)?
                    .as_array()
                    .unwrap()
                    .clone();
                let mut local_context = context.clone();
                local_context.path.push_back_mut(PathSegment::StartingWith);
                let mut result = self.compute_with_context(&fold_clause.starting_with, context)?;
                local_context.path.drop_last_mut();
                local_context.path.push_back_mut(PathSegment::Fold);
                for (element_index, element_compute_body) in array.into_iter().enumerate() {
                    local_context
                        .path
                        .push_back_mut(PathSegment::ArrayIndex(element_index));
                    let precomputed_element =
                        self.compute_with_context(&element_compute_body, &local_context)?;
                    local_context
                        .constants
                        .insert_mut(fold_clause.r#as.clone(), precomputed_element);
                    local_context
                        .constants
                        .insert_mut(fold_clause.accumulating_in.clone(), result);
                    local_context.path.push_back_mut(PathSegment::Through);
                    result = self.compute_with_context(&fold_clause.through, &local_context)?;
                    local_context.constants.remove_mut(&fold_clause.r#as);
                    local_context
                        .constants
                        .remove_mut(&fold_clause.accumulating_in);
                    local_context.path.drop_last_mut();
                    local_context.path.drop_last_mut();
                }
                result
            }
            Value::Branching(branching_clause) => {
                let mut local_context = context.clone();
                local_context.path.push_back_mut(PathSegment::If);
                let if_result = self
                    .compute_with_context(&branching_clause.r#if, &local_context)?
                    .as_bool()
                    .unwrap();
                let result = if if_result {
                    local_context.path.drop_last_mut();
                    local_context.path.push_back_mut(PathSegment::Then);
                    self.compute_with_context(&branching_clause.then, &local_context)?
                } else {
                    local_context.path.drop_last_mut();
                    local_context.path.push_back_mut(PathSegment::Else);
                    self.compute_with_context(&branching_clause.r#else, &local_context)?
                };
                result
            }
            Value::TryOr(try_or_clause) => {
                let mut local_context = context.clone();
                local_context.path.push_back_mut(PathSegment::Try);
                let result = match self.compute_with_context(&try_or_clause.r#try, &local_context) {
                    Ok(result) => result,
                    Err(error) => {
                        local_context.constants.insert_mut(
                            try_or_clause.with_error.clone(),
                            RcOrValue::Value(Value::String(error.to_string())),
                        );
                        self.compute_with_context(&try_or_clause.or, &local_context)?
                    }
                };
                result
            }
            Value::FromAt(from_at_clause) => {
                let mut local_context = context.clone();
                local_context.path.push_back_mut(PathSegment::From);
                let mut result = self.compute_with_context(&from_at_clause.from, &local_context)?;
                local_context.path.drop_last_mut();
                local_context.path.push_back_mut(PathSegment::At);
                for (at_segment_index, at_segment) in from_at_clause.at.iter().enumerate() {
                    local_context
                        .path
                        .push_back_mut(PathSegment::AtIndex(at_segment_index));
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
                                        "Can not get element at index {array_index} from array of \
                                         length {} at the point {:?}",
                                        array.len(),
                                        context.path
                                    )
                                })?
                                .clone()
                        }
                    };
                    local_context.path.drop_last_mut();
                }
                result
            }
            Value::Object(object) => {
                if object.len() == 1 {
                    let (function_name, argument) = object.iter().next().unwrap();
                    let arguments_bodies_context =
                        context.extended([PathSegment::Function(function_name.clone())], [], []);
                    if let Some(function_body) = context.functions.get(function_name).cloned() {
                        let mut compute_context = arguments_bodies_context.clone();
                        if let Value::Object(arguments) = argument.value()
                            && arguments.len() > 1
                        {
                            for (argument_name, argument_compute_body) in arguments.iter() {
                                compute_context.constants.insert_mut(
                                    argument_name.clone(),
                                    self.compute_with_context(
                                        argument_compute_body,
                                        &arguments_bodies_context.extended(
                                            [PathSegment::Argument(argument_name.clone())],
                                            [],
                                            [],
                                        ),
                                    )?,
                                );
                            }
                        } else {
                            compute_context.constants.insert_mut(
                                DEFAULT_ARGUMENT_NAME.to_string(),
                                self.compute_with_context(argument, &arguments_bodies_context)?,
                            );
                        }
                        return self
                            .compute_with_context(&RcOrValue::Rc(function_body), &compute_context);
                    } else if let Some(function) = self.embedded_functions.get(function_name) {
                        let function_arguments =
                            self.compute_with_context(&argument, &arguments_bodies_context)?;
                        return (function.function)(function_arguments);
                    }
                }
                let mut result_object_keyvalues = vec![None; object.len()];
                for (keyvalue_index, (key, value_compute_body)) in object.iter().enumerate() {
                    result_object_keyvalues[keyvalue_index] = Some((
                        key.clone(),
                        self.compute_with_context(
                            value_compute_body,
                            &context.extended([PathSegment::ObjectKey(key.clone())], [], []),
                        )?,
                    ));
                }
                RcOrValue::Value(Value::Object(BTreeMap::from_iter(
                    result_object_keyvalues
                        .into_iter()
                        .filter_map(|element| element),
                )))
            }
            Value::Array(array) => {
                let mut result_array_elements = Vec::with_capacity(array.len());
                for (element_index, element_compute_body) in array.iter().enumerate() {
                    result_array_elements.push(self.compute_with_context(
                        element_compute_body,
                        &context.extended([PathSegment::ArrayIndex(element_index)], [], []),
                    )?);
                }
                RcOrValue::Value(Value::Array(result_array_elements))
            }
            Value::String(string) => {
                if string == DEFAULT_ARGUMENT_NAME {
                    context
                        .constants
                        .get(DEFAULT_ARGUMENT_NAME)
                        .unwrap()
                        .clone()
                } else {
                    RcOrValue::Value(Value::String(string.clone()))
                }
            }
            Value::Number(number) => RcOrValue::Value(Value::Number(number.clone())),
            Value::Bool(bool) => RcOrValue::Value(Value::Bool(*bool)),
            Value::Null => RcOrValue::Value(Value::Null),
        })
    }

    pub fn check_types(&self, program: Rc<Value>) -> Result<Type> {
        self.get_type(
            TypeOrRcOrValue::RcOrValue(RcOrValue::Rc(program)),
            &mut TypeCheckingContext {
                path: Path(vec![]),
                functions: ListMap::new(),
                constants: ListMap::new(),
                entered_functions: BTreeSet::new(),
                recursed_functions_types: SmallMap::new(),
            },
        )
    }

    fn get_type(
        &self,
        program: TypeOrRcOrValue,
        context: &mut TypeCheckingContext,
    ) -> Result<Type> {
        let result = match program {
            TypeOrRcOrValue::Type(program_type) => program_type,
            TypeOrRcOrValue::RcOrValue(program) => match *program.value() {
                Value::Constant(ref constant_clause) => {
                    if let Some(constant_value) = context.constants.get(&constant_clause.constant) {
                        context
                            .path
                            .0
                            .push(PathSegment::Constant(constant_clause.constant.clone()));
                        let result = self.get_type(constant_value.clone(), context)?;
                        context.path.0.pop();
                        result
                    } else {
                        return Err(anyhow!(
                            "Unknown constant {:?} at {:#?}",
                            constant_clause.constant,
                            context.path
                        ));
                    }
                }
                Value::With(ref with_clause) => {
                    for (function_name, function_body) in with_clause.with.functions.iter() {
                        context
                            .functions
                            .push(&function_name, function_body.clone());
                    }
                    context.path.0.push(PathSegment::With);
                    context.path.0.push(PathSegment::Constants);
                    for (constant_name, constant_compute_body) in with_clause.with.constants.iter()
                    {
                        context
                            .path
                            .0
                            .push(PathSegment::Constant(constant_name.clone()));
                        let precomputed_constant_type = self.get_type(
                            TypeOrRcOrValue::RcOrValue(constant_compute_body.clone()),
                            context,
                        )?;
                        context.path.0.pop();
                        context.constants.push(
                            &constant_name,
                            TypeOrRcOrValue::Type(precomputed_constant_type),
                        );
                    }
                    context.path.0.pop();
                    context.path.0.push(PathSegment::Compute);
                    let result = self.get_type(
                        TypeOrRcOrValue::RcOrValue(with_clause.compute.clone()),
                        context,
                    )?;
                    context.path.0.pop();
                    context.path.0.pop();
                    for function_name in with_clause.with.functions.keys() {
                        context.functions.remove(function_name);
                    }
                    for constant_name in with_clause.with.constants.keys() {
                        context.constants.remove(constant_name);
                    }
                    result
                }
                Value::Map(ref map_clause) => {
                    context.path.0.push(PathSegment::Map);
                    let actual_array_type =
                        self.get_type(TypeOrRcOrValue::RcOrValue(map_clause.map.clone()), context)?;
                    context.path.0.pop();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        context.constants.push(
                            &map_clause.r#as,
                            TypeOrRcOrValue::Type(*array_element_type.clone()),
                        );
                        context.path.0.push(PathSegment::Through);
                        let result = self.get_type(
                            TypeOrRcOrValue::RcOrValue(map_clause.through.clone()),
                            context,
                        )?;
                        context.path.0.pop();
                        context.constants.remove(&map_clause.r#as);
                        Type::Array(Box::new(result))
                    } else {
                        return Err(anyhow!(
                            "Expected array for map clause at {:?}, got {actual_array_type:?}",
                            context.path
                        ));
                    }
                }
                Value::Filter(ref filter_clause) => {
                    context.path.0.push(PathSegment::Filter);
                    let actual_array_type = self.get_type(
                        TypeOrRcOrValue::RcOrValue(filter_clause.filter.clone()),
                        context,
                    )?;
                    context.path.0.pop();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        context.constants.push(
                            &filter_clause.r#as,
                            TypeOrRcOrValue::Type(*array_element_type.clone()),
                        );
                        context.path.0.push(PathSegment::Through);
                        let through_type = self.get_type(
                            TypeOrRcOrValue::RcOrValue(filter_clause.through.clone()),
                            context,
                        )?;
                        context.path.0.pop();
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
                        Type::Array(array_element_type.clone())
                    } else {
                        return Err(anyhow!(
                            "Expected array for filter clause at {:?}, got {actual_array_type:?}",
                            context.path
                        ));
                    }
                }
                Value::Fold(ref fold_clause) => {
                    context.path.0.push(PathSegment::Fold);
                    let actual_array_type = self.get_type(
                        TypeOrRcOrValue::RcOrValue(fold_clause.fold.clone()),
                        context,
                    )?;
                    context.path.0.pop();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        let starting_with_type = self.get_type(
                            TypeOrRcOrValue::RcOrValue(fold_clause.starting_with.clone()),
                            context,
                        )?;
                        context.constants.push(
                            &fold_clause.r#as,
                            TypeOrRcOrValue::Type(*array_element_type.clone()),
                        );
                        context.constants.push(
                            &fold_clause.accumulating_in,
                            TypeOrRcOrValue::Type(starting_with_type.clone()),
                        );
                        context.path.0.push(PathSegment::Through);
                        let through_type = self.get_type(
                            TypeOrRcOrValue::RcOrValue(fold_clause.through.clone()),
                            context,
                        )?;
                        context.path.0.pop();
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
                        Type::Array(Box::new(through_type))
                    } else {
                        return Err(anyhow!(
                            "Expected array for fold clause at {:?}, got {actual_array_type:?}",
                            context.path
                        ));
                    }
                }
                Value::Branching(ref branching_clause) => {
                    context.path.0.push(PathSegment::If);
                    let if_branch_type = self.get_type(
                        TypeOrRcOrValue::RcOrValue(branching_clause.r#if.clone()),
                        context,
                    )?;
                    context.assert_equal(&Type::Bool, &if_branch_type)?;
                    *context.path.0.last_mut().unwrap() = PathSegment::Then;
                    let then_branch_type = self.get_type(
                        TypeOrRcOrValue::RcOrValue(branching_clause.then.clone()),
                        context,
                    )?;
                    *context.path.0.last_mut().unwrap() = PathSegment::Else;
                    let else_branch_type = self.get_type(
                        TypeOrRcOrValue::RcOrValue(branching_clause.r#else.clone()),
                        context,
                    )?;
                    context.path.0.pop();
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
                    then_branch_type
                }
                Value::TryOr(ref try_or_clause) => {
                    context.path.0.push(PathSegment::If);
                    let try_branch_type = self.get_type(
                        TypeOrRcOrValue::RcOrValue(try_or_clause.r#try.clone()),
                        context,
                    )?;
                    *context.path.0.last_mut().unwrap() = PathSegment::Or;
                    context.constants.push(
                        &try_or_clause.with_error,
                        TypeOrRcOrValue::Type(Type::String),
                    );
                    let or_branch_type = self.get_type(
                        TypeOrRcOrValue::RcOrValue(try_or_clause.or.clone()),
                        context,
                    )?;
                    context.path.0.pop();
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
                    try_branch_type
                }
                Value::FromAt(ref from_at_clause) => {
                    context.path.0.push(PathSegment::From);
                    let mut result = self.get_type(
                        TypeOrRcOrValue::RcOrValue(from_at_clause.from.clone()),
                        context,
                    )?;
                    *context.path.0.last_mut().unwrap() = PathSegment::At;
                    if from_at_clause.at.is_empty() {
                        return Err(anyhow!("Expected a non-empty list at {:?}", context.path));
                    }
                    for (at_segment_index, at_segment) in from_at_clause.at.iter().enumerate() {
                        context.path.0.push(PathSegment::AtIndex(at_segment_index));
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
                        context.path.0.pop();
                    }
                    context.path.0.pop();
                    result
                }
                Value::Object(ref object) => {
                    if object.len() == 1 {
                        let (function_name, argument) = object.iter().next().unwrap();
                        if let Some(function_body) = context.functions.get(function_name).cloned() {
                            context
                                .path
                                .0
                                .push(PathSegment::Function(function_name.clone()));
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
                            if let Value::Object(arguments) = argument.value()
                                && arguments.len() > 1
                            {
                                for (argument_name, argument_compute_body) in arguments.iter() {
                                    context
                                        .path
                                        .0
                                        .push(PathSegment::Argument(argument_name.clone()));
                                    let argument_type = self.get_type(
                                        TypeOrRcOrValue::RcOrValue(argument_compute_body.clone()),
                                        context,
                                    )?;
                                    context.path.0.pop();
                                    arguments_names.push(argument_name.clone());
                                    context
                                        .constants
                                        .push(&argument_name, TypeOrRcOrValue::Type(argument_type));
                                }
                            } else {
                                let argument_type = self.get_type(
                                    TypeOrRcOrValue::RcOrValue(argument.clone()),
                                    context,
                                )?;
                                arguments_names.push(DEFAULT_ARGUMENT_NAME.to_string());
                                context.constants.push(
                                    DEFAULT_ARGUMENT_NAME,
                                    TypeOrRcOrValue::Type(argument_type),
                                );
                            }
                            context.entered_functions.insert(function_name.clone());
                            let result = self.get_type(
                                TypeOrRcOrValue::RcOrValue(RcOrValue::Rc(function_body.clone())),
                                context,
                            )?;
                            context.path.0.pop();
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
                                .push(PathSegment::EmbeddedFunction(function_name.clone()));
                            let arguments_type = self
                                .get_type(TypeOrRcOrValue::RcOrValue(argument.clone()), context)?;
                            let generic_values =
                                context.assert_equal(&function.argument_type, &arguments_type)?;
                            context.path.0.pop();
                            let mut result = function.return_type.clone();
                            context.substitute_generic_arguments_values(
                                &mut result,
                                &generic_values,
                            )?;
                            return Ok(result);
                        }
                    }
                    let mut result_map = BTreeMap::new();
                    for (key, value) in object {
                        context.path.0.push(PathSegment::ObjectKey(key.clone()));
                        result_map.insert(
                            key.clone(),
                            self.get_type(TypeOrRcOrValue::RcOrValue(value.clone()), context)?,
                        );
                        context.path.0.pop();
                    }
                    Type::Object(result_map)
                }
                Value::Array(ref array) => {
                    let mut non_recursed_elements_indexes_and_types =
                        Vec::with_capacity(array.len());
                    let mut recursed_elements_functions_names = vec![];
                    for (element_index, element) in array.iter().enumerate() {
                        context.path.0.push(PathSegment::ArrayIndex(element_index));
                        match self.get_type(TypeOrRcOrValue::RcOrValue(element.clone()), context)? {
                            Type::RecursedFunction(recursed_function_name) => {
                                recursed_elements_functions_names.push(recursed_function_name);
                            }
                            non_recursed_type => {
                                non_recursed_elements_indexes_and_types
                                    .push((element_index, non_recursed_type));
                            }
                        }
                        context.path.0.pop();
                    }
                    if let Some(first_non_recursed_element_type) =
                        non_recursed_elements_indexes_and_types
                            .first()
                            .and_then(|(_, element_type)| Some(element_type))
                    {
                        if let Some((unexpected_type_element_index, unexpected_type)) =
                            non_recursed_elements_indexes_and_types.iter().find(
                                |(_, element_type)| element_type != first_non_recursed_element_type,
                            )
                        {
                            context
                                .path
                                .0
                                .push(PathSegment::ArrayIndex(*unexpected_type_element_index));
                            let result_error =
                                context.error(first_non_recursed_element_type, unexpected_type);
                            context.path.0.pop();
                            return Err(result_error);
                        } else {
                            Type::Array(Box::new(first_non_recursed_element_type.clone()))
                        }
                    } else if let Some(first_recursed_element_function_name) =
                        recursed_elements_functions_names.first()
                    {
                        Type::Array(Box::new(Type::RecursedFunction(
                            first_recursed_element_function_name.clone(),
                        )))
                    } else {
                        return Err(anyhow!("Expected non-empty array at {:?}", context.path));
                    }
                }
                Value::String(ref string) => {
                    if string == DEFAULT_ARGUMENT_NAME
                        && let Some(constant) = context.constants.get(DEFAULT_ARGUMENT_NAME)
                    {
                        self.get_type(constant.clone(), context)?
                    } else {
                        Type::String
                    }
                }
                Value::Number(_) => Type::Number,
                Value::Bool(_) => Type::Bool,
                Value::Null => Type::Null,
            },
        };
        Ok(result)
    }
}
