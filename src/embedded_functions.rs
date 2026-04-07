use std::collections::BTreeMap;

use anyhow::Result;
use paste::paste;

use crate::{define_default_interpreter_supported_functions, Function, Interpreter, Type, Value};

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
