use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use paste::paste;

use crate::{define_default_interpreter_supported_functions, Function, Interpreter, Type, Value};

define_default_interpreter_supported_functions!(
    SUM Type::Array(Box::new(Type::Number)), Type::Number, argument {
        Ok(Arc::new(Value::Number(
            argument
                .as_array()
                .unwrap()
                .iter()
                .map(|element| element.as_number().unwrap())
                .sum()
        )))
    }
    PRODUCT Type::Array(Box::new(Type::Number)), Type::Number, argument {
        Ok(Arc::new(Value::Number(
            argument
                .as_array()
                .unwrap()
                .iter()
                .map(|element| element.as_number().unwrap())
                .product()
        )))
    }
    LEN Type::String, Type::Number, argument {
        Ok(Arc::new(Value::Number(
            argument
                .as_string()
                .unwrap()
                .len() as f64
        )))
    }
    SIZE Type::Array(Box::new(Type::GenericArgument(0))), Type::Number, argument {
        Ok(Arc::new(Value::Number(
            argument
                .as_array()
                .unwrap()
                .len() as f64
        )))
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
    IS_SORTED Type::Array(Box::new(Type::Number)), Type::Bool, argument {
        Ok(Arc::new(Value::Bool(
            argument
                .as_array()
                .unwrap()
                .iter()
                .map(|element| element.as_number().unwrap())
                .is_sorted()
        )))
    }
    ARE_EQUAL Type::Array(Box::new(Type::GenericArgument(0))), Type::Bool, argument {
        let array = argument.as_array().unwrap();
        Ok(Arc::new(Value::Bool(array.get(0).map_or(true, |first| array.iter().all(|x| x == first)))))
    }
    CONCAT Type::Array(Box::new(Type::String)), Type::String, argument {
        let mut result = String::new();
        for element in argument.as_array().unwrap().iter() {
            result += element.as_string().unwrap();
        }
        Ok(Arc::new(Value::String(
            argument
                .as_array()
                .unwrap()
                .iter()
                .map(|element| element.as_string().unwrap())
                .cloned()
                .collect::<Vec<_>>()
                .join("")
        )))
    }
    SEQUENCE Type::Object(BTreeMap::from([
        ("from".to_string(), Type::Number),
        ("to".to_string(), Type::Number),
        ("step".to_string(), Type::Number)
    ])), Type::Array(Box::new(Type::Number)), argument {
        let arguments = argument.as_object().unwrap();
        let from = arguments.get("from").unwrap().as_number().unwrap();
        let to = arguments.get("to").unwrap().as_number().unwrap();
        let step = arguments.get("step").unwrap().as_number().unwrap();
        let estimated_capacity = (to - from) / step;
        if estimated_capacity <= 0.0 {
            Ok(Arc::new(Value::Array(vec![])))
        } else {
            let mut result = Vec::with_capacity(estimated_capacity as usize);
            let mut current = from;
            while current <= to {
                result.push(Arc::new(Value::Number(current)));
                current += step;
            }
            Ok(Arc::new(Value::Array(result)))
        }
    }
);
