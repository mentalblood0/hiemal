pub mod embedded_functions;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::{anyhow, Context, Error, Result};

#[derive(PartialEq, Debug, Clone, PartialOrd, Ord, Eq)]
pub enum Type {
    Number,
    String,
    Bool,
    Null,
    Array(Box<Type>),
    Object(BTreeMap<String, Type>),
    GenericArgument(u8),
    RecursedAlias(String),
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct With {
    #[serde(default)]
    definitions: BTreeMap<String, Arc<Value>>,
    #[serde(default)]
    constants: BTreeMap<String, Arc<Value>>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct WithCompute {
    with: With,
    compute: Arc<Value>,
}

fn default_alias() -> String {
    "_".to_string()
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Map {
    map: Arc<Value>,
    #[serde(default = "default_alias")]
    as_alias: String,
    through: Arc<Value>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Filter {
    filter: Arc<Value>,
    #[serde(default = "default_alias")]
    as_alias: String,
    through: Arc<Value>,
}

fn default_current_value_alias() -> String {
    "current".to_string()
}

fn default_accumulator_value_alias() -> String {
    "accumulator".to_string()
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Reduce {
    reduce: Arc<Value>,
    #[serde(default = "default_current_value_alias")]
    as_alias: String,
    starting_with: Arc<Value>,
    #[serde(default = "default_accumulator_value_alias")]
    accumulating_in_alias: String,
    through: Arc<Value>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Branching {
    #[serde(rename = "IF")]
    if_: Arc<Value>,
    then: Arc<Value>,
    #[serde(rename = "ELSE")]
    else_: Arc<Value>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum Value {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    Array(Vec<Arc<Value>>),
    With(WithCompute),
    Map(Map),
    Filter(Filter),
    Reduce(Reduce),
    Branching(Branching),
    Object(BTreeMap<String, Arc<Value>>),
}

impl Value {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(result) => Some(*result),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&String> {
        match self {
            Value::String(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(result) => Some(*result),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Arc<Value>>> {
        match self {
            Value::Array(result) => Some(result),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Arc<Value>>> {
        match self {
            Value::Object(result) => Some(result),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Function {
    pub argument_type: Type,
    pub return_type: Type,
    pub function: fn(Arc<Value>) -> Result<Arc<Value>>,
}

pub struct Interpreter {
    pub supported_functions: BTreeMap<String, Function>,
}

#[macro_export]
macro_rules! define_default_interpreter_supported_functions {
    (
        $(
            $function_name:ident
            $function_argument_type:expr, $function_return_type:expr, $function_argument:ident $function_code:block
        )*
    ) => {
        paste! {
            $(
                pub fn [<$function_name:lower>]($function_argument: Arc<Value>) -> Result<Arc<Value>> $function_code
            )*

            impl Default for Interpreter {
                fn default() -> Interpreter {
                    Interpreter {
                        supported_functions: BTreeMap::from([
                            $(
                                (
                                    stringify!($function_name).to_string(),
                                    Function {
                                        argument_type: $function_argument_type,
                                        return_type: $function_return_type,
                                        function: [<$function_name:lower>],
                                    },
                                ),
                            )*
                        ]),
                    }
                }
            }
        }
    };
}

pub enum PathSegment {
    ObjectKey(String),
    Alias(String),
    EmbeddedFunction(String),
    ArrayIndex(usize),
}

pub struct Path(pub Vec<PathSegment>);

#[derive(Clone, Debug)]
pub enum TypeOrValue {
    Type(Type),
    Value(Arc<Value>),
}

#[derive(Debug)]
pub struct TypeCheckingContext {
    pub path: Vec<String>,
    pub aliases: BTreeMap<String, Vec<TypeOrValue>>,
    pub entered_aliases: BTreeSet<String>,
    pub recursed_aliases_types: BTreeMap<String, Type>,
}

impl TypeCheckingContext {
    pub fn add_alias(&mut self, name: String, type_or_value: TypeOrValue) {
        self.aliases.entry(name).or_default().push(type_or_value);
    }

    pub fn remove_alias(&mut self, name: &String) {
        self.aliases.get_mut(name).unwrap().pop();
    }

    pub fn error(&self, expected_type: &Type, got_type: &Type) -> Error {
        anyhow!(
            "Expected value of type {expected_type:?} but got value of type {got_type:?} at path \
             {:?}",
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
        // println!("assert_equal {expected_type:?} {actual_type:?}");
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

pub struct ComputationContext {
    pub path: Vec<String>,
    pub aliases: BTreeMap<String, Vec<Arc<Value>>>,
}

impl ComputationContext {
    pub fn add_alias(&mut self, name: String, value: Arc<Value>) {
        self.aliases.entry(name).or_default().push(value);
    }

    pub fn remove_alias(&mut self, name: &String) {
        self.aliases.get_mut(name).unwrap().pop();
    }
}

impl Interpreter {
    pub fn compute(&self, program: Arc<Value>) -> Result<Arc<Value>> {
        self.check_types(program.clone())?;
        self.compute_with_context(
            program,
            &mut ComputationContext {
                path: vec![],
                aliases: BTreeMap::new(),
            },
        )
    }

    fn compute_with_context(
        &self,
        program: Arc<Value>,
        context: &mut ComputationContext,
    ) -> Result<Arc<Value>> {
        Ok(match *program {
            Value::With(ref with_clause) => {
                for (alias_name, alias_value) in with_clause.with.definitions.iter() {
                    context.add_alias(alias_name.clone(), alias_value.clone());
                }
                context.path.push("WITH".to_string());
                context.path.push("CONSTANTS".to_string());
                for (alias_name, alias_value) in with_clause.with.constants.iter() {
                    context.path.push(alias_name.clone());
                    let precomputed_value =
                        self.compute_with_context(alias_value.clone(), context)?;
                    context.path.pop();
                    context.add_alias(alias_name.clone(), precomputed_value);
                }
                *context.path.last_mut().unwrap() = "COMPUTE".to_string();
                let result = self.compute_with_context(with_clause.compute.clone(), context)?;
                context.path.pop();
                context.path.pop();
                for alias_name in with_clause.with.definitions.keys() {
                    context.remove_alias(alias_name);
                }
                for alias_name in with_clause.with.constants.keys() {
                    context.remove_alias(alias_name);
                }
                result
            }
            Value::Map(ref map_clause) => {
                let array = self
                    .compute_with_context(map_clause.map.clone(), context)?
                    .as_array()
                    .unwrap()
                    .clone();
                let mut result = vec![];
                context.path.push("MAP".to_string());
                for (element_index, element) in array.iter().enumerate() {
                    context.add_alias(map_clause.as_alias.clone(), element.clone());
                    context.path.push(element_index.to_string());
                    context.path.push("THROUGH".to_string());
                    result.push(self.compute_with_context(map_clause.through.clone(), context)?);
                    context.path.pop();
                    context.path.pop();
                    context.remove_alias(&map_clause.as_alias);
                }
                context.path.pop();
                Arc::new(Value::Array(result))
            }
            Value::Filter(ref filter_clause) => {
                let array = self
                    .compute_with_context(filter_clause.filter.clone(), context)?
                    .as_array()
                    .unwrap()
                    .clone();
                let mut result = vec![];
                context.path.push("FILTER".to_string());
                for (element_index, element) in array.iter().enumerate() {
                    context.add_alias(filter_clause.as_alias.clone(), element.clone());
                    context.path.push(element_index.to_string());
                    context.path.push("THROUGH".to_string());
                    if self
                        .compute_with_context(filter_clause.through.clone(), context)?
                        .as_bool()
                        .unwrap()
                    {
                        result.push(element.clone());
                    }
                    context.path.pop();
                    context.path.pop();
                    context.remove_alias(&filter_clause.as_alias);
                }
                context.path.pop();
                Arc::new(Value::Array(result))
            }
            Value::Reduce(ref reduce_clause) => {
                let array = self
                    .compute_with_context(reduce_clause.reduce.clone(), context)?
                    .as_array()
                    .unwrap()
                    .clone();
                context.path.push("STARTING_WITH".to_string());
                let mut result =
                    self.compute_with_context(reduce_clause.starting_with.clone(), context)?;
                *context.path.last_mut().unwrap() = "REDUCE".to_string();
                for (element_index, element) in array.iter().enumerate() {
                    context.add_alias(reduce_clause.as_alias.clone(), element.clone());
                    context.add_alias(reduce_clause.accumulating_in_alias.clone(), result.clone());
                    context.path.push(element_index.to_string());
                    context.path.push("THROUGH".to_string());
                    result = self.compute_with_context(reduce_clause.through.clone(), context)?;
                    context.path.pop();
                    context.path.pop();
                    context.remove_alias(&reduce_clause.as_alias);
                    context.remove_alias(&reduce_clause.accumulating_in_alias);
                }
                context.path.pop();
                result
            }
            Value::Branching(ref branching_clause) => {
                context.path.push("IF".to_string());
                let if_result = self
                    .compute_with_context(branching_clause.if_.clone(), context)?
                    .as_bool()
                    .unwrap();
                let result = if if_result {
                    *context.path.last_mut().unwrap() = "THEN".to_string();
                    self.compute_with_context(branching_clause.then.clone(), context)?
                } else {
                    *context.path.last_mut().unwrap() = "ELSE".to_string();
                    self.compute_with_context(branching_clause.else_.clone(), context)?
                };
                context.path.pop();
                result
            }
            Value::Object(ref object) => {
                if object.len() == 1 {
                    let (name, arguments) = object.iter().next().unwrap();
                    if let Some(aliased_value) = context
                        .aliases
                        .get(name)
                        .and_then(|aliases_with_this_name| aliases_with_this_name.last())
                        .cloned()
                    {
                        let mut aliases_names = vec![];
                        if let Value::Object(ref aliases) = **arguments {
                            if aliases.len() == 1 {
                                aliases_names.push("_".to_string());
                                context.add_alias("_".to_string(), arguments.clone());
                            } else {
                                for (alias_name, alias_value) in aliases.iter() {
                                    aliases_names.push(alias_name.clone());
                                    context.add_alias(alias_name.clone(), alias_value.clone());
                                }
                            }
                        } else {
                            aliases_names.push("_".to_string());
                            context.add_alias("_".to_string(), arguments.clone());
                        }
                        context.path.push(name.clone());
                        let result = self.compute_with_context(aliased_value, context)?;
                        context.path.pop();
                        for alias_name in aliases_names {
                            context.remove_alias(&alias_name);
                        }
                        return Ok(result);
                    }
                    let function = self.supported_functions.get(name).unwrap();
                    context.path.push(name.clone());
                    let function_arguments =
                        self.compute_with_context(arguments.clone(), context)?;
                    let result = (function.function)(function_arguments)?;
                    context.path.pop();
                    result
                } else {
                    let mut result_map = BTreeMap::new();
                    for (key, value) in object {
                        context.path.push(key.clone());
                        result_map.insert(
                            key.clone(),
                            self.compute_with_context(value.clone(), context)?,
                        );
                        context.path.pop();
                    }
                    Arc::new(Value::Object(result_map))
                }
            }
            Value::Array(ref array) => {
                let mut result_array = vec![];
                for (element_index, element) in array.iter().enumerate() {
                    context.path.push(element_index.to_string());
                    result_array.push(self.compute_with_context(element.clone(), context)?);
                    context.path.pop();
                }
                Arc::new(Value::Array(result_array))
            }
            Value::String(ref string) => {
                if let Some(aliased_value) = context
                    .aliases
                    .get(string)
                    .and_then(|values_for_this_name| values_for_this_name.last())
                    .cloned()
                {
                    context.path.push(string.clone());
                    let result = self.compute_with_context(aliased_value, context)?;
                    context.path.pop();
                    result
                } else {
                    Arc::new(Value::String(string.clone()))
                }
            }
            ref value => Arc::new(value.clone()),
        })
    }

    pub fn check_types(&self, program: Arc<Value>) -> Result<Type> {
        self.get_type(
            TypeOrValue::Value(program),
            &mut TypeCheckingContext {
                path: vec![],
                aliases: BTreeMap::new(),
                entered_aliases: BTreeSet::new(),
                recursed_aliases_types: BTreeMap::new(),
            },
        )
    }

    fn get_type(&self, program: TypeOrValue, context: &mut TypeCheckingContext) -> Result<Type> {
        // println!("{:?} {:?}", context, program);
        // println!("{:?}", context.path);
        let result = match program {
            TypeOrValue::Type(program_type) => program_type,
            TypeOrValue::Value(program) => match *program {
                Value::With(ref with_clause) => {
                    for (alias_name, alias_value) in with_clause.with.definitions.iter() {
                        context
                            .add_alias(alias_name.clone(), TypeOrValue::Value(alias_value.clone()));
                    }
                    context.path.push("WITH".to_string());
                    context.path.push("CONSTANTS".to_string());
                    for (alias_name, alias_value) in with_clause.with.constants.iter() {
                        context.path.push(alias_name.clone());
                        let precomputed_type =
                            self.get_type(TypeOrValue::Value(alias_value.clone()), context)?;
                        context.path.pop();
                        context.add_alias(alias_name.clone(), TypeOrValue::Type(precomputed_type));
                    }
                    context.path.pop();
                    context.path.push("COMPUTE".to_string());
                    let result =
                        self.get_type(TypeOrValue::Value(with_clause.compute.clone()), context)?;
                    context.path.pop();
                    context.path.pop();
                    for alias_name in with_clause.with.definitions.keys() {
                        context.remove_alias(alias_name);
                    }
                    for alias_name in with_clause.with.constants.keys() {
                        context.remove_alias(alias_name);
                    }
                    result
                }
                Value::Map(ref map_clause) => {
                    context.path.push("MAP".to_string());
                    let actual_array_type =
                        self.get_type(TypeOrValue::Value(map_clause.map.clone()), context)?;
                    context.path.pop();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        context.add_alias(
                            map_clause.as_alias.clone(),
                            TypeOrValue::Type(*array_element_type.clone()),
                        );
                        context.path.push("THROUGH".to_string());
                        let result =
                            self.get_type(TypeOrValue::Value(map_clause.through.clone()), context)?;
                        context.path.pop();
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
                    context.path.push("FILTER".to_string());
                    let actual_array_type =
                        self.get_type(TypeOrValue::Value(filter_clause.filter.clone()), context)?;
                    context.path.pop();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        context.add_alias(
                            filter_clause.as_alias.clone(),
                            TypeOrValue::Type(*array_element_type.clone()),
                        );
                        context.path.push("THROUGH".to_string());
                        let through_type = self
                            .get_type(TypeOrValue::Value(filter_clause.through.clone()), context)?;
                        context.path.pop();
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
                Value::Reduce(ref reduce_clause) => {
                    context.path.push("REDUCE".to_string());
                    let actual_array_type =
                        self.get_type(TypeOrValue::Value(reduce_clause.reduce.clone()), context)?;
                    context.path.pop();
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        let starting_with_type = self.get_type(
                            TypeOrValue::Value(reduce_clause.starting_with.clone()),
                            context,
                        )?;
                        context.add_alias(
                            reduce_clause.as_alias.clone(),
                            TypeOrValue::Type(*array_element_type.clone()),
                        );
                        context.add_alias(
                            reduce_clause.accumulating_in_alias.clone(),
                            TypeOrValue::Type(starting_with_type.clone()),
                        );
                        context.path.push("THROUGH".to_string());
                        let through_type = self
                            .get_type(TypeOrValue::Value(reduce_clause.through.clone()), context)?;
                        context.path.pop();
                        context
                            .assert_equal(&through_type, &starting_with_type)
                            .with_context(|| {
                                anyhow!(
                                    "Expected reduce at path {:?} to use function which returns \
                                     value of type {starting_with_type:?} (as is starting value), \
                                     but it returns {through_type:?}",
                                    context.path
                                )
                            })?;
                        context.remove_alias(&reduce_clause.as_alias);
                        context.remove_alias(&reduce_clause.accumulating_in_alias);
                        Type::Array(Box::new(through_type))
                    } else {
                        return Err(anyhow!(
                            "Expected array for reduce clause at path {:?}, got \
                             {actual_array_type:?}",
                            context.path
                        ));
                    }
                }
                Value::Branching(ref branching_clause) => {
                    context.path.push("IF".to_string());
                    let if_branch_type =
                        self.get_type(TypeOrValue::Value(branching_clause.if_.clone()), context)?;
                    context.path.pop();
                    if if_branch_type != Type::Bool {
                        return Err(anyhow!(
                            "Expected condition at path {:?} to be of boolean type, but it is of \
                             type {if_branch_type:?}",
                            context.path
                        ));
                    }
                    context.path.push("THEN".to_string());
                    let then_branch_type =
                        self.get_type(TypeOrValue::Value(branching_clause.then.clone()), context)?;
                    context.path.pop();
                    context.path.push("ELSE".to_string());
                    let else_branch_type =
                        self.get_type(TypeOrValue::Value(branching_clause.else_.clone()), context)?;
                    context.path.pop();
                    context
                        .assert_equal(&then_branch_type, &else_branch_type)
                        .with_context(|| {
                            anyhow!(
                                "Expected 'then' and 'else' branches at path {:?} to be of the \
                                 same type, but 'then' branch is of type {then_branch_type:?} and \
                                 'else' branch is of type {else_branch_type:?}",
                                context.path
                            )
                        })?;
                    then_branch_type
                }
                Value::Object(ref object) => {
                    if object.len() == 1 {
                        let (name, arguments) = object.iter().next().unwrap();
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
                            if let Value::Object(ref aliases) = **arguments {
                                if aliases.len() == 1 {
                                    aliases_names.push("_".to_string());
                                    context.add_alias(
                                        "_".to_string(),
                                        TypeOrValue::Value(arguments.clone()),
                                    );
                                } else {
                                    for (alias_name, alias_value) in aliases.iter() {
                                        aliases_names.push(alias_name.clone());
                                        context.add_alias(
                                            alias_name.clone(),
                                            TypeOrValue::Value(alias_value.clone()),
                                        );
                                    }
                                }
                            } else {
                                aliases_names.push("_".to_string());
                                context.add_alias(
                                    "_".to_string(),
                                    TypeOrValue::Value(arguments.clone()),
                                );
                            }
                            context.path.push(name.clone());
                            context.entered_aliases.insert(name.clone());
                            let result = self.get_type(aliased_value, context)?;
                            context.path.pop();
                            context.entered_aliases.remove(name);
                            for alias_name in aliases_names {
                                context.remove_alias(&alias_name);
                                context.recursed_aliases_types.remove(&alias_name);
                            }
                            return Ok(result);
                        }
                        if let Some(function) = self.supported_functions.get(name) {
                            context.path.push(name.clone());
                            let arguments_type =
                                self.get_type(TypeOrValue::Value(arguments.clone()), context)?;
                            let generic_values =
                                context.assert_equal(&function.argument_type, &arguments_type)?;
                            context.path.pop();
                            let mut result = function.return_type.clone();
                            context.substitute_generic_arguments_values(
                                &mut result,
                                &generic_values,
                            )?;
                            result
                        } else {
                            return Err(anyhow!(
                                "Expected supported function at path {:?}, but got unsupported \
                                 function {name:?}. Supported functions are: {:?}",
                                context.path,
                                self.supported_functions
                                    .keys()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ));
                        }
                    } else {
                        let mut result_map = BTreeMap::new();
                        for (key, value) in object {
                            context.path.push(key.clone());
                            result_map.insert(
                                key.clone(),
                                self.get_type(TypeOrValue::Value(value.clone()), context)?,
                            );
                            context.path.pop();
                        }
                        Type::Object(result_map)
                    }
                }
                Value::Array(ref array) => {
                    let mut non_recursed_elements_indexes_and_types =
                        Vec::with_capacity(array.len());
                    let mut recursed_elements_aliases_names = vec![];
                    for (element_index, element) in array.iter().enumerate() {
                        context.path.push(element_index.to_string());
                        match self.get_type(TypeOrValue::Value(element.clone()), context)? {
                            Type::RecursedAlias(recursed_alias_name) => {
                                recursed_elements_aliases_names.push(recursed_alias_name);
                            }
                            non_recursed_type => {
                                non_recursed_elements_indexes_and_types
                                    .push((element_index, non_recursed_type));
                            }
                        }
                        context.path.pop();
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
                            context.path.push(unexpected_type_element_index.to_string());
                            let result_error = Err(anyhow!(
                                "Expected value at path {:?} to be of type \
                                 {first_non_recursed_element_type:?}, but it is of type \
                                 {unexpected_type:?}",
                                context.path
                            ));
                            context.path.pop();
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
                        context
                            .recursed_aliases_types
                            .entry(string.clone())
                            .or_insert(Type::RecursedAlias(string.clone()))
                            .clone()
                    } else if let Some(aliased_value) = context
                        .aliases
                        .get(string)
                        .and_then(|values_for_this_name| values_for_this_name.last())
                        .cloned()
                    {
                        context.path.push(string.clone());
                        let result = self.get_type(aliased_value.clone(), context)?;
                        context.path.pop();
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::OnceLock;

    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn default_interpreter() -> &'static Interpreter {
        static INTERPRETER: OnceLock<Interpreter> = OnceLock::new();
        INTERPRETER.get_or_init(|| Interpreter::default())
    }

    #[test]
    fn test_simple_embedded_functions() {
        assert_eq!(
            *default_interpreter()
                .compute(
                    serde_json::from_value(json!({
                        "SUM": [
                            {"PRODUCT": [2, 3]},
                            {"LEN": {"CONCAT": ["lala", "lolo"]}},
                            4
                        ]
                    }))
                    .unwrap(),
                )
                .unwrap(),
            Value::Number(18.0)
        );
    }

    #[test]
    fn test_with() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                        "SUM": [
                            {
                                "WITH": {"DEFINITIONS": {"x": 2, "y": 3}},
                                "COMPUTE": {"PRODUCT": ["x", "x", "y"]}
                            },
                            {"LEN": {"CONCAT": ["lala", "lolo"]}},
                            4
                        ]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
            Value::Number(24.0)
        );
    }

    #[test]
    fn test_user_functions_definitions() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                        "SUM": [
                            {
                                "WITH": {
                                    "DEFINITIONS": {
                                        "SQUARE": {"PRODUCT": ["_", "_"]},
                                        "y": 3
                                    }
                                },
                                "COMPUTE": {"PRODUCT": [
                                    {"SQUARE": 2},
                                    {
                                        "SQUARE": {
                                            "SQUARE": {
                                                "PRODUCT": [
                                                    {"SQUARE": 1},
                                                    {"SUM": ["y", -1]}
                                                ]
                                            }
                                        }
                                    }
                                ]}
                            },
                            {"LEN": {"CONCAT": ["lala", "lolo"]}},
                            4
                        ]
                    }))
                    .unwrap()
                ),)
                .unwrap(),
            Value::Number(76.0)
        );
    }

    #[test]
    fn test_generics() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                        "SUM": [
                            {
                                "GET_ELEMENT": {
                                    "from": [
                                        {"SIZE": [1, 2, 3]},
                                        {"SIZE": ["a", "b"]},
                                    ],
                                    "at": 1
                                }
                            },
                            1
                        ]
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Number(3.0)
        );
    }

    #[test]
    fn test_map() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                        "SUM": {
                            "MAP": [
                                {"SIZE": [1, 2, 3]},
                                1
                            ],
                            "THROUGH": {"SUM": ["_", 1]}
                        }
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Number(6.0)
        );
    }

    #[test]
    fn test_filter() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                        "SUM": {
                            "FILTER": [
                                {"SIZE": [1, 2, 3]},
                                2,
                                1
                            ],
                            "AS_ALIAS": "x",
                            "THROUGH": {"IS_SORTED": ["x", 2]}
                        }
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Number(3.0)
        );
    }

    #[test]
    fn test_reduce() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                        "REDUCE": [
                            {"SIZE": [1, 2, 3]},
                            2,
                            1
                        ],
                        "STARTING_WITH": 0,
                        "THROUGH": {
                            "SUM": [
                                "accumulator",
                                {"PRODUCT": ["current", "current"]}
                            ]
                        }
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Number(14.0)
        );
    }

    #[test]
    fn test_factorial() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                        "WITH": {
                            "DEFINITIONS": {
                                "FACTORIAL": {
                                    "PRODUCT": {
                                        "SEQUENCE": {
                                            "from": 1,
                                            "to": "_",
                                            "step": 1
                                        }
                                    }
                                }
                            }
                        },
                        "COMPUTE": {
                            "FACTORIAL": 5
                        }
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Number(120.0)
        );
    }

    #[test]
    fn test_definitions_vs_constants() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                        "WITH": {"CONSTANTS": {"x": 1}},
                        "COMPUTE": {
                            "WITH": {
                                "DEFINITIONS": {"definition": "x"},
                                "CONSTANTS": {"x": 2, "constant": "x"}
                            },
                            "COMPUTE": ["definition", "constant"]
                        }
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Array(vec![
                Arc::new(Value::Number(2.0)),
                Arc::new(Value::Number(1.0))
            ])
        );
    }

