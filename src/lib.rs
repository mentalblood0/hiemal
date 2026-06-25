pub mod compiler;
pub mod computer;
pub mod containers;
pub mod default_argument_name;
pub mod includes_cache;
pub mod intermediate_representation;
pub mod program;
pub mod r#type;
pub mod value;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use crate::{compiler::compile, computer::Computer};

    fn assert_compute_result(program: serde_json::Value, computed_result: serde_json::Value) {
        let intermediate_representation =
            compile(&serde_json::from_value(program).unwrap()).unwrap();
        let computer = Computer::default();
        assert_eq!(
            computer.compute(&intermediate_representation).unwrap(),
            serde_json::from_value(computed_result).unwrap()
        );
    }
}
