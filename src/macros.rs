#[macro_export]
macro_rules! define_type_functions {
    (
        $type_name:ident,
        $raw_type:ty,
        $(
            $function_name:ident {
                $(
                    $function_argument_name:ident: $function_argument_type:ty
                )+
            } $function_self:ident $function_code:block
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

            $(
                #[derive(Deserialize, Debug)]
                pub struct $function_name {
                    $(
                        $function_argument_name: $function_argument_type,
                    )+
                }

                impl $function_name {
                    pub fn compute(&$function_self) -> Result<$raw_type> $function_code
                }
            )+

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
