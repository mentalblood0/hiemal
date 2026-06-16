pub mod compiler;
pub mod computer;
pub mod containers;
pub mod default_argument_name;
pub mod intermediate_representation;
pub mod program;
pub mod r#type;
pub mod value;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{compiler::compile, computer::compute};

    #[test]
    fn test_numbers() {
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
        .unwrap();
    }

    #[test]
    fn test_recursive_normal() {
        compute(
            &compile(
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
                                0,
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
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn test_recursive_big() {
        compile(
            &serde_json::from_value(json!({
              "scope": {
                "functions": {
                  "fibonacci_1:": {
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
                              "fibonacci_2:": {
                                "sum": [
                                  "_",
                                  -1
                                ]
                              }
                            },
                            0,
                            {
                              "fibonacci_3:": {
                                "sum": [
                                  "_",
                                  -2
                                ]
                              }
                            }
                          ]
                      }
                    }
                  },
                  "fibonacci_2:": {
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
                              "fibonacci_1:": {
                                "sum": [
                                  "_",
                                  -1
                                ]
                              }
                            },
                            0,
                            {
                              "fibonacci_3:": {
                                "sum": [
                                  "_",
                                  -2
                                ]
                              }
                            }
                          ]
                      }
                    }
                  },
                  "fibonacci_3:": {
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
                              "fibonacci_1:": {
                                "sum": [
                                  "_",
                                  -1
                                ]
                              }
                            },
                            0,
                            {
                              "fibonacci_2:": {
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
                  "fibonacci_1:": 10,
                  "fibonacci_2:": 10,
                  "fibonacci_3:": 10,
                }
            }}))
            .unwrap(),
        )
        .unwrap();
    }
}
