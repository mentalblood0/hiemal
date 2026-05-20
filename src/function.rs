use anyhow::Result;

use crate::r#type::Type;
use crate::value::Value;

#[derive(Debug)]
pub struct Function {
    pub argument_type: Type,
    pub return_type: Type,
    pub function: fn(Value) -> Result<Value>,
}
