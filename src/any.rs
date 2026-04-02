use anyhow::Result;
use paste::paste;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::define_types_functions;

define_types_functions!(
    computed Number is f64 {
        Sum {
            terms: Vec<ValueNumber>
        } self {
            let mut result = 0f64;
            for term in self.terms.iter() {
                result += term.compute()?;
            }
            Ok(result)
        }
        Multiply {
            terms: Vec<ValueNumber>
        } self {
            let mut result = 1f64;
            for term in self.terms.iter() {
                result *= term.compute()?;
            }
            Ok(result)
        }
    }
    computed String is String {
        Concat {
            strings: Vec<ValueString>
        } self {
            let mut result = "".to_string();
            for string in self.strings.iter() {
                result += &string.compute()?;
            }
            Ok(result)
        }
        Repeat {
            string: Box<ValueString>
            amount: ValueNumber
        } self {
            let string = self.string.compute()?;
            let amount = self.amount.compute()? as usize;
            Ok(string.repeat(amount))
        }
    }
    computed StringArray is Vec<String> {
        Split {
            string: ValueString
            delimiter: ValueString
        } self {
            let string = self.string.compute()?;
            let delimiter = self.delimiter.compute()?;
            Ok(string.split(&delimiter).map(|s| s.to_string()).collect())
        }
    }
);

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
