pub mod embedded_functions;
pub mod function;
pub mod includes_cache;
pub mod interpreter;
pub mod path;
pub mod r#type;
pub mod value;

#[cfg(test)]
mod tests {
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
    fn test_numbers() {
        assert_eq!(
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!([
                        1234,
                        1234.1234,
                        "1234",
                        "1234.1234",
                        "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                        "12345678901234567890123456789012345678901234567890123456789012345678901234567.890",
                        "12345678901234567890123456789012345678901234567890123456789012345678901234567/890",
                        {"PRODUCT": [2, 3]},
                        {"SIZE": [1, 2, 3]},
                        {"FROM": {
                                "MAP": [1],
                                "THROUGH": {"SUM": ["_", 1]}
                            },
                            "AT": [0]
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "SUM": [
                            {"PRODUCT": [2, 3]},
                            {"LEN": {"CONCAT": ["lala", "lolo"]}},
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "SUM": [
                            {
                                "WITH": {"DEFINITIONS": {"x": 2, "y": 3}},
                                "COMPUTE": {"PRODUCT": ["x", "x", "y"]}
                            },
                            {"LEN": {"CONCAT": ["lala", "lolo"]}},
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
    fn test_user_functions_definitions() {
        assert_eq!(
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "SUM": [
                            {
                                "WITH": {
                                    "DEFINITIONS": {
                                        "SQUARE": {"PRODUCT": ["_", "_"]},
                                        "y": 3
                                    }
                                },
                                "COMPUTE": {"PRODUCT": [
                                    {"SQUARE": 2},
                                    {
                                        "SQUARE": {
                                            "SQUARE": {
                                                "PRODUCT": [
                                                    {"SQUARE": 1},
                                                    {"SUM": ["y", -1]}
                                                ]
                                            }
                                        }
                                    }
                                ]}
                            },
                            {"LEN": {"CONCAT": ["lala", "lolo"]}},
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "SUM": [
                            {
                                    "FROM": [
                                        {"SIZE": [1, 2, 3]},
                                        {"SIZE": ["a", "b"]},
                                    ],
                                    "AT": [1]
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "SUM": {
                            "MAP": [
                                {"SIZE": [1, 2, 3]},
                                1
                            ],
                            "THROUGH": {"SUM": ["_", 1]}
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "SUM": {
                            "FILTER": [
                                {"SIZE": [1, 2, 3]},
                                2,
                                1
                            ],
                            "AS_ALIAS": "x",
                            "THROUGH": {"IS_SORTED": ["x", 2]}
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "FOLD": [
                            {"SIZE": [1, 2, 3]},
                            2,
                            1
                        ],
                        "STARTING_WITH": 0,
                        "THROUGH": {
                            "SUM": [
                                "accumulator",
                                {"PRODUCT": ["current", "current"]}
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "WITH": {
                            "DEFINITIONS": {
                                "FACTORIAL": {
                                    "PRODUCT": {
                                        "SEQUENCE": {
                                            "from": 1,
                                            "to": "_",
                                            "step": 1
                                        }
                                    }
                                }
                            }
                        },
                        "COMPUTE": {
                            "FACTORIAL": 5
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
    fn test_definitions_vs_constants() {
        assert_eq!(
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "WITH": {"CONSTANTS": {"x": 1}},
                        "COMPUTE": {
                            "WITH": {
                                "DEFINITIONS": {"definition": "x"},
                                "CONSTANTS": {"x": 2, "constant": "x"}
                            },
                            "COMPUTE": ["definition", "constant"]
                        }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!([2, 1])).unwrap()
        );
    }

    #[test]
    fn test_branching() {
        assert_eq!(
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "IF": true,
                        "THEN": 1,
                        "ELSE": 0
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "TRY": {"FROM": ["a", "b"], "AT": [2]},
                        "OR": "error",
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(
                "Can not get element at index 2 from array of length 2 at the point Path([Try, \
                 At, AtIndex(0)])"
            ))
            .unwrap()
        );
    }

    #[test]
    fn test_from_at_list() {
        assert_eq!(
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "FROM": ["a", "b"],
                        "AT": [1]
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "FROM": {"a": "a value", "b": "b value"},
                        "AT": ["b"]
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "FROM": {"a": "a value", "b": [1, 2]},
                        "AT": ["b", 1]
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "TRY": {"FROM": ["a", "b"], "AT": [2]},
                        "OR": "error",
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
            serde_json::from_value(json!(
                "Can not get element at index 2 from array of length 2 at the point Path([Try, \
                 At, AtIndex(0)])"
            ))
            .unwrap()
        );
    }

    #[test]
    fn test_arguments() {
        assert_eq!(
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                        "WITH": {
                            "DEFINITIONS": {
                                "F1": {
                                    "SUM": ["_", 1]
                                },
                                "F2": {
                                    "SUM": ["_", 2]
                                },
                                "F3": {
                                    "SUM": ["_", 3]
                                },
                                "F": {
                                    "F1": {
                                        "F2": {
                                            "F3": "_"
                                        }
                                    }
                                }
                            }
                        },
                        "COMPUTE": {
                            "F": 0
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                      "WITH": {
                        "DEFINITIONS": {
                          "FIBONACCI": {
                            "IF": {
                              "IS_SORTED": [
                                "_",
                                1
                              ]
                            },
                            "THEN": "_",
                            "ELSE": {
                                "SUM": [
                                  {
                                    "FIBONACCI": {
                                      "SUM": [
                                        "_",
                                        -1
                                      ]
                                    }
                                  },
                                  {
                                    "FIBONACCI": {
                                      "SUM": [
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
                      "COMPUTE": {
                        "FIBONACCI": 10
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                      "WITH": {
                        "DEFINITIONS": {
                          "FIBONACCI": {
                            "IF": {
                              "IS_SORTED": [
                                "_",
                                1
                              ]
                            },
                            "THEN": "_",
                            "ELSE": {
                              "WITH": {
                                "CONSTANTS": {
                                  "x": "_"
                                }
                              },
                              "COMPUTE": {
                                "FIBONACCI": {
                                  "SUM": [
                                    "x",
                                    -1
                                  ]
                                }
                              }
                            }
                          }
                        }
                      },
                      "COMPUTE": {
                        "FIBONACCI": 10
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
        assert!(default_interpreter()
            .compute(
                &serde_json::from_value(json!({
                  "WITH": {
                    "DEFINITIONS": {
                      "FIBONACCI": {
                        "IF": {
                          "IS_SORTED": [
                            "_",
                            1
                          ]
                        },
                        "THEN": "_",
                        "ELSE": {
                          "WITH": {
                            "CONSTANTS": {
                              "x": "_"
                            }
                          },
                          "COMPUTE": {
                            "SUM": [
                              {
                                "FIBONACCI": "lalala"
                              },
                              {
                                "FIBONACCI": {
                                  "SUM": [
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
                  "COMPUTE": {
                    "FIBONACCI": 10
                  }
                }))
                .unwrap(),
                &mut IncludesCache::default()
            )
            .is_err());
    }
    #[test]
    fn test_recursive_long() {
        let builder = std::thread::Builder::new().stack_size(2 * 1024 * 1024);
        let handler = builder
            .spawn(|| {
                assert_eq!(
                    *default_interpreter()
                        .compute(
                            &serde_json::from_value(json!({
                              "WITH": {
                                "DEFINITIONS": {
                                  "FIBONACCI_1": {
                                    "IF": {
                                      "IS_SORTED": [
                                        "_",
                                        1
                                      ]
                                    },
                                    "THEN": "_",
                                    "ELSE": {
                                      "WITH": {
                                        "CONSTANTS": {
                                          "x": "_"
                                        }
                                      },
                                      "COMPUTE": {
                                        "SUM": [
                                          {
                                            "FIBONACCI_2": {
                                              "SUM": [
                                                "x",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "FIBONACCI_2": {
                                              "SUM": [
                                                "x",
                                                -2
                                              ]
                                            }
                                          }
                                        ]
                                      }
                                    }
                                  },
                                  "FIBONACCI_2": {
                                    "IF": {
                                      "IS_SORTED": [
                                        "_",
                                        1
                                      ]
                                    },
                                    "THEN": "_",
                                    "ELSE": {
                                      "WITH": {
                                        "CONSTANTS": {
                                          "x": "_"
                                        }
                                      },
                                      "COMPUTE": {
                                        "SUM": [
                                          {
                                            "FIBONACCI_3": {
                                              "SUM": [
                                                "x",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "FIBONACCI_3": {
                                              "SUM": [
                                                "x",
                                                -2
                                              ]
                                            }
                                          }
                                        ]
                                      }
                                    }
                                  },
                                  "FIBONACCI_3": {
                                    "IF": {
                                      "IS_SORTED": [
                                        "_",
                                        1
                                      ]
                                    },
                                    "THEN": "_",
                                    "ELSE": {
                                      "WITH": {
                                        "CONSTANTS": {
                                          "x": "_"
                                        }
                                      },
                                      "COMPUTE": {
                                        "SUM": [
                                          {
                                            "FIBONACCI_4": {
                                              "SUM": [
                                                "x",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "FIBONACCI_4": {
                                              "SUM": [
                                                "x",
                                                -2
                                              ]
                                            }
                                          }
                                        ]
                                      }
                                    }
                                  },
                                  "FIBONACCI_4": {
                                    "IF": {
                                      "IS_SORTED": [
                                        "_",
                                        1
                                      ]
                                    },
                                    "THEN": "_",
                                    "ELSE": {
                                      "WITH": {
                                        "CONSTANTS": {
                                          "x": "_"
                                        }
                                      },
                                      "COMPUTE": {
                                        "SUM": [
                                          {
                                            "FIBONACCI_1": {
                                              "SUM": [
                                                "x",
                                                -1
                                              ]
                                            }
                                          },
                                          {
                                            "FIBONACCI_1": {
                                              "SUM": [
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
                              "COMPUTE": {
                                "FIBONACCI_1": 10
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
            *default_interpreter()
                .compute(
                    &serde_json::from_value(json!({
                      "from local file": {
                        "INCLUDE_FILE": "examples/factorial.yml"
                      },
                      "from net": {
                        "INCLUDE_URL": "https://raw.githubusercontent.com/mentalblood0/hiemal/refs/heads/main/examples/factorial.yml"
                      }
                    }))
                    .unwrap(),
                    &mut IncludesCache::default()
                )
                .unwrap(),
                serde_json::from_value(json!({
                    "from local file": 120,
                    "from net": 120
                })).unwrap()
        );
    }
}
