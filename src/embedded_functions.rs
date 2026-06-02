use std::collections::BTreeMap;

use crate::function::Function;
use crate::interpreter::Interpreter;
use crate::r#type::Type;
use crate::value::Value;

use anyhow::Context;
use dashu::Rational;

impl Default for Interpreter {
    fn default() -> Interpreter {
        Interpreter {
            embedded_functions: BTreeMap::from([
                (
                    "sum".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::Number)),
                        return_type: Type::Number,
                        function: |argument: Value| {
                            Ok(Value::Number(
                                argument
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|element| element.as_number().unwrap())
                                    .fold(Rational::ZERO, |accumulator, current| {
                                        accumulator + current
                                    }),
                            ))
                        },
                    },
                ),
                (
                    "product".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::Number)),
                        return_type: Type::Number,
                        function: |argument: Value| {
                            Ok(Value::Number(
                                argument
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|element| element.as_number().unwrap())
                                    .fold(Rational::ONE, |accumulator, current| {
                                        accumulator * current
                                    }),
                            ))
                        },
                    },
                ),
                (
                    "len".to_string(),
                    Function {
                        argument_type: Type::String,
                        return_type: Type::Number,
                        function: |argument: Value| {
                            Ok(Value::Number(Rational::from(
                                argument.as_string().unwrap().len_chars(),
                            )))
                        },
                    },
                ),
                (
                    "slice".to_string(),
                    Function {
                        argument_type: Type::Object(BTreeMap::from([
                            ("source".to_string(), Type::String),
                            ("from".to_string(), Type::Number),
                            ("to".to_string(), Type::Number),
                        ])),
                        return_type: Type::String,
                        function: |argument: Value| {
                            let arguments = argument.as_object().unwrap();
                            let source = arguments["source"].as_string().unwrap();
                            let from =
                                arguments["from"].as_number().unwrap().to_f64_fast() as usize;
                            let to = arguments["to"].as_number().unwrap().to_f64_fast() as usize;
                            Ok(Value::String(
                                source
                                    .get_slice(from..to)
                                    .with_context(|| {
                                        format!("Can not get slice {from}..{to} from {source:?}")
                                    })?
                                    .into(),
                            ))
                        },
                    },
                ),
                (
                    "size".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::GenericArgument(0))),
                        return_type: Type::Number,
                        function: |argument: Value| {
                            Ok(Value::Number(Rational::from(
                                argument.as_array().unwrap().len(),
                            )))
                        },
                    },
                ),
                (
                    "is sorted".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::Number)),
                        return_type: Type::Bool,
                        function: |argument: Value| {
                            Ok(Value::Bool(
                                argument
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|element| element.as_number().unwrap())
                                    .is_sorted(),
                            ))
                        },
                    },
                ),
                (
                    "are equal".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::GenericArgument(0))),
                        return_type: Type::Bool,
                        function: |argument: Value| {
                            let array = argument.as_array().unwrap();
                            Ok(Value::Bool(
                                array
                                    .get(0)
                                    .map_or(true, |first| array.iter().all(|x| x == first)),
                            ))
                        },
                    },
                ),
                (
                    "concat".to_string(),
                    Function {
                        argument_type: Type::Array(Box::new(Type::String)),
                        return_type: Type::String,
                        function: |argument: Value| {
                            let mut result = ropey::Rope::new();
                            for element in argument.as_array().unwrap().iter() {
                                result.append(element.as_string().unwrap().clone());
                            }
                            Ok(Value::String(result))
                        },
                    },
                ),
                (
                    "sequence".to_string(),
                    Function {
                        argument_type: Type::Object(BTreeMap::from([
                            ("from".to_string(), Type::Number),
                            ("to".to_string(), Type::Number),
                            ("step".to_string(), Type::Number),
                        ])),
                        return_type: Type::Array(Box::new(Type::Number)),
                        function: |argument: Value| {
                            let arguments = argument.as_object().unwrap();
                            let from = arguments.get("from").unwrap().as_number().unwrap();
                            let to = arguments.get("to").unwrap().as_number().unwrap();
                            let step = arguments.get("step").unwrap().as_number().unwrap();
                            let estimated_capacity = (to.clone() - from.clone()) / step.clone();
                            if estimated_capacity <= Rational::ZERO {
                                Ok(Value::Array(rpds::VectorSync::new_sync()))
                            } else {
                                let mut result = rpds::VectorSync::new_sync();
                                let mut current = from;
                                while current <= to {
                                    result.push_back_mut(Value::Number(current.clone()));
                                    current += step.clone();
                                }
                                Ok(Value::Array(result))
                            }
                        },
                    },
                ),
            ]),
        }
    }
}
