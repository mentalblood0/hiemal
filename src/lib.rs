use anyhow::Result;
use paste::paste;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[macro_export]
macro_rules! define_types_functions {
    (
        $(
            computed $type_name:ident is $raw_type:ty, default is $raw_type_default:expr;
            {
                $(
                    $function_name:ident {
                        $(
                            $function_argument_name:ident: $function_argument_type:ty
                        )+
                    } $function_self:ident $function_code:block
                )+
            }
        )+
    ) => {
        paste! {
            $(
                #[derive(Serialize, Deserialize, Debug, PartialEq)]
                #[serde(rename_all="SCREAMING_SNAKE_CASE")]
                #[serde(deny_unknown_fields)]
                pub enum [<Computable $type_name>] {
                    $(
                        $function_name($function_name),
                    )+
                }

                #[derive(Serialize, Deserialize, Debug, PartialEq)]
                #[serde(untagged)]
                pub enum [<ComputableOrRaw $type_name>] {
                    Computable([<Computable $type_name>]),
                    Raw($raw_type),
                }

                impl Default for [<ComputableOrRaw $type_name>] {
                    fn default() -> Self {
                        Self::Raw($raw_type_default)
                    }
                }

                $(
                    #[derive(Serialize, Deserialize, Debug, PartialEq)]
                    pub struct $function_name {
                        $(
                            $function_argument_name: $function_argument_type,
                        )+
                    }

                    impl $function_name {
                        pub fn compute($function_self) -> Result<$raw_type> $function_code
                    }
                )+

                impl [<ComputableOrRaw $type_name>] {
                    pub fn compute(self) -> Result<$raw_type> {
                        match self {
                            [<ComputableOrRaw $type_name>]::Computable(computable) => match computable {
                                $(
                                    [<Computable $type_name>]::$function_name(computable) => computable.compute(),
                                )+
                            }
                            [<ComputableOrRaw $type_name>]::Raw(raw_value) => Ok(raw_value.clone()),
                        }
                    }
                }
            )+

            #[derive(Serialize, Deserialize, Debug, PartialEq)]
            #[serde(untagged)]
            #[serde(deny_unknown_fields)]
            pub enum ComputableOrRawAny {
                $(
                    $type_name([<ComputableOrRaw $type_name>]),
                )+
                TransparentArray(Vec<ComputableOrRawAny>),
                TransparentObject(BTreeMap<String, ComputableOrRawAny>),
            }

            impl ComputableOrRawAny {
                pub fn compute(self) -> Result<serde_json::Value> {
                    Ok(match self {
                        $(
                            ComputableOrRawAny::$type_name(value) => serde_json::to_value(value.compute()?)?,
                        )+
                        ComputableOrRawAny::TransparentArray(array) => {
                            let mut result = vec![];
                            for value_any in array {
                                result.push(value_any.compute()?);
                            }
                            serde_json::Value::Array(result)
                        }
                        ComputableOrRawAny::TransparentObject(map) => {
                            let mut result = serde_json::Map::new();
                            for (key, value_any) in map {
                                result.insert(key.clone(), value_any.compute()?);
                            }
                            serde_json::Value::Object(result)
                        }
                    })
                }
            }
        }
    };
}

define_types_functions!(
    computed Number is f64, default is 0f64; {
        Sum {
            terms: Vec<ComputableOrRawNumber>
        } self {
            let mut result = 0f64;
            for mut term in self.terms {
                result += std::mem::take(&mut term).compute()?;
            }
            Ok(result)
        }
        Multiply {
            terms: Vec<ComputableOrRawNumber>
        } self {
            let mut result = 1f64;
            for mut term in self.terms {
                result *= std::mem::take(&mut term).compute()?;
            }
            Ok(result)
        }
    }
    computed String is String, default is "".to_string(); {
        Concat {
            strings: Vec<ComputableOrRawString>
        } self {
            let mut result = "".to_string();
            for string in self.strings {
                result += &string.compute()?;
            }
            Ok(result)
        }
        Repeat {
            string: Box<ComputableOrRawString>
            amount: ComputableOrRawNumber
        } self {
            let string = self.string.compute()?;
            let amount = self.amount.compute()? as usize;
            Ok(string.repeat(amount))
        }
    }
    computed StringArray is Vec<String>, default is vec![]; {
        Split {
            string: ComputableOrRawString
            delimiter: ComputableOrRawString
        } self {
            let string = self.string.compute()?;
            let delimiter = self.delimiter.compute()?;
            Ok(string.split(&delimiter).map(|s| s.to_string()).collect())
        }
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn execute_and_assert<'a>(
        program_structure: serde_json::Value,
        correct_result: serde_json::Value,
    ) {
        let program: ComputableOrRawAny = serde_json::from_value(program_structure).unwrap();
        let result = serde_json::to_value(program.compute().unwrap()).unwrap();
        assert_eq!(result, correct_result);
    }

    #[test]
    fn test_examples() {
        execute_and_assert(
            json!({
                "SUM": {
                    "terms": [
                        {
                            "SUM": {
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
                "SUM": {
                    "terms": [
                        {
                            "SUM": {
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
                "SUM": {
                    "terms": [
                        {
                            "MULTIPLY": {
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
                    "CONCAT": {
                        "strings": [
                            {
                                "REPEAT": {
                                    "string": "la",
                                    "amount": {
                                        "SUM": {
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
                "SPLIT": {
                    "string": "la la la",
                    "delimiter": " "
                }
            }),
            json!(["la", "la", "la"]),
        );
    }
}
