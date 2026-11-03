use std::{collections::BTreeMap, sync::LazyLock};

use serde::Deserialize;

use crate::{
    program::EmbeddedFunction,
    r#type::{TypeKind, TypeProperties},
};


#[derive(Deserialize, Default)]
#[serde(bound = "T: Deserialize<'de>")]
pub enum StaticOrDynamic<T> {
    #[serde(rename = "static")]
    Static(T),
    #[default]
    #[serde(rename = "dynamic")]
    Dynamic
}

#[derive(Deserialize, Default)]
pub struct Return {
    #[serde(default)]
    pub r#type: StaticOrDynamic<TypeKind>,
    #[serde(default)]
    pub properties: StaticOrDynamic<TypeProperties>,
}

#[derive(Deserialize)]
pub struct EmbeddedFunctionSignature {
    pub argument: TypeKind,
    #[serde(default)]
    pub r#return: Return,
}

pub static EMBEDDED_FUNCTIONS: LazyLock<Vec<EmbeddedFunctionSignature>> = LazyLock::new(|| {
    let mut name_and_signature_pairs = serde_saphyr::from_str::<
        BTreeMap<EmbeddedFunction, EmbeddedFunctionSignature>,
    >(include_str!("embedded_functions.yml"))
    .unwrap()
    .into_iter()
    .collect::<Vec<_>>();
    name_and_signature_pairs.sort_by_key(|a| a.0);
    name_and_signature_pairs
        .into_iter()
        .map(|(_, signature)| signature)
        .collect()
});
