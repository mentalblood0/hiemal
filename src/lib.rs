pub mod compiler;
pub mod default_argument_name;
pub mod intermediate_representation;
pub mod program;
pub mod r#type;
pub mod value;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use crate::{compiler::compile, r#type::Type};

    #[test]
    fn test_numbers() {
        assert_eq!(
            compile(
                &serde_json::from_value(json!([
                1234,
                1234.1234,
                "1234",
                "1234.1234",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567.890",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567/890",
            ]))
                .unwrap(),
            )
            .unwrap()
            .r#type,
            Type::Array(Box::new(Type::Number))
        )
    }

    #[test]
    fn test_recursive_normal() {
        compile(
            &serde_json::from_value(json!({
              "scope": {
                "functions": {
                  "fibonacci:": {
                    "branching": {
                      "if": {
                        "is sorted": [
                          "_",
                          1
                        ]
                      },
                      "then": "_",
                      "else": {
                          "sum": [
                            {
                              "fibonacci:": {
                                "sum": [
                                  "_",
                                  -1
                                ]
                              }
                            },
                            1,
                            {
                              "fibonacci:": {
                                "sum": [
                                  "_",
                                  -2
                                ]
                              }
                            }
                          ]
                      }
                    }
                  }
                },
                "compute": {
                  "fibonacci:": 10
                }
            }}))
            .unwrap(),
        )
        .unwrap();
    }
}
