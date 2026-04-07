use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use paste::paste;

#[derive(PartialEq, Debug)]
pub enum Type {
    Number,
    String,
    Bool,
    Null,
    Array(Box<Type>),
    AnyObject,
    Object(BTreeMap<String, Type>),
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug)]
pub struct With {
    with: BTreeMap<String, Arc<Value>>,
    compute: Box<Value>,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug)]
#[serde(untagged)]
pub enum Value {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    Array(Vec<Value>),
    With(With),
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

    pub fn as_array(&self) -> Option<&Vec<Value>> {
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
    pub function: fn(&Value) -> Result<Value>,
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
                pub fn [<$function_name:lower>]($function_argument: &Value) -> Result<Value> $function_code
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

define_default_interpreter_supported_functions!(
    SUM Type::Array(Box::new(Type::Number)), Type::Number, argument {
        let mut result = 0f64;
        for element in argument.as_array().unwrap().iter() {
            result += element.as_number().unwrap();
        }
        Ok(Value::Number(result))
    }
    MULTIPLY Type::Array(Box::new(Type::Number)), Type::Number, argument {
        let mut result = 1f64;
        for element in argument.as_array().unwrap().iter() {
            result *= element.as_number().unwrap();
        }
        Ok(Value::Number(result))
    }
    LEN Type::String, Type::Number, argument {
        let result = argument.as_string().unwrap().len() as f64;
        Ok(Value::Number(result))
    }
    CONCAT Type::Array(Box::new(Type::String)), Type::String, argument {
        let mut result = String::new();
        for element in argument.as_array().unwrap().iter() {
            result += element.as_string().unwrap();
        }
        Ok(Value::String(result))
    }
);

pub struct TypeCheckingContext {
    pub path: Vec<String>,
    pub aliases: BTreeMap<String, Vec<Arc<Value>>>,
}

impl Interpreter {
    pub fn assert_type(&self, program: &Value, expected_type: &Type) -> Result<()> {
        self.assert_type_with_context(
            program,
            expected_type,
            &mut TypeCheckingContext {
                path: vec![],
                aliases: BTreeMap::new(),
            },
        )
    }

    fn assert_type_with_context(
        &self,
        program: &Value,
        expected_type: &Type,
        context: &mut TypeCheckingContext,
    ) -> Result<()> {
        match program {
            Value::With(with_clause) => {
                for (alias_name, alias_value) in with_clause.with.iter() {
                    context
                        .aliases
                        .entry(alias_name.clone())
                        .or_default()
                        .push(alias_value.clone());
                }
                self.assert_type_with_context(&with_clause.compute, expected_type, context)?;
                for alias_name in with_clause.with.keys() {
                    context.aliases.entry(alias_name.clone()).and_modify(
                        |aliases_with_this_name| {
                            aliases_with_this_name.pop();
                        },
                    );
                }
            }
            Value::Object(object) => {
                if object.len() == 1 {
                    let (name, arguments) = object.iter().next().unwrap();
                    if let Value::Object(ref aliases) = **arguments {
                        if let Some(aliased_value) = context
                            .aliases
                            .get(name)
                            .and_then(|aliases_with_this_name| aliases_with_this_name.last())
                            .cloned()
                        {
                            for (alias_name, alias_value) in aliases.iter() {
                                context
                                    .aliases
                                    .entry(alias_name.clone())
                                    .or_default()
                                    .push(alias_value.clone());
                            }
                            context.path.push(name.clone());
                            self.assert_type_with_context(&aliased_value, expected_type, context)?;
                            context.path.pop();
                            for alias_name in aliases.keys() {
                                context.aliases.entry(alias_name.clone()).and_modify(
                                    |aliases_with_this_name| {
                                        aliases_with_this_name.pop();
                                    },
                                );
                            }
                            return Ok(());
                        }
                    }
                    if let Some(function) = self.supported_functions.get(name) {
                        if expected_type != &function.return_type {
                            return Err(anyhow!(
                                "Expected type {expected_type:?} at path {:?}, but got function \
                                 {name:?} which returns {:?}",
                                context.path,
                                function.return_type
                            ));
                        }
                        context.path.push(name.clone());
                        self.assert_type_with_context(arguments, &function.argument_type, context)?;
                        context.path.pop();
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
                    if let Type::Object(object_keys_types) = expected_type {
                        for expected_key in object_keys_types.keys() {
                            if !object.contains_key(expected_key) {
                                return Err(anyhow!(
                                    "Expected key {expected_key:?} in object at path {:?}",
                                    context.path
                                ));
                            }
                        }
                        for (key, expected_key_type) in object_keys_types {
                            self.assert_type_with_context(
                                object.get(key).unwrap(),
                                expected_key_type,
                                context,
                            )?;
                        }
                    } else {
                        return Err(anyhow!(
                            "Expected type {expected_type:?} at path {:?}, but got object \
                             {object:?}",
                            context.path
                        ));
                    }
                }
            }
            Value::Array(array) => {
                if let Type::Array(expected_array_element_type) = expected_type {
                    for (element_index, element) in array.iter().enumerate() {
                        context.path.push(element_index.to_string());
                        self.assert_type_with_context(
                            element,
                            &expected_array_element_type,
                            context,
                        )?;
                        context.path.pop();
                    }
                } else {
                    return Err(anyhow!(
                        "Expected type {expected_type:?} at path {:?}, but got {program:?}",
                        context.path
                    ));
                }
            }
            Value::Number(_) => {
                if expected_type != &Type::Number {
                    return Err(anyhow!(
                        "Expected value of type {expected_type:?} at path {:?}, but got \
                         {program:?}",
                        context.path
                    ));
                }
            }
            Value::String(string) => {
                if let Some(aliased_value) = context
                    .aliases
                    .get(string)
                    .and_then(|values_for_this_name| values_for_this_name.last())
                    .cloned()
                {
                    self.assert_type_with_context(&aliased_value, expected_type, context)?;
                } else if expected_type != &Type::String {
                    return Err(anyhow!(
                        "Expected value of type {expected_type:?} at path {:?}, but got \
                         {program:?}",
                        context.path
                    ));
                }
            }
            Value::Bool(_) => {
                if expected_type != &Type::Bool {
                    return Err(anyhow!(
                        "Expected value of type {expected_type:?} at path {:?}, but got \
                         {program:?}",
                        context.path
                    ));
                }
            }
            Value::Null => {
                if expected_type != &Type::Null {
                    return Err(anyhow!(
                        "Expected value of type {expected_type:?} at path {:?}, but got \
                         {program:?}",
                        context.path
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_examples() {
        let interpreter = Interpreter::default();
        interpreter
            .assert_type(
                &serde_json::from_value(json!({
                    "SUM": [
                        {"MULTIPLY": [2, 3]},
                        {"LEN": {"CONCAT": ["lala", "lolo"]}},
                        4
                    ]
                }))
                .unwrap(),
                &Type::Number,
            )
            .unwrap();
        interpreter
            .assert_type(
                &serde_json::from_value(json!({
                    "SUM": [
                        {
                            "with": {"x": 2, "y": 3},
                            "compute": {"MULTIPLY": ["x", "x", "y"]}
                        },
                        {"LEN": {"CONCAT": ["lala", "lolo"]}},
                        4
                    ]
                }))
                .unwrap(),
                &Type::Number,
            )
            .unwrap();
        interpreter
            .assert_type(
                &serde_json::from_value(json!({
                    "SUM": [
                        {
                            "with": {
                                "SQUARE": {"MULTIPLY": ["x", "x"]},
                                "y": 3
                            },
                            "compute": {"MULTIPLY": [
                                {"SQUARE": {"x": 2}},
                                "y"
                            ]}
                        },
                        {"LEN": {"CONCAT": ["lala", "lolo"]}},
                        4
                    ]
                }))
                .unwrap(),
                &Type::Number,
            )
            .unwrap();
    }
}
