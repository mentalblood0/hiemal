use std::collections::BTreeMap;

use crate::function::Function;
use crate::interpreter::Interpreter;
use crate::r#type::Type;
use crate::value::{RcOrValue, Value};

use dashu::Rational;

impl Default for Interpreter {
    fn default() -> Interpreter {
        Interpreter {
            supported_functions: BTreeMap::from([
                (
                    "SUM".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::Number)),
                        return_type: Type::Number,
                        function: |argument: RcOrValue| {
                            Ok(RcOrValue::Value(Value::Number(
                                argument
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|element| element.as_number().unwrap())
                                    .fold(Rational::ZERO, |accumulator, current| {
                                        accumulator + current
                                    }),
                            )))
                        },
                    },
                ),
                (
                    "PRODUCT".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::Number)),
                        return_type: Type::Number,
                        function: |argument: RcOrValue| {
                            Ok(RcOrValue::Value(Value::Number(
                                argument
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|element| element.as_number().unwrap())
                                    .fold(Rational::ONE, |accumulator, current| {
                                        accumulator * current
                                    }),
                            )))
                        },
                    },
                ),
                (
                    "LEN".to_string(),
                    Function {
                        argument_type: Type::String,
                        return_type: Type::Number,
                        function: |argument: RcOrValue| {
                            Ok(RcOrValue::Value(Value::Number(Rational::from(
                                argument.as_string().unwrap().len(),
                            ))))
                        },
                    },
                ),
                (
                    "SIZE".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::GenericArgument(0))),
                        return_type: Type::Number,
                        function: |argument: RcOrValue| {
                            Ok(RcOrValue::Value(Value::Number(Rational::from(
                                argument.as_array().unwrap().len(),
                            ))))
                        },
                    },
                ),
                (
                    "IS_SORTED".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::Number)),
                        return_type: Type::Bool,
                        function: |argument: RcOrValue| {
                            Ok(RcOrValue::Value(Value::Bool(
                                argument
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|element| element.as_number().unwrap())
                                    .is_sorted(),
                            )))
                        },
                    },
                ),
                (
                    "ARE_EQUAL".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::GenericArgument(0))),
                        return_type: Type::Bool,
                        function: |argument: RcOrValue| {
                            let array = argument.as_array().unwrap();
                            Ok(RcOrValue::Value(Value::Bool(
                                array
                                    .get(0)
                                    .map_or(true, |first| array.iter().all(|x| x == first)),
                            )))
                        },
                    },
                ),
                (
                    "CONCAT".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::String)),
                        return_type: Type::String,
                        function: |argument: RcOrValue| {
                            let mut result = String::new();
                            for element in argument.as_array().unwrap().iter() {
                                result += element.as_string().unwrap();
                            }
                            Ok(RcOrValue::Value(Value::String(
                                argument
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|element| element.as_string().unwrap())
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(""),
                            )))
                        },
                    },
                ),
                (
                    "SEQUENCE".to_string(),
                    Function {
                        argument_type: Type::Object(BTreeMap::from([
                            ("from".to_string(), Type::Number),
                            ("to".to_string(), Type::Number),
                            ("step".to_string(), Type::Number),
                        ])),
                        return_type: Type::Array(Box::new(Type::Number)),
                        function: |argument: RcOrValue| {
                            let arguments = argument.as_object().unwrap();
                            let from = arguments.get("from").unwrap().as_number().unwrap();
                            let to = arguments.get("to").unwrap().as_number().unwrap();
                            let step = arguments.get("step").unwrap().as_number().unwrap();
                            let estimated_capacity = (to.clone() - from.clone()) / step.clone();
                            if estimated_capacity <= Rational::ZERO {
                                Ok(RcOrValue::Value(Value::Array(vec![])))
                            } else {
                                let mut result =
                                    Vec::with_capacity(estimated_capacity.to_f64_fast() as usize);
                                let mut current = from;
                                while current <= to {
                                    result.push(RcOrValue::Value(Value::Number(current.clone())));
                                    current += step.clone();
                                }
                                Ok(RcOrValue::Value(Value::Array(result)))
                            }
                        },
                    },
                ),
            ]),
        }
    }
}
