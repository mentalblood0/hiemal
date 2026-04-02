#[macro_export]
macro_rules! define_types_functions {
    (
        $(
            computed $type_name:ident is $raw_type:ty
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

                $(
                    #[derive(Serialize, Deserialize, Debug, PartialEq)]
                    pub struct $function_name {
                        $(
                            $function_argument_name: $function_argument_type,
                        )+
                    }

                    impl $function_name {
                        pub fn compute(&$function_self) -> Result<$raw_type> $function_code
                    }
                )+

                impl [<ComputableOrRaw $type_name>] {
                    pub fn compute(&self) -> Result<$raw_type> {
                        match self {
                            [<ComputableOrRaw $type_name>]::Computable(computable) => match computable {
                                $(
                                    [<Computable $type_name>]::$function_name(computable) => computable.compute(),
                                )+
                            }
                            [<ComputableOrRaw $type_name>]::Raw(raw_value) => Ok(raw_value.clone())
                        }
                    }
                }
            )+

            #[derive(Serialize, Deserialize, Debug, PartialEq)]
            #[serde(untagged)]
            pub enum ComputableOrRawAny {
                $(
                    $type_name([<ComputableOrRaw $type_name>]),
                )+
                TransparentArray(Vec<ComputableOrRawAny>),
                TransparentObject(BTreeMap<String, ComputableOrRawAny>),
            }

            impl ComputableOrRawAny {
                pub fn compute(&self) -> Result<serde_json::Value> {
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
