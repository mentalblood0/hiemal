#[macro_export]
macro_rules! define_type_functions {
    (
        $type_name:ident,
        $raw_type:ty,
        $(
            $function_name:ident
        )+
    ) => {
        paste! {
            #[derive(Deserialize, Debug)]
            pub enum [<Computable $type_name>] {
                $(
                    $function_name($function_name),
                )+
            }

            #[derive(Deserialize, Debug)]
            #[serde(untagged)]
            pub enum [<Value $type_name>] {
                Computable([<Computable $type_name>]),
                Raw($raw_type),
            }

            impl [<Value $type_name>] {
                pub fn compute(&self) -> Result<$raw_type> {
                    match self {
                        [<Value $type_name>]::Computable(computable) => match computable {
                            $(
                                [<Computable $type_name>]::$function_name(computable) => computable.compute(),
                            )+
                        }
                        [<Value $type_name>]::Raw(raw_value) => Ok(raw_value.clone())
                    }
                }
            }
        }
    };
}
