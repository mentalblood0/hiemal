pub mod embedded_functions;

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};

#[derive(PartialEq, Debug, Clone, PartialOrd, Ord, Eq)]
pub enum Type {
    Number,
    String,
    Bool,
    Null,
    Array(Box<Type>),
    AnyObject,
    Object(BTreeMap<String, Type>),
    GenericArgument(u8),
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct With {
    with: BTreeMap<String, Arc<Value>>,
    compute: Arc<Value>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Map {
    map: Arc<Value>,
    through: Arc<Value>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug, Clone)]
#[serde(untagged)]
pub enum Value {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    Array(Vec<Arc<Value>>),
    With(With),
    Map(Map),
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

#[derive(Clone)]
pub enum TypeOrValue {
    Type(Type),
    Value(Arc<Value>),
}

pub struct TypeCheckingContext {
    pub path: Vec<String>,
    pub aliases: BTreeMap<String, Vec<TypeOrValue>>,
}

pub struct ComputationContext {
    pub path: Vec<String>,
    pub aliases: BTreeMap<String, Vec<Arc<Value>>>,
}

impl Interpreter {
    fn get_generic_arguments_values(
        &self,
        generic: &Type,
        actual: &Type,
    ) -> Result<[Option<Type>; 256]> {
        let mut result: [Option<Type>; 256] = std::array::from_fn(|_| None);
        self.get_generic_arguments_values_into_dict(generic, actual, &mut result)?;
        Ok(result)
    }

    fn get_generic_arguments_values_into_dict(
        &self,
        generic: &Type,
        actual: &Type,
        result: &mut [Option<Type>; 256],
    ) -> Result<()> {
        match (generic, actual) {
            (Type::GenericArgument(id), _) => {
                result[*id as usize] = Some(actual.clone());
            }
            (Type::Object(generic_object_argument), Type::Object(actual_object_argument)) => {
                for (key, generic_value_type) in generic_object_argument {
                    self.get_generic_arguments_values_into_dict(
                        generic_value_type,
                        actual_object_argument.get(key).ok_or_else(|| {
                            anyhow!(
                                "Actual type {actual:?} does not match generic type {generic:?} \
                                 because generic type contains key {key:?} while actual type is \
                                 not"
                            )
                        })?,
                        result,
                    )
                    .with_context(|| {
                        format!(
                            "Actual {actual:?} does not match generic type {generic:?} because \
                             actual type value type at key {key:?} does not match that of generic \
                             type"
                        )
                    })?;
                }
            }
            (Type::Array(generic_array_argument), Type::Array(actual_array_argument)) => {
                self.get_generic_arguments_values_into_dict(
                    generic_array_argument,
                    actual_array_argument,
                    result,
                )
                .with_context(|| {
                    format!("Actual {actual:?} does not match generic type {generic:?}")
                })?;
            }
            (Type::Number, Type::Number) => {}
            (Type::String, Type::String) => {}
            (Type::Bool, Type::Bool) => {}
            (Type::Null, Type::Null) => {}
            _ => {
                return Err(anyhow!(
                    "Actual type {actual:?} does not match generic type {generic:?}"
                ));
            }
        }
        Ok(())
    }

