use std::collections::BTreeMap;

use anyhow::{anyhow, Result};

#[derive(PartialEq, Debug)]
pub enum Type {
    Number,
    String,
    Bool,
    Null,
    Array(Box<Type>),
    Object(BTreeMap<String, Type>),
}

pub struct Function {
    pub argument_type: Type,
    pub return_type: Type,
    pub function: fn(&serde_json::Value) -> Result<serde_json::Value>,
}

pub struct Interpreter {
    pub supported_functions: BTreeMap<String, Function>,
}

pub fn sum(argument: &serde_json::Value) -> Result<serde_json::Value> {
    let mut result = 0f64;
    for element in argument.as_array().unwrap().iter() {
        result += element.as_number().unwrap().as_f64().unwrap();
    }
    Ok(serde_json::to_value(result).unwrap())
}

impl Default for Interpreter {
    fn default() -> Interpreter {
        Interpreter {
            supported_functions: BTreeMap::from([(
                "SUM".to_string(),
                Function {
                    argument_type: Type::Array(Box::new(Type::Number)),
                    return_type: Type::Number,
                    function: sum,
                },
            )]),
        }
    }
}

pub struct TypeCheckingContext {
    pub path: Vec<String>,
}

impl Interpreter {
    pub fn assert_type(&self, program: &serde_json::Value, expected_type: &Type) -> Result<()> {
        self.assert_type_with_context(
            program,
            expected_type,
            &mut TypeCheckingContext { path: vec![] },
        )
    }

    fn assert_type_with_context(
        &self,
        program: &serde_json::Value,
        expected_type: &Type,
        context: &mut TypeCheckingContext,
    ) -> Result<()> {
        match program {
            serde_json::Value::Object(object) => {
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
            serde_json::Value::Array(array) => {
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
            serde_json::Value::Number(number) => {
                if expected_type != &Type::Number {
                    return Err(anyhow!(
                        "Expected type {expected_type:?} at path {:?}, but got number {number:?}",
                        context.path
                    ));
                }
            }
            serde_json::Value::String(string) => {
                if expected_type != &Type::String {
                    return Err(anyhow!(
                        "Expected type {expected_type:?} at path {:?}, but got string {string:?}",
                        context.path
                    ));
                }
            }
            serde_json::Value::Bool(bool) => {
                if expected_type != &Type::Bool {
                    return Err(anyhow!(
                        "Expected type {expected_type:?} at path {:?}, but got boolean {bool:?}",
                        context.path
                    ));
                }
            }
            serde_json::Value::Null => {
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
                &json!({
                    "SUM": [
                        {"SUM": [1, 2]},
                        3
                    ]
                }),
                &Type::Number,
            )
            .unwrap();
    }
}
