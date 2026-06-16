use std::collections::BTreeMap;

#[derive(PartialEq, Debug, Clone, Eq)]
pub enum Type {
    Number,
    String,
    Bool,
    Null,
    Array(Box<Type>),
    Object(BTreeMap<String, Type>),
    Unknown(usize),
}
