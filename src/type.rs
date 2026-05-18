use std::collections::BTreeMap;

#[derive(PartialEq, Debug, Clone, PartialOrd, Ord, Eq)]
pub enum Type {
    Number,
    String,
    Bool,
    Null,
    Array(Box<Type>),
    Object(BTreeMap<String, Type>),
    GenericArgument(u8),
    RecursedFunction(String),
}
