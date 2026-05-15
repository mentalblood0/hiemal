use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use anyhow::{anyhow, Context, Error, Result};

use crate::function::Function;
use crate::includes_cache::IncludesCache;
use crate::path::{Path, PathSegment};
use crate::r#type::Type;
use crate::value::{AtSegment, Include, RcOrValue, SmallMap, Value, ValueWithIncludes};

pub struct Interpreter {
    pub supported_functions: BTreeMap<String, Function>,
}

#[derive(Clone, Debug)]
pub enum TypeOrRcOrValue {
    Type(Type),
    RcOrValue(RcOrValue),
}

impl TypeOrRcOrValue {
    pub fn clone_rc_if_complex_otherwise_type_or_value(&self) -> Self {
        match self {
            TypeOrRcOrValue::Type(_) => self.clone(),
            TypeOrRcOrValue::RcOrValue(rc_or_value) => match rc_or_value {
                RcOrValue::Rc(rc) => TypeOrRcOrValue::RcOrValue(RcOrValue::Rc(rc.clone())),
                RcOrValue::Value(value) => match value {
                    Value::Number(_) | Value::String(_) | Value::Bool(_) | Value::Null => {
                        TypeOrRcOrValue::RcOrValue(RcOrValue::Value(value.clone()))
                    }
                    complex_value => {
                        TypeOrRcOrValue::RcOrValue(RcOrValue::Rc(Rc::new(complex_value.clone())))
                    }
                },
            },
        }
    }
}

#[derive(Debug)]
pub struct TypeCheckingContext {
    pub path: Path,
    pub aliases: SmallMap<String, Vec<TypeOrRcOrValue>>,
    pub entered_aliases: BTreeSet<String>,
    pub recursed_aliases_types: SmallMap<String, Type>,
}

impl TypeCheckingContext {
    pub fn add_alias(&mut self, name: &str, type_or_rc_or_value: TypeOrRcOrValue) {
        if let Some(aliases_with_this_name) = self.aliases.get_mut(name) {
            aliases_with_this_name.push(type_or_rc_or_value);
        } else {
            self.aliases
                .insert(name.to_string(), vec![type_or_rc_or_value]);
        }
    }

    pub fn remove_alias(&mut self, name: &String) {
        self.aliases.get_mut(name).unwrap().pop();
    }

