use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use paste::paste;

#[derive(PartialEq, Debug)]
pub enum Type {
    Number,
    String,
    Bool,
    Null,
    Array(Box<Type>),
    Object(BTreeMap<String, Type>),
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Debug)]
#[serde(untagged)]
pub enum Value {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
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

    pub fn as_object(&self) -> Option<&BTreeMap<String, Value>> {
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
}

impl Interpreter {
    pub fn assert_type(&self, program: &Value, expected_type: &Type) -> Result<()> {
        self.assert_type_with_context(
            program,
            expected_type,
            &mut TypeCheckingContext { path: vec![] },
        )
    }

    fn assert_type_with_context(
        &self,
        program: &Value,
        expected_type: &Type,
        context: &mut TypeCheckingContext,
    ) -> Result<()> {
        match program {
            Value::Object(object) => {
                if object.len() == 1 {
                    let (function_name, function_argument) = object.iter().next().unwrap();
                    if let Some(function) = self.supported_functions.get(function_name) {
                        if expected_type != &function.return_type {
                            return Err(anyhow!(
                                "Expected type {expected_type:?} at path {:?}, but got function \
                                 {function_name:?} which returns {:?}",
                                context.path,
                                function.return_type
                            ));
                        }
                        context.path.push(function_name.clone());
                        self.assert_type_with_context(
                            function_argument,
                            &function.argument_type,
                            context,
                        )?;
                        context.path.pop();
                    } else {
                        return Err(anyhow!(
                            "Expected supported function at path {:?}, but got unsupported \
                             function {function_name:?}. Supported functions are: {:?}",
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
                        "Expected type {expected_type:?} at path {:?}, but got array {array:?}",
                        context.path
                    ));
                }
            }
            Value::Number(number) => {
                if expected_type != &Type::Number {
                    return Err(anyhow!(
                        "Expected type {expected_type:?} at path {:?}, but got number {number:?}",
                        context.path
                    ));
                }
            }
            Value::String(string) => {
                if expected_type != &Type::String {
                    return Err(anyhow!(
                        "Expected type {expected_type:?} at path {:?}, but got string {string:?}",
                        context.path
                    ));
                }
            }
            Value::Bool(bool) => {
                if expected_type != &Type::Bool {
                    return Err(anyhow!(
                        "Expected type {expected_type:?} at path {:?}, but got boolean {bool:?}",
                        context.path
                    ));
                }
            }
            Value::Null => {
                if expected_type != &Type::Null {
                    return Err(anyhow!(
                        "Expected type {expected_type:?} at path {:?}, but got null",
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
    }
}
