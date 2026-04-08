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
        Ok(Value::Number(argument.as_string().unwrap().len() as f64))
    }
    SIZE Type::Array(Box::new(Type::GenericArgument(0))), Type::Number, argument {
        Ok(Value::Number(argument.as_array().unwrap().len() as f64))
    }
    GET_ELEMENT Type::Object(BTreeMap::from([
        ("from".to_string(), Type::Array(Box::new(Type::GenericArgument(0)))),
        ("at".to_string(), Type::Number)
    ])), Type::GenericArgument(0), argument {
        let arguments = argument.as_object().unwrap();
        let array = arguments.get("from").unwrap().as_array().unwrap();
        let index = arguments.get("at").unwrap().as_number().unwrap() as usize;
        Ok(array.get(index).unwrap().clone())
    }
    CONCAT Type::Array(Box::new(Type::String)), Type::String, argument {
        let mut result = String::new();
        for element in argument.as_array().unwrap().iter() {
            result += element.as_string().unwrap();
        }
        Ok(Value::String(result))
    }
);
