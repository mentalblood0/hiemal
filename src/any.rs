use std::collections::BTreeMap;

use crate::number::ValueNumber;
use crate::string::ValueString;

pub enum Any {
    Number(ValueNumber),
    List(Vec<Any>),
    Dict(BTreeMap<ValueString, Any>),
}
