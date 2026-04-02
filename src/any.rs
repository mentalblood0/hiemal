use anyhow::Result;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::array::ValueStringArray;
use crate::number::ValueNumber;
use crate::string::ValueString;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum ValueAny {
    Number(ValueNumber),
    String(ValueString),
    StringArray(ValueStringArray),
    Array(Vec<ValueAny>),
    Object(BTreeMap<String, ValueAny>),
}

impl ValueAny {
    pub fn compute(&self) -> Result<serde_json::Value> {
        Ok(match self {
            ValueAny::Number(value) => serde_json::to_value(value.compute()?)?,
            ValueAny::String(value) => serde_json::to_value(value.compute()?)?,
            ValueAny::StringArray(value) => serde_json::to_value(value.compute()?)?,
            ValueAny::Array(array) => {
                let mut result = vec![];
                for value_any in array {
                    result.push(value_any.compute()?);
                }
                serde_json::Value::Array(result)
            }
            ValueAny::Object(map) => {
                let mut result = serde_json::Map::new();
                for (key, value_any) in map {
                    result.insert(key.clone(), value_any.compute()?);
                }
                serde_json::Value::Object(result)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::any::ValueAny;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn execute_and_assert<'a>(
        program_structure: serde_json::Value,
        correct_result: serde_json::Value,
    ) {
        let program: ValueAny = serde_json::from_value(program_structure).unwrap();
        let result = serde_json::to_value(program.compute().unwrap()).unwrap();
        assert_eq!(result, correct_result);
    }

    #[test]
    fn test_examples() {
        execute_and_assert(
            json!({
                "Sum": {
                    "terms": [
                        {
                            "Sum": {
                                "terms": [1, 2]
                            }
                        },
                        3
                    ]
                }
            }),
            json!(6.0),
        );
        execute_and_assert(
            json!({
                "Sum": {
                    "terms": [
                        {
                            "Sum": {
                                "terms": [1.2, 2.3]
                            }
                        },
                        3.4
                    ]
                }
            }),
            json!(6.9),
        );
        execute_and_assert(
            json!({
                "Sum": {
                    "terms": [
                        {
                            "Multiply": {
                                "terms": [1, 2]
                            }
                        },
                        3
                    ]
                }
            }),
            json!(5.0),
        );
        execute_and_assert(
            json!({
                "some": [
                    "other",
                    "values"
                ],
                "key": {
                    "Concat": {
                        "strings": [
                            {
                                "Repeat": {
                                    "string": "la",
                                    "amount": {
                                        "Sum": {
                                            "terms": [0, 2]
                                        }
                                    }
                                }
                            },
                            "lo"
                        ]
                    }
                }
            }),
            json!({
                "some": [
                    "other",
                    "values"
                ],
                "key": "lalalo"
            }),
        );
        execute_and_assert(
            json!({
                "Split": {
                    "string": "la la la",
                    "delimiter": " "
                }
            }),
            json!(["la", "la", "la"]),
        );
    }
}