    pub fn error(&self, expected_type: &Type, got_type: &Type) -> Error {
        anyhow!(
            "Expected value {expected_type:?} but got value {got_type:?} at path {:?}",
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
            (Type::RecursedAlias(recursed_alias_name), actual) => {
                match self.recursed_aliases_types[recursed_alias_name].clone() {
                    Type::RecursedAlias(_) => {
                        self.recursed_aliases_types
                            .insert(recursed_alias_name.clone(), actual.clone());
                    }
                    inferred_recursed_alias_type => {
                        if inferred_recursed_alias_type != *actual {
                            return Err(self.error(&inferred_recursed_alias_type, actual));
                        }
                    }
                }
            }
            (expected, Type::RecursedAlias(recursed_alias_name)) => {
                match self.recursed_aliases_types[recursed_alias_name].clone() {
                    Type::RecursedAlias(_) => {
                        self.recursed_aliases_types
                            .insert(recursed_alias_name.clone(), expected.clone());
                    }
                    inferred_recursed_alias_type => {
                        if inferred_recursed_alias_type != *expected {
                            return Err(self.error(&inferred_recursed_alias_type, expected));
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
                         at path {:?}",
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
                    "Error while getting generic arguments values at path {:?}",
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

const DEFAULT_ALIAS: &str = "_";

pub struct ComputationContext {
    pub path: Path,
    pub aliases: SmallMap<String, Vec<RcOrValue>>,
}

impl ComputationContext {
    pub fn add_alias(&mut self, name: &str, rc_or_value: RcOrValue) {
        if let Some(aliases_with_this_name) = self.aliases.get_mut(name) {
            aliases_with_this_name.push(rc_or_value);
        } else {
            self.aliases.insert(name.to_string(), vec![rc_or_value]);
        }
    }

    pub fn remove_alias(&mut self, name: &String) {
        self.aliases.get_mut(name).unwrap().pop();
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
                &mut ComputationContext {
                    path: Path(vec![]),
                    aliases: SmallMap::new(),
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
                        .with_context(|| format!("Can not parse included file at path {path:?}"))?,
                        Some(ext) if ext == "json" => serde_json::from_reader(
                            std::io::BufReader::new(std::fs::File::open(path.clone())?),
                        )
                        .with_context(|| format!("Can not parse included file at path {path:?}"))?,
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
        context: &mut ComputationContext,
    ) -> Result<RcOrValue> {
        Ok(match program.value() {
            Value::With(with_clause) => {
                for (alias_name, alias_value) in with_clause.with.definitions.iter() {
                    context.add_alias(
                        &alias_name,
                        alias_value.clone_rc_if_complex_otherwise_value(),
                    );
                }
                context.path.0.push(PathSegment::With);
                context.path.0.push(PathSegment::Constants);
                for (alias_name, alias_value) in with_clause.with.constants.iter() {
                    context.path.0.push(PathSegment::Alias(alias_name.clone()));
                    let precomputed_value = self.compute_with_context(&alias_value, context)?;
                    context.path.0.pop();
                    context.add_alias(&alias_name, precomputed_value);
                }
                *context.path.0.last_mut().unwrap() = PathSegment::Compute;
                let result = self.compute_with_context(&with_clause.compute, context)?;
                context.path.0.pop();
                context.path.0.pop();
                for alias_name in with_clause.with.definitions.keys() {
                    context.remove_alias(alias_name);
                }
                for alias_name in with_clause.with.constants.keys() {
                    context.remove_alias(alias_name);
                }
                result
            }
            Value::Map(map_clause) => {
                let array = self
                    .compute_with_context(&map_clause.map, context)?
                    .as_array()
                    .unwrap()
                    .clone();
                let mut result = vec![];
                context.path.0.push(PathSegment::Map);
                for (element_index, element) in array.into_iter().enumerate() {
                    context.add_alias(&map_clause.as_alias, element);
                    context.path.0.push(PathSegment::ArrayIndex(element_index));
                    context.path.0.push(PathSegment::Through);
                    result.push(self.compute_with_context(&map_clause.through, context)?);
                    context.path.0.pop();
                    context.path.0.pop();
                    context.remove_alias(&map_clause.as_alias);
                }
                context.path.0.pop();
                RcOrValue::Value(Value::Array(result))
            }
            Value::Filter(filter_clause) => {
                let array = self
                    .compute_with_context(&filter_clause.filter, context)?
                    .as_array()
                    .unwrap()
                    .clone();
                let mut result = vec![];
                context.path.0.push(PathSegment::Filter);
                for (element_index, element) in array.into_iter().enumerate() {
                    context.add_alias(
                        &filter_clause.as_alias,
                        element.clone_rc_if_complex_otherwise_value(),
                    );
                    context.path.0.push(PathSegment::ArrayIndex(element_index));
                    context.path.0.push(PathSegment::Through);
                    if self
                        .compute_with_context(&filter_clause.through, context)?
                        .as_bool()
                        .unwrap()
                    {
                        result.push(element);
                    }
                    context.path.0.pop();
                    context.path.0.pop();
                    context.remove_alias(&filter_clause.as_alias);
                }
                context.path.0.pop();
                RcOrValue::Value(Value::Array(result))
            }
            Value::Fold(fold_clause) => {
                let array = self
                    .compute_with_context(&fold_clause.fold, context)?
                    .as_array()
                    .unwrap()
                    .clone();
                context.path.0.push(PathSegment::StartingWith);
                let mut result = self.compute_with_context(&fold_clause.starting_with, context)?;
                *context.path.0.last_mut().unwrap() = PathSegment::Fold;
                for (element_index, element) in array.into_iter().enumerate() {
                    context.add_alias(&fold_clause.as_alias, element);
                    context.add_alias(&fold_clause.accumulating_in_alias, result);
                    context.path.0.push(PathSegment::ArrayIndex(element_index));
                    context.path.0.push(PathSegment::Through);
                    result = self.compute_with_context(&fold_clause.through, context)?;
                    context.path.0.pop();
                    context.path.0.pop();
                    context.remove_alias(&fold_clause.as_alias);
                    context.remove_alias(&fold_clause.accumulating_in_alias);
                }
                context.path.0.pop();
                result
            }
            Value::Branching(branching_clause) => {
                context.path.0.push(PathSegment::If);
                let if_result = self
                    .compute_with_context(&branching_clause.r#if, context)?
                    .as_bool()
                    .unwrap();
                let result = if if_result {
                    *context.path.0.last_mut().unwrap() = PathSegment::Then;
                    self.compute_with_context(&branching_clause.then, context)?
                } else {
                    *context.path.0.last_mut().unwrap() = PathSegment::Else;
                    self.compute_with_context(&branching_clause.r#else, context)?
                };
                context.path.0.pop();
                result
            }
            Value::TryOr(try_or_clause) => {
                context.path.0.push(PathSegment::Try);
                let result = match self.compute_with_context(&try_or_clause.r#try, context) {
                    Ok(result) => result,
                    Err(error) => {
                        context.add_alias(
                            &try_or_clause.with_error_alias,
                            RcOrValue::Value(Value::String(error.to_string())),
                        );
                        self.compute_with_context(&try_or_clause.or, context)?
                    }
                };
                context.path.0.pop();
                result
            }
            Value::FromAt(from_at_clause) => {
                context.path.0.push(PathSegment::From);
                let mut result = self.compute_with_context(&from_at_clause.from, context)?;
                *context.path.0.last_mut().unwrap() = PathSegment::At;
                for (at_segment_index, at_segment) in from_at_clause.at.iter().enumerate() {
                    context.path.0.push(PathSegment::AtIndex(at_segment_index));
                    result = match at_segment {
                        AtSegment::ObjectKey(object_key) => result
                            .as_object()
                            .unwrap()
                            .get(&*object_key)
                            .unwrap()
                            .clone_rc_if_complex_otherwise_value(),
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
                                .clone_rc_if_complex_otherwise_value()
                        }
                    };
                    context.path.0.pop();
                }
                context.path.0.pop();
                result
            }
            Value::Object(object) => {
                if object.len() == 1 {
                    let (name, argument) = object.iter().next().unwrap();
                    if let Some(aliased_value) = context
                        .aliases
                        .get(name)
                        .and_then(|aliases_with_this_name| aliases_with_this_name.last())
                        .and_then(|rc_or_value| {
                            Some(rc_or_value.clone_rc_if_complex_otherwise_value())
                        })
                    {
                        let mut aliases_names = vec![];
                        if let Value::Object(aliases) = argument.value() {
                            if aliases.len() == 1 {
                                aliases_names.push("_".to_string());
                                context.add_alias(
                                    DEFAULT_ALIAS,
                                    argument.clone_rc_if_complex_otherwise_value(),
                                );
                            } else {
                                for (alias_name, aliased_value) in aliases.iter() {
                                    aliases_names.push(alias_name.clone());
                                    context.add_alias(
                                        &alias_name,
                                        aliased_value.clone_rc_if_complex_otherwise_value(),
                                    );
                                }
                            }
                        } else {
                            aliases_names.push("_".to_string());
                            context.add_alias(
                                DEFAULT_ALIAS,
                                argument.clone_rc_if_complex_otherwise_value(),
                            );
                        }
                        context.path.0.push(PathSegment::Alias(name.clone()));
                        let result = self.compute_with_context(&aliased_value, context)?;
                        context.path.0.pop();
                        for alias_name in aliases_names {
                            context.remove_alias(&alias_name);
                        }
                        return Ok(result);
                    }
                    if let Some(function) = self.supported_functions.get(name) {
                        context
                            .path
                            .0
                            .push(PathSegment::EmbeddedFunction(name.clone()));
                        let function_arguments = self.compute_with_context(&argument, context)?;
                        let result = (function.function)(function_arguments)?;
                        context.path.0.pop();
                        return Ok(result);
                    }
                }
                let mut result_map = BTreeMap::new();
                for (key, value) in object {
                    context.path.0.push(PathSegment::ObjectKey(key.clone()));
                    result_map.insert(key.clone(), self.compute_with_context(&value, context)?);
                    context.path.0.pop();
                }
                RcOrValue::Value(Value::Object(result_map))
            }
            Value::Array(array) => {
                let mut result_array = vec![];
                for (element_index, element) in array.iter().enumerate() {
                    context.path.0.push(PathSegment::ArrayIndex(element_index));
                    result_array.push(self.compute_with_context(&element, context)?);
                    context.path.0.pop();
                }
                RcOrValue::Value(Value::Array(result_array))
            }
            Value::String(string) => {
                if let Some(aliased_value) = context
                    .aliases
                    .get_mut(string)
                    .and_then(|values_for_this_name| values_for_this_name.pop())
                {
                    context.path.0.push(PathSegment::Alias(string.clone()));
                    let result = self.compute_with_context(&aliased_value, context)?;
                    context.path.0.pop();
                    context.add_alias(string, aliased_value);
                    result
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
                aliases: SmallMap::new(),
                entered_aliases: BTreeSet::new(),
                recursed_aliases_types: SmallMap::new(),
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
                Value::With(ref with_clause) => {
                    for (alias_name, alias_value) in with_clause.with.definitions.iter() {
                        context.add_alias(
                            &alias_name,
                            TypeOrRcOrValue::RcOrValue(
                                alias_value.clone_rc_if_complex_otherwise_value(),
                            ),
                        );
                    }
                    context.path.0.push(PathSegment::With);
                    context.path.0.push(PathSegment::Constants);
                    for (alias_name, alias_value) in with_clause.with.constants.iter() {
                        context.path.0.push(PathSegment::Alias(alias_name.clone()));
                        let precomputed_type = self
                            .get_type(TypeOrRcOrValue::RcOrValue(alias_value.clone()), context)?;
                        context.path.0.pop();
                        context.add_alias(&alias_name, TypeOrRcOrValue::Type(precomputed_type));
                    }
                    context.path.0.pop();
                    context.path.0.push(PathSegment::Compute);
                    let result = self.get_type(
                        TypeOrRcOrValue::RcOrValue(with_clause.compute.clone()),
                        context,
                    )?;
                    context.path.0.pop();
                    context.path.0.pop();
                    for alias_name in with_clause.with.definitions.keys() {
                        context.remove_alias(alias_name);
                    }
                    for alias_name in with_clause.with.constants.keys() {
                        context.remove_alias(alias_name);
                    }
                    result
                }
                Value::Map(ref map_clause) => {
                    context.path.0.push(PathSegment::Map);
                    let actual_array_type =
                        self.get_type(TypeOrRcOrValue::RcOrValue(map_clause.map.clone()), context)?;
                    context.path.0.pop();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        context.add_alias(
                            &map_clause.as_alias,
                            TypeOrRcOrValue::Type(*array_element_type.clone()),
                        );
                        context.path.0.push(PathSegment::Through);
                        let result = self.get_type(
                            TypeOrRcOrValue::RcOrValue(map_clause.through.clone()),
                            context,
                        )?;
                        context.path.0.pop();
                        context.remove_alias(&map_clause.as_alias);
                        Type::Array(Box::new(result))
                    } else {
                        return Err(anyhow!(
                            "Expected array for map clause at path {:?}, got {actual_array_type:?}",
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
                        context.add_alias(
                            &filter_clause.as_alias,
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
                                    "Expected filter at path {:?} to use function which returns \
                                     boolean value, but it returns {through_type:?}",
                                    context.path
                                )
                            })?;
                        context.remove_alias(&filter_clause.as_alias);
                        Type::Array(array_element_type.clone())
                    } else {
                        return Err(anyhow!(
                            "Expected array for filter clause at path {:?}, got \
                             {actual_array_type:?}",
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
                        context.add_alias(
                            &fold_clause.as_alias,
                            TypeOrRcOrValue::Type(*array_element_type.clone()),
                        );
                        context.add_alias(
                            &fold_clause.accumulating_in_alias,
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
                                    "Expected fold at path {:?} to use function which returns \
                                     value {starting_with_type:?} (as is starting value), but it \
                                     returns {through_type:?}",
                                    context.path
                                )
                            })?;
                        context.remove_alias(&fold_clause.as_alias);
                        context.remove_alias(&fold_clause.accumulating_in_alias);
                        Type::Array(Box::new(through_type))
                    } else {
                        return Err(anyhow!(
                            "Expected array for fold clause at path {:?}, got \
                             {actual_array_type:?}",
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
                                "Expected 'then' and 'else' branches at path {:?} to be of the \
                                 same type, but 'then' branch is {then_branch_type:?} and 'else' \
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
                    context.add_alias(
                        &try_or_clause.with_error_alias,
                        TypeOrRcOrValue::Type(Type::String),
                    );
                    let or_branch_type = self.get_type(
                        TypeOrRcOrValue::RcOrValue(try_or_clause.or.clone()),
                        context,
                    )?;
                    context.path.0.pop();
                    context.remove_alias(&try_or_clause.with_error_alias);
                    context
                        .assert_equal(&try_branch_type, &or_branch_type)
                        .with_context(|| {
                            anyhow!(
                                "Expected 'try' and 'or' branches at path {:?} to be of the same \
                                 type, but 'try' branch is {try_branch_type:?} and 'or' branch is \
                                 {or_branch_type:?}",
                                context.path
                            )
                        })?;
                    try_branch_type
                }
                Value::FromAt(ref from_at_clause) => {
                    context.path.0.push(PathSegment::From);
                    let mut result = self.get_type(
                        TypeOrRcOrValue::RcOrValue(
                            from_at_clause.from.clone_rc_if_complex_otherwise_value(),
                        ),
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
                                             key {object_key:?} at this point, but it has no such \
                                             key"
                                        ));
                                    }
                                }
                                r#type => {
                                    return Err(anyhow!(
                                        "Expected to reach from 'from' to be an object at this \
                                         point, but it is {type:?}"
                                    ));
                                }
                            },
                            AtSegment::ArrayIndex(_) => match result {
                                Type::Array(element_type) => {
                                    result = *element_type;
                                }
                                r#type => {
                                    return Err(anyhow!(
                                        "Expected to reach from 'from' to be an array at this \
                                         point, but it is {type:?}"
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
                        let (name, argument) = object.iter().next().unwrap();
                        if let Some(aliased_value) = context
                            .aliases
                            .get(name)
                            .and_then(|aliases_with_this_name| aliases_with_this_name.last())
                            .cloned()
                        {
                            if context.entered_aliases.contains(name) {
                                if let Some(this_recursed_alias_type) =
                                    context.recursed_aliases_types.get(name)
                                {
                                    return Ok(this_recursed_alias_type.clone());
                                } else {
                                    context
                                        .recursed_aliases_types
                                        .insert(name.clone(), Type::RecursedAlias(name.clone()));
                                }
                            }
                            let mut aliases_names = vec![];
                            if let Value::Object(aliases) = argument.value() {
                                if aliases.len() == 1 {
                                    aliases_names.push("_".to_string());
                                    context.add_alias(
                                        DEFAULT_ALIAS,
                                        TypeOrRcOrValue::RcOrValue(argument.clone()),
                                    );
                                } else {
                                    for (alias_name, alias_value) in aliases.iter() {
                                        aliases_names.push(alias_name.clone());
                                        context.add_alias(
                                            &alias_name,
                                            TypeOrRcOrValue::RcOrValue(alias_value.clone()),
                                        );
                                    }
                                }
                            } else {
                                aliases_names.push("_".to_string());
                                context.add_alias(
                                    DEFAULT_ALIAS,
                                    TypeOrRcOrValue::RcOrValue(argument.clone()),
                                );
                            }
                            context.path.0.push(PathSegment::Alias(name.clone()));
                            context.entered_aliases.insert(name.clone());
                            let result = self.get_type(aliased_value, context)?;
                            context.path.0.pop();
                            context.entered_aliases.remove(name);
                            for alias_name in aliases_names {
                                context.remove_alias(&alias_name);
                                context.recursed_aliases_types.remove(&alias_name);
                            }
                            return Ok(result);
                        }
                        if let Some(function) = self.supported_functions.get(name) {
                            context
                                .path
                                .0
                                .push(PathSegment::EmbeddedFunction(name.clone()));
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
                    let mut recursed_elements_aliases_names = vec![];
                    for (element_index, element) in array.iter().enumerate() {
                        context.path.0.push(PathSegment::ArrayIndex(element_index));
                        match self.get_type(TypeOrRcOrValue::RcOrValue(element.clone()), context)? {
                            Type::RecursedAlias(recursed_alias_name) => {
                                recursed_elements_aliases_names.push(recursed_alias_name);
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
                            let result_error = Err(anyhow!(
                                "Expected value at path {:?} to be \
                                 {first_non_recursed_element_type:?}, but it is \
                                 {unexpected_type:?}",
                                context.path
                            ));
                            context.path.0.pop();
                            return result_error;
                        } else {
                            Type::Array(Box::new(first_non_recursed_element_type.clone()))
                        }
                    } else if let Some(first_recursed_element_alias_name) =
                        recursed_elements_aliases_names.first()
                    {
                        Type::Array(Box::new(Type::RecursedAlias(
                            first_recursed_element_alias_name.clone(),
                        )))
                    } else {
                        return Err(anyhow!(
                            "Expected non-empty array at path {:?}",
                            context.path
                        ));
                    }
                }
                Value::String(ref string) => {
                    if context.entered_aliases.contains(string) {
                        if let Some(already_discovered_type) =
                            context.recursed_aliases_types.get(string)
                        {
                            already_discovered_type.clone()
                        } else {
                            context
                                .recursed_aliases_types
                                .insert(string.clone(), Type::RecursedAlias(string.clone()));
                            Type::RecursedAlias(string.clone())
                        }
                    } else if let Some(aliased_value) = context
                        .aliases
                        .get_mut(string)
                        .and_then(|values_for_this_name| values_for_this_name.pop())
                    {
                        context.path.0.push(PathSegment::Alias(string.clone()));
                        let result = self.get_type(
                            aliased_value.clone_rc_if_complex_otherwise_type_or_value(),
                            context,
                        )?;
                        context.path.0.pop();
                        context.add_alias(&string, aliased_value);
                        result
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
