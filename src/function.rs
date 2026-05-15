use anyhow::Result;

use crate::r#type::Type;
use crate::value::RcOrValue;

#[derive(Debug)]
pub struct Function {
    pub argument_type: Type,
    pub return_type: Type,
    pub function: fn(RcOrValue) -> Result<RcOrValue>,
}