    fn substitute_generic_arguments_values(
        &self,
        generic: &mut Type,
        values: &[Option<Type>; 256],
    ) -> Result<()> {
        match generic {
            Type::GenericArgument(id) => {
                *generic = values.get(*id as usize).unwrap().clone().with_context(|| {
                    format!(
                        "Can not resolve generic argument {id:?} from other generic-actual types"
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
                for (alias_name, alias_value) in with_clause.with.iter() {
                    context
                        .aliases
                        .entry(alias_name.clone())
                        .or_default()
                        .push(alias_value.clone());
                }
                let result = self.compute_with_context(with_clause.compute.clone(), context)?;
                for alias_name in with_clause.with.keys() {
                    context.aliases.entry(alias_name.clone()).and_modify(
                        |aliases_with_this_name| {
                            aliases_with_this_name.pop();
                        },
                    );
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
                for element in array.iter() {
                    context
                        .aliases
                        .entry("_".to_string())
                        .or_default()
                        .push(element.clone());
                    result.push(self.compute_with_context(map_clause.through.clone(), context)?);
                    context
                        .aliases
                        .entry("_".to_string())
                        .and_modify(|aliases_with_this_name| {
                            aliases_with_this_name.pop();
                        });
                }
                Arc::new(Value::Array(result))
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
                                context
                                    .aliases
                                    .entry("_".to_string())
                                    .or_default()
                                    .push(arguments.clone());
                            } else {
                                for (alias_name, alias_value) in aliases.iter() {
                                    aliases_names.push(alias_name.clone());
                                    context
                                        .aliases
                                        .entry(alias_name.clone())
                                        .or_default()
                                        .push(alias_value.clone());
                                }
                            }
                        } else {
                            aliases_names.push("_".to_string());
                            context
                                .aliases
                                .entry("_".to_string())
                                .or_default()
                                .push(arguments.clone());
                        }
                        context.path.push(name.clone());
                        let result = self.compute_with_context(aliased_value, context)?;
                        context.path.pop();
                        for alias_name in aliases_names {
                            context.aliases.entry(alias_name.clone()).and_modify(
                                |aliases_with_this_name| {
                                    aliases_with_this_name.pop();
                                },
                            );
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
                        result_map.insert(
                            key.clone(),
                            self.compute_with_context(value.clone(), context)?,
                        );
                    }
                    Arc::new(Value::Object(result_map))
                }
            }
            Value::Array(ref array) => {
                let mut result_array = vec![];
                for array_element in array.iter() {
                    result_array.push(self.compute_with_context(array_element.clone(), context)?)
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
                    self.compute_with_context(aliased_value, context)?
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
            },
        )
    }

    fn get_type(&self, program: TypeOrValue, context: &mut TypeCheckingContext) -> Result<Type> {
        Ok(match program {
            TypeOrValue::Type(program_type) => program_type.clone(),
            TypeOrValue::Value(program) => match *program {
                Value::With(ref with_clause) => {
                    for (alias_name, alias_value) in with_clause.with.iter() {
                        context
                            .aliases
                            .entry(alias_name.clone())
                            .or_default()
                            .push(TypeOrValue::Value(alias_value.clone()));
                    }
                    let result =
                        self.get_type(TypeOrValue::Value(with_clause.compute.clone()), context)?;
                    for alias_name in with_clause.with.keys() {
                        context.aliases.entry(alias_name.clone()).and_modify(
                            |aliases_with_this_name| {
                                aliases_with_this_name.pop();
                            },
                        );
                    }
                    result
                }
                Value::Map(ref map_clause) => {
                    let actual_array_type =
                        self.get_type(TypeOrValue::Value(map_clause.map.clone()), context)?;
                    if let Type::Array(ref array_element_type) = actual_array_type {
                        context
                            .aliases
                            .entry("_".to_string())
                            .or_default()
                            .push(TypeOrValue::Type(*array_element_type.clone()));
                        let result =
                            self.get_type(TypeOrValue::Value(map_clause.through.clone()), context)?;
                        context.aliases.entry("_".to_string()).and_modify(
                            |aliases_with_this_name| {
                                aliases_with_this_name.pop();
                            },
                        );
                        Type::Array(Box::new(result))
                    } else {
                        return Err(anyhow!(
                            "Expected array for map clause at path {:?}, got {actual_array_type:?}",
                            context.path
                        ));
                    }
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
                                    context
                                        .aliases
                                        .entry("_".to_string())
                                        .or_default()
                                        .push(TypeOrValue::Value(arguments.clone()));
                                } else {
                                    for (alias_name, alias_value) in aliases.iter() {
                                        aliases_names.push(alias_name.clone());
                                        context
                                            .aliases
                                            .entry(alias_name.clone())
                                            .or_default()
                                            .push(TypeOrValue::Value(alias_value.clone()));
                                    }
                                }
                            } else {
                                aliases_names.push("_".to_string());
                                context
                                    .aliases
                                    .entry("_".to_string())
                                    .or_default()
                                    .push(TypeOrValue::Value(arguments.clone()));
                            }
                            context.path.push(name.clone());
                            let result = self.get_type(aliased_value, context)?;
                            context.path.pop();
                            for alias_name in aliases_names {
                                context.aliases.entry(alias_name.clone()).and_modify(
                                    |aliases_with_this_name| {
                                        aliases_with_this_name.pop();
                                    },
                                );
                            }
                            return Ok(result);
                        }
                        if let Some(function) = self.supported_functions.get(name) {
                            context.path.push(name.clone());
                            let arguments_type =
                                self.get_type(TypeOrValue::Value(arguments.clone()), context)?;
                            let generic_arguments_values = &self.get_generic_arguments_values(
                                &function.argument_type,
                                &arguments_type,
                            )?;
                            let concrete_arguments_type = {
                                let mut result = function.argument_type.clone();
                                self.substitute_generic_arguments_values(
                                    &mut result,
                                    generic_arguments_values,
                                )?;
                                result
                            };
                            let concrete_return_type = {
                                let mut result = function.return_type.clone();
                                self.substitute_generic_arguments_values(
                                    &mut result,
                                    generic_arguments_values,
                                )?;
                                result
                            };
                            if arguments_type != concrete_arguments_type {
                                return Err(anyhow!(
                                    "Expected argument of type {:?} for function at path {:?}, \
                                     but got {arguments_type:?}",
                                    &function.argument_type,
                                    context.path
                                ));
                            }
                            context.path.pop();
                            concrete_return_type
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
                            result_map.insert(
                                key.clone(),
                                self.get_type(TypeOrValue::Value(value.clone()), context)?,
                            );
                        }
                        Type::Object(result_map)
                    }
                }
                Value::Array(ref array) => {
                    let array_element_type = self.get_type(
                        TypeOrValue::Value(
                            array
                                .first()
                                .ok_or_else(|| {
                                    anyhow!("Expected non-empty array at path {:?}", context.path)
                                })?
                                .clone(),
                        ),
                        context,
                    )?;
                    for (array_element_index, array_element) in array[1..].iter().enumerate() {
                        context.path.push(array_element_index.to_string());
                        let current_array_element_type =
                            self.get_type(TypeOrValue::Value(array_element.clone()), context)?;
                        context.path.pop();
                        if current_array_element_type != array_element_type {
                            return Err(anyhow!(
                                "Expected all elements of array at path {:?} to be of type \
                                 {array_element_type:?} (as first element), but got element of \
                                 type {current_array_element_type:?} at index {}",
                                context.path,
                                array_element_index + 1
                            ));
                        }
                    }
                    Type::Array(Box::new(array_element_type))
                }
                Value::String(ref string) => {
                    if let Some(aliased_value) = context
                        .aliases
                        .get(string)
                        .and_then(|values_for_this_name| values_for_this_name.last())
                        .cloned()
                    {
                        self.get_type(aliased_value.clone(), context)?
                    } else {
                        Type::String
                    }
                }
                Value::Number(_) => Type::Number,
                Value::Bool(_) => Type::Bool,
                Value::Null => Type::Null,
            },
        })
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
                            {"MULTIPLY": [2, 3]},
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
                                "WITH": {"x": 2, "y": 3},
                                "COMPUTE": {"MULTIPLY": ["x", "x", "y"]}
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
                                    "SQUARE": {"MULTIPLY": ["_", "_"]},
                                    "y": 3
                                },
                                "COMPUTE": {"MULTIPLY": [
                                    {"SQUARE": {"_": 2}},
                                    {
                                        "SQUARE": {
                                            "SQUARE": {
                                                "MULTIPLY": [
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
                            ],
                            "THROUGH": {"SUM": ["_", 1]}
                        }
                    }))
                    .unwrap()
                ))
                .unwrap(),
            Value::Number(5.0)
        );
    }
}