    #[test]
    fn test_branching() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                        "IF": true,
                        "THEN": 1,
                        "ELSE": 0
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Number(1.0)
        );
    }

    #[test]
    fn test_recursive_normal() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                      "WITH": {
                        "DEFINITIONS": {
                          "FIBONACCI": {
                            "IF": {
                              "IS_SORTED": [
                                "_",
                                1
                              ]
                            },
                            "THEN": "_",
                            "ELSE": {
                              "WITH": {
                                "CONSTANTS": {
                                  "x": "_"
                                }
                              },
                              "COMPUTE": {
                                "SUM": [
                                  {
                                    "FIBONACCI": {
                                      "SUM": [
                                        "x",
                                        -1
                                      ]
                                    }
                                  },
                                  {
                                    "FIBONACCI": {
                                      "SUM": [
                                        "x",
                                        -2
                                      ]
                                    }
                                  }
                                ]
                              }
                            }
                          }
                        }
                      },
                      "COMPUTE": {
                        "FIBONACCI": 10
                      }
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Number(55.0)
        );
    }
    #[test]
    fn test_recursive_short() {
        assert_eq!(
            *default_interpreter()
                .compute(Arc::new(
                    serde_json::from_value(json!({
                      "WITH": {
                        "DEFINITIONS": {
                          "FIBONACCI": {
                            "IF": {
                              "IS_SORTED": [
                                "_",
                                1
                              ]
                            },
                            "THEN": "_",
                            "ELSE": {
                              "WITH": {
                                "CONSTANTS": {
                                  "x": "_"
                                }
                              },
                              "COMPUTE": {
                                "FIBONACCI": {
                                  "SUM": [
                                    "x",
                                    -1
                                  ]
                                }
                              }
                            }
                          }
                        }
                      },
                      "COMPUTE": {
                        "FIBONACCI": 10
                      }
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Number(1.0)
        );
    }
    #[test]
    fn test_recursive_error() {
        assert!(default_interpreter()
            .compute(Arc::new(
                serde_json::from_value(json!({
                  "WITH": {
                    "DEFINITIONS": {
                      "FIBONACCI": {
                        "IF": {
                          "IS_SORTED": [
                            "_",
                            1
                          ]
                        },
                        "THEN": "_",
                        "ELSE": {
                          "WITH": {
                            "CONSTANTS": {
                              "x": "_"
                            }
                          },
                          "COMPUTE": {
                            "SUM": [
                              {
                                "FIBONACCI": "lalala"
                              },
                              {
                                "FIBONACCI": {
                                  "SUM": [
                                    "x",
                                    -2
                                  ]
                                }
                              }
                            ]
                          }
                        }
                      }
                    }
                  },
                  "COMPUTE": {
                    "FIBONACCI": 10
                  }
                }))
                .unwrap(),
            ))
            .is_err());
    }
    #[test]
    fn test_recursive_long() {
        let builder = std::thread::Builder::new().stack_size(2 * 1024 * 1024);
        let handler = builder
            .spawn(|| {
                assert_eq!(
                    *default_interpreter()
                        .compute(Arc::new(
                            serde_json::from_value(json!({
                              "WITH": {
                                "DEFINITIONS": {
                                  "FIBONACCI_1": {
                                    "IF": {
                                      "IS_SORTED": [
                                        "_",
                                        1
                                      ]
                                    },
                                    "THEN": "_",
                                    "ELSE": {
                                      "WITH": {
                                        "CONSTANTS": {
                                          "x": "_"
                                        }
                                      },
                                      "COMPUTE": {
                                        "SUM": [
                                          {
                                            "FIBONACCI_2": {
                                              "SUM": [
                                                "x",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "FIBONACCI_2": {
                                              "SUM": [
                                                "x",
                                                -2
                                              ]
                                            }
                                          }
                                        ]
                                      }
                                    }
                                  },
                                  "FIBONACCI_2": {
                                    "IF": {
                                      "IS_SORTED": [
                                        "_",
                                        1
                                      ]
                                    },
                                    "THEN": "_",
                                    "ELSE": {
                                      "WITH": {
                                        "CONSTANTS": {
                                          "x": "_"
                                        }
                                      },
                                      "COMPUTE": {
                                        "SUM": [
                                          {
                                            "FIBONACCI_3": {
                                              "SUM": [
                                                "x",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "FIBONACCI_3": {
                                              "SUM": [
                                                "x",
                                                -2
                                              ]
                                            }
                                          }
                                        ]
                                      }
                                    }
                                  },
                                  "FIBONACCI_3": {
                                    "IF": {
                                      "IS_SORTED": [
                                        "_",
                                        1
                                      ]
                                    },
                                    "THEN": "_",
                                    "ELSE": {
                                      "WITH": {
                                        "CONSTANTS": {
                                          "x": "_"
                                        }
                                      },
                                      "COMPUTE": {
                                        "SUM": [
                                          {
                                            "FIBONACCI_4": {
                                              "SUM": [
                                                "x",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "FIBONACCI_4": {
                                              "SUM": [
                                                "x",
                                                -2
                                              ]
                                            }
                                          }
                                        ]
                                      }
                                    }
                                  },
                                  "FIBONACCI_4": {
                                    "IF": {
                                      "IS_SORTED": [
                                        "_",
                                        1
                                      ]
                                    },
                                    "THEN": "_",
                                    "ELSE": {
                                      "WITH": {
                                        "CONSTANTS": {
                                          "x": "_"
                                        }
                                      },
                                      "COMPUTE": {
                                        "SUM": [
                                          {
                                            "FIBONACCI_1": {
                                              "SUM": [
                                                "x",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "FIBONACCI_1": {
                                              "SUM": [
                                                "x",
                                                -2
                                              ]
                                            }
                                          }
                                        ]
                                      }
                                    }
                                  }
                                }
                              },
                              "COMPUTE": {
                                "FIBONACCI_1": 10
                              }
                            }))
                            .unwrap()
                        ))
                        .unwrap(),
                    Value::Number(55.0)
                );
            })
            .unwrap();
        handler.join().unwrap();
    }
}
