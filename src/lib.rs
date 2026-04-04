use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use paste::paste;

use serde::{Deserialize, Serialize};

#[macro_export]
macro_rules! define_types_functions {
    (
        $(
            computed $type_name:ident is $raw_type:ty {
                $(
                    $function_name:ident {
                        $(
                            $function_argument_name:ident: $function_argument_type:ty
                        )+
                    } $function_self:ident $function_context:ident $function_code:block
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
                    Placeholder(String)
                }

                #[derive(Serialize, Deserialize, Debug, PartialEq)]
                #[serde(untagged)]
                pub enum [<ComputableOrRaw $type_name>] {
                    Computable([<Computable $type_name>]),
                    Raw($raw_type),
                }

                impl Default for [<ComputableOrRaw $type_name>] {
                    fn default() -> Self {
                        let default_raw_value: $raw_type = ::std::default::Default::default();
                        Self::Raw(default_raw_value)
                    }
                }

                $(
                    #[derive(Serialize, Deserialize, Debug, PartialEq)]
                    pub struct $function_name {
                        $(
                            pub $function_argument_name: $function_argument_type,
                        )+
                    }

                    impl $function_name {
                        pub fn validate_placeholders_with_context(&self, context: &mut PlaceholdersValidationContext) -> Result<()> {
                            $(
                                self.$function_argument_name.validate_placeholders_with_context(context)?;
                            )+
                            Ok(())
                        }

                        pub fn compute_with_context(&$function_self, $function_context: &mut ComputationContext) -> Result<$raw_type> $function_code
                    }
                )+


                impl [<ComputableOrRaw $type_name>] {
                    pub fn validate_placeholders_with_context(&self, context: &mut PlaceholdersValidationContext) -> Result<()> {
                        match self {
                            [<ComputableOrRaw $type_name>]::Computable(computable) => match computable {
                                $(
                                    [<Computable $type_name>]::$function_name(computable) => computable.validate_placeholders_with_context(context),
                                )+
                                [<Computable $type_name>]::Placeholder(_) => Err(anyhow!("Computation of programs with placeholders is not supported yet")),
                            }
                            [<ComputableOrRaw $type_name>]::Raw(_) => Ok(()),
                        }
                    }

                    fn compute_with_context(&self, context: &mut ComputationContext) -> Result<$raw_type> {
                        match self {
                            [<ComputableOrRaw $type_name>]::Computable(computable) => match computable {
                                $(
                                    [<Computable $type_name>]::$function_name(computable) => computable.compute_with_context(context),
                                )+
                                [<Computable $type_name>]::Placeholder(_) => Err(anyhow!("Computation of programs with placeholders is not supported yet")),
                            }
                            [<ComputableOrRaw $type_name>]::Raw(raw_value) => Ok(raw_value.clone()),
                        }
                    }
                }
            )+

            pub struct PlaceholdersValidationContext {
                pub available_values: BTreeMap<String, Vec<Arc<Any>>>,
                pub path: Vec<String>
            }

            pub struct ComputationContext {
                pub available_values: BTreeMap<String, Vec<Arc<Any>>>,
            }

            #[derive(Serialize, Deserialize, Debug, PartialEq)]
            #[serde(deny_unknown_fields)]
            pub struct With {
                pub values: BTreeMap<String, Arc<Any>>,
                pub compute: Arc<Any>
            }

            #[derive(Serialize, Deserialize, Debug, PartialEq)]
            pub enum Closure {
                With(With)
            }

            #[derive(Serialize, Deserialize, Debug, PartialEq)]
            #[serde(untagged)]
            #[serde(deny_unknown_fields)]
            pub enum Any {
                $(
                    $type_name([<ComputableOrRaw $type_name>]),
                )+
                Closure(Closure),
                TransparentArray(Vec<Any>),
                TransparentObject(BTreeMap<String, Any>),
            }

            impl Any {
                pub fn validate_placeholders(&self) -> Result<()> {
                    self.validate_placeholders_with_context(&mut PlaceholdersValidationContext {
                        available_values: BTreeMap::new(),
                        path: vec![]
                    })
                }

                fn validate_placeholders_with_context(&self, context: &mut PlaceholdersValidationContext) -> Result<()> {
                    match self {
                        $(
                            Any::$type_name(value) => value.validate_placeholders_with_context(context)?,
                        )+
                        Any::Closure(closure) => match closure {
                            Closure::With(with_values_compute) => {
                                for (key, new_value) in with_values_compute.values.iter() {
                                    context.available_values.entry(key.clone()).or_insert(vec![]).push(new_value.clone());
                                }
                                for key in with_values_compute.values.keys() {
                                    context.available_values.entry(key.clone());
                                    if let Some(current_values_at_key) = context.available_values.get_mut(key) {
                                        current_values_at_key.pop();
                                    }
                                }
                            }
                        }
                        Any::TransparentArray(array) => {
                            for any in array {
                                any.validate_placeholders_with_context(context)?;
                            }
                        }
                        Any::TransparentObject(map) => {
                            for (_, any) in map {
                                any.validate_placeholders_with_context(context)?;
                            }
                        }
                    }
                    Ok(())
                }

                pub fn compute(&self) -> Result<serde_json::Value> {
                    self.compute_with_context(&mut ComputationContext {
                        available_values: BTreeMap::new()
                    })
                }

                fn compute_with_context(&self, context: &mut ComputationContext) -> Result<serde_json::Value> {
                    Ok(match self {
                        $(
                            Any::$type_name(value) => serde_json::to_value(value.compute_with_context(context)?)?,
                        )+
                        Any::Closure(closure) => match closure {
                            Closure::With(with_values_compute) => {
                                for (key, new_value) in with_values_compute.values.iter() {
                                    context.available_values.entry(key.clone()).or_insert(vec![]).push(new_value.clone());
                                }
                                let result = with_values_compute.compute.compute_with_context(context)?;
                                for key in with_values_compute.values.keys() {
                                    context.available_values.entry(key.clone());
                                    if let Some(current_values_at_key) = context.available_values.get_mut(key) {
                                        current_values_at_key.pop();
                                    }
                                }
                                result
                            }
                        }
                        Any::TransparentArray(array) => {
                            let mut result = vec![];
                            for value_any in array {
                                result.push(value_any.compute_with_context(context)?);
                            }
                            serde_json::Value::Array(result)
                        }
                        Any::TransparentObject(map) => {
                            let mut result = serde_json::Map::new();
                            for (key, value_any) in map {
                                result.insert(key.clone(), value_any.compute_with_context(context)?);
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
    computed Number is f64 {
        Sum {
            terms: Box<ComputableOrRawNumberArray>
        } self context {
            let mut result = 0f64;
            for term in self.terms.compute_with_context(context)?.iter() {
                result += term;
            }
            Ok(result)
        }
        Multiply {
            terms: Box<ComputableOrRawNumberArray>
        } self context {
            let mut result = 1f64;
            for term in self.terms.compute_with_context(context)?.iter() {
                result *= term;
            }
            Ok(result)
        }
    }
    computed String is String {
        Concat {
            strings: Box<ComputableOrRawStringArray>
        } self context {
            let mut result = "".to_string();
            for string in self.strings.compute_with_context(context)?.iter() {
                result += string;
            }
            Ok(result)
        }
        Repeat {
            string: Box<ComputableOrRawString>
            amount: ComputableOrRawNumber
        } self context {
            let string = self.string.compute_with_context(context)?;
            let amount = self.amount.compute_with_context(context)? as usize;
            Ok(string.repeat(amount))
        }
    }
    computed StringArray is Vec<String> {
        Split {
            string: ComputableOrRawString
            delimiter: ComputableOrRawString
        } self context {
            let string = self.string.compute_with_context(context)?;
            let delimiter = self.delimiter.compute_with_context(context)?;
            Ok(string.split(&delimiter).map(|s| s.to_string()).collect())
        }
    }
    computed NumberArray is Vec<f64> {
        Bytes {
            string: ComputableOrRawString
        } self context {
            let string = self.string.compute_with_context(context)?;
            Ok(string.bytes().map(|byte| byte as f64).collect())
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
    ) -> Result<()> {
        let program = serde_json::from_value::<Any>(program_structure)?;
        dbg!(&program);
        program.validate_placeholders().unwrap();
        let result = serde_json::to_value(program.compute()?)?;
        assert_eq!(result, correct_result);
        Ok(())
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
                        4
                    ]
                }
            }),
            json!(7.0),
        )
        .unwrap();
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
        )
        .unwrap();
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
        )
        .unwrap();
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
        )
        .unwrap();
        execute_and_assert(
            json!({
                "SPLIT": {
                    "string": "la la la",
                    "delimiter": " "
                }
            }),
            json!(["la", "la", "la"]),
        )
        .unwrap();
    }
}
