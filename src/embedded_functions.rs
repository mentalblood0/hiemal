use std::sync::LazyLock;

use crate::r#type::TypeKind;

pub static EMBEDDED_FUNCTIONS: LazyLock<[TypeKind; 19]> =
    LazyLock::new(|| serde_saphyr::from_str(include_str!("embedded_functions.yml")).unwrap());
