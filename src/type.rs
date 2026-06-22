use std::collections::{BTreeMap, BTreeSet};

#[repr(u8)]
#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
pub enum Type {
    Number,
    String,
    Bool,
    Null,
    Array(Box<Type>),
    Object(BTreeMap<String, Type>),
    Union(BTreeSet<Type>),
    Any,
    Unknown(usize),
}
