pub mod default_argument_name;
pub mod embedded_functions;
pub mod function;
pub mod includes_cache;
pub mod interpreter;
pub mod path;
pub mod r#type;
pub mod value;

#[cfg(test)]
mod tests {
    use crate::value::ValueWithIncludes;

    use super::*;

    use std::sync::OnceLock;

    use pretty_assertions::assert_eq;
    use serde_json::json;

    use includes_cache::IncludesCache;
    use interpreter::Interpreter;

    fn default_interpreter() -> &'static Interpreter {
        static RESULT: OnceLock<Interpreter> = OnceLock::new();
        RESULT.get_or_init(|| Interpreter::default())
    }

    #[test]
    fn test_serde() {
        assert_eq!(
            serde_json::from_value::<ValueWithIncludes>(json!(["b"])).unwrap(),
            ValueWithIncludes::Array(vec![ValueWithIncludes::Other(serde_json::Value::String(
                "b".to_string()
            ))])
        );
    }

    #[test]
    fn test_numbers() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!([
                        1234,
                        1234.1234,
                        "1234",
                        "1234.1234",
                        "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                        "12345678901234567890123456789012345678901234567890123456789012345678901234567.890",
                        "12345678901234567890123456789012345678901234567890123456789012345678901234567/890",
                        {"product": [2, 3]},
                        {"size": [1, 2, 3]},
                        {"from": {
                                "map": [1],
                                "through": {"sum": ["_", 1]}
                            },
                            "at": [0]
                        }
                    ]))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!([
                1234,
                1234.1234,
                "1234",
                "1234.1234",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567.890",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567/890",
                6,
                3,
                2
            ]))
            .unwrap()
        );
    }

    #[test]
    fn test_simple_embedded_functions() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "sum": [
                            {"product": [2, 3]},
                            {"len": {"concat": ["lala", "lolo"]}},
                            4
                        ]
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(18)).unwrap()
        );
    }

    #[test]
    fn test_with() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "sum": [
                            {
                                "with": {"constants": {"x": 2, "y": 3}},
                                "compute": {"product": [{"constant": "x"}, {"constant": "x"}, {"constant": "y"}]}
                            },
                            {"len": {"concat": ["lala", "lolo"]}},
                            4
                        ]
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(24)).unwrap()
        );
    }

    #[test]
    fn test_user_functions() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "sum": [
                            {
                                "with": {
                                    "functions": {
                                        "square": {"product": ["_", "_"]},
                                    },
                                    "constants": {
                                        "y": 3
                                    }
                                },
                                "compute": {"product": [
                                    {"square": 2},
                                    {
                                        "square": {
                                            "square": {
                                                "product": [
                                                    {"square": 1},
                                                    {"sum": [{"constant": "y"}, -1]}
                                                ]
                                            }
                                        }
                                    }
                                ]}
                            },
                            {"len": {"concat": ["lala", "lolo"]}},
                            4
                        ]
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(76)).unwrap()
        );
    }

    #[test]
    fn test_generics() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "sum": [
                            {
                                    "from": [
                                        {"size": [1, 2, 3]},
                                        {"size": ["a", "b"]},
                                    ],
                                    "at": [1]
                            },
                            1
                        ]
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(3)).unwrap()
        );
    }

    #[test]
    fn test_map() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "sum": {
                            "map": [
                                {"size": [1, 2, 3]},
                                1
                            ],
                            "through": {"sum": ["_", 1]}
                        }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(6)).unwrap()
        );
    }

    #[test]
    fn test_filter() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "sum": {
                            "filter": [
                                {"size": [1, 2, 3]},
                                2,
                                1
                            ],
                            "as": "x",
                            "through": {"is sorted": [{"constant": "x"}, 2]}
                        }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(3)).unwrap()
        );
    }

    #[test]
    fn test_fold() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "fold": [
                            {"size": [1, 2, 3]},
                            2,
                            1
                        ],
                        "starting with": 0,
                        "through": {
                            "sum": [
                                {"constant": "accumulator"},
                                {"product": [{"constant": "current"}, {"constant": "current"}]}
                            ]
                        }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(14)).unwrap()
        );
    }

    #[test]
    fn test_factorial() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "with": {
                            "functions": {
                                "factorial": {
                                    "product": {
                                        "sequence": {
                                            "from": 1,
                                            "to": "_",
                                            "step": 1
                                        }
                                    }
                                }
                            }
                        },
                        "compute": {
                            "factorial": 5
                        }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(120)).unwrap()
        );
    }

    #[test]
    fn test_branching() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "if": true,
                        "then": 1,
                        "else": 0
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(1)).unwrap()
        );
    }

    #[test]
    fn test_try_or() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "try": {"from": ["a", "b"], "at": [2]},
                        "or": {"constant": "error"},
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(
                "Can not get element at index 2 from array of length 2 at path segment 2 of \
                 from-at clause at Path([Try])"
            ))
            .unwrap()
        );
    }

    #[test]
    fn test_from_at_list() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "from": ["a", "b"],
                        "at": [1]
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!("b")).unwrap()
        );
    }

    #[test]
    fn test_from_at_object() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "from": {"a": "a value", "b": "b value"},
                        "at": ["b"]
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!("b value")).unwrap()
        );
    }

    #[test]
    fn test_from_at_complex() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "from": {"a": "a value", "b": [1, 2]},
                        "at": ["b", 1]
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(2)).unwrap()
        );
    }

    #[test]
    fn test_from_at_error() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "try": {"from": ["a", "b"], "at": [2]},
                        "or": {"constant": "error"},
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(
                "Can not get element at index 2 from array of length 2 at path segment 2 of \
                 from-at clause at Path([Try])"
            ))
            .unwrap()
        );
    }

    #[test]
    fn test_arguments_local() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "with": {
                            "functions": {
                                "f1": {
                                    "sum": ["_", 1]
                                },
                                "f2": {
                                    "sum": ["_", 2]
                                },
                                "f3": {
                                    "sum": ["_", 3]
                                },
                                "f": {
                                    "f1": {
                                        "f2": {
                                            "f3": "_"
                                        }
                                    }
                                }
                            }
                        },
                        "compute": {
                            "f": 0
                        }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(6)).unwrap()
        );
    }

    #[test]
    fn test_arguments_nonlocal() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "with": {
                            "functions": {
                                "f": {
                                    "sum": [{"constant": "x"}, "_", 3]
                                },
                            },
                            "constants": {
                                "x": 1
                            }
                        },
                        "compute": {
                            "f": {
                                "f": {
                                    "x": 0,
                                    "_": -1
                                }
                            }
                        }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(6)).unwrap()
        );
    }

    #[test]
    fn test_recursive_normal() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                      "with": {
                        "functions": {
                          "fibonacci": {
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
                                    "fibonacci": {
                                      "sum": [
                                        "_",
                                        -1
                                      ]
                                    }
                                  },
                                  {
                                    "fibonacci": {
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
                        "fibonacci": 10
                      }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(55)).unwrap()
        );
    }

    #[test]
    fn test_recursive_short() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                      "with": {
                        "functions": {
                          "fibonacci": {
                            "if": {
                              "is sorted": [
                                "_",
                                1
                              ]
                            },
                            "then": "_",
                            "else": {
                                "fibonacci": {
                                  "sum": [
                                    "_",
                                    -1
                                  ]
                                }
                            }
                          }
                        }
                      },
                      "compute": {
                        "fibonacci": 10
                      }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(1)).unwrap()
        );
    }

    #[test]
    fn test_recursive_error() {
        assert!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                      "with": {
                        "functions": {
                          "fibonacci": {
                            "if": {
                              "is sorted": [
                                "_",
                                1
                              ]
                            },
                            "then": "_",
                            "else": {
                              "with": {
                                "constants": {
                                  "x": "_"
                                }
                              },
                              "compute": {
                                "sum": [
                                  {
                                    "fibonacci": "lalala"
                                  },
                                  {
                                    "fibonacci": {
                                      "sum": [
                                        "x",
                                        -2
                                      ]
                                    }
                                  }
                                ]
                              }
                            }
                          }
                        }
                      },
                      "compute": {
                        "fibonacci": 10
                      }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .is_err()
        );
    }
    #[test]
    fn test_recursive_long() {
        let builder = std::thread::Builder::new().stack_size(2 * 1024 * 1024);
        let handler = builder
            .spawn(|| {
                assert_eq!(
                    default_interpreter()
                        .compute(
                            &serde_json::from_value(json!({
                              "with": {
                                "functions": {
                                  "fibonacci_1": {
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
                                            "fibonacci_2": {
                                              "sum": [
                                                "_",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "fibonacci_2": {
                                              "sum": [
                                                "_",
                                                -2
                                              ]
                                            }
                                          }
                                        ]
                                    }
                                  },
                                  "fibonacci_2": {
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
                                            "fibonacci_1": {
                                              "sum": [
                                                "_",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "fibonacci_1": {
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
                                "fibonacci_1": 10
                              }
                            }))
                            .unwrap(),
                            &mut IncludesCache::default()
                        )
                        .unwrap(),
                    serde_json::from_value(json!(55)).unwrap()
                );
            })
            .unwrap();
        handler.join().unwrap();
    }

    #[test]
    fn test_includes() {
        assert_eq!(
            default_interpreter()
                .compute(
                    &serde_json::from_value(json!([{
                        "with": {
                            "include": ["examples/factorial.yml", "with"],
                        },
                        "compute": {
                            "factorial": 5
                        }
                    }, {
                        "include": ["https://raw.githubusercontent.com/mentalblood0/hiemal/refs/heads/main/examples/factorial.yml", "compute", "factorial"]
                    }]))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!([
                120,
                5
            ]))
            .unwrap()
        );
    }
}
