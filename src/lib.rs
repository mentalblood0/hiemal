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

    #[test]
    fn test_numbers() {
        assert_compute_result(
            json!([
                1234,
                1234.1234,
                "1234",
                "1234.1234",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567.890",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567/890",
            ]),
            json!([
                1234,
                1234.1234,
                "1234",
                "1234.1234",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567.890",
                "12345678901234567890123456789012345678901234567890123456789012345678901234567/890",
            ]),
        )
    }

    #[test]
    fn test_recursive_normal() {
        assert_compute_result(
            json!({
                "functions": {
                  "fibonacci:": {
                      "match": { "is sorted": ["_", 1] },
                      "cases": [
                          [true, "_"],
                          [
                              false,
                              {
                                  "sum": [
                                    {
                                      "fibonacci:": {
                                        "sum": [
                                          "_",
                                          -1
                                        ]
                                      }
                                    },
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
                          ]
                      ]
                  }
                },
                "compute": {
                  "fibonacci:": 10
                }
            }),
            json!(55),
        );
    }

    #[test]
    fn test_recursive_big() {
        assert_compute_result(
            json!({
                "functions": {
                  "fibonacci_1:": {
                      "match": { "is sorted": ["_", 1] },
                      "cases": [
                          [true, "_"],
                          [
                              false,
                              {
                                  "sum": [
                                    {
                                      "fibonacci_2:": {
                                        "sum": [
                                          "_",
                                          -1
                                        ]
                                      }
                                    },
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
                          ]
                      ]
                  },
                  "fibonacci_2:": {
                      "match": { "is sorted": ["_", 1] },
                      "cases": [
                          [true, "_"],
                          [
                              false,
                              {
                                  "sum": [
                                    {
                                      "fibonacci_1:": {
                                        "sum": [
                                          "_",
                                          -1
                                        ]
                                      }
                                    },
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
                          ]
                      ]
                  },
                  "fibonacci_3:": {
                      "match": { "is sorted": ["_", 1] },
                      "cases": [
                          [true, "_"],
                          [
                              false,
                              {
                                  "sum": [
                                    {
                                      "fibonacci_1:": {
                                        "sum": [
                                          "_",
                                          -1
                                        ]
                                      }
                                    },
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
                          ]
                      ]
                  }

                },
                "compute": [
                  {"fibonacci_1:": 10},
                  {"fibonacci_2:": 10},
                  {"fibonacci_3:": 10},
                ]
            }),
            json!([55, 55, 55]),
        );
    }

    #[test]
    fn test_includes() {
        assert_compute_result(
            json!([{
                "functions": {
                    "fibonacci:": {"from": "examples/fibonacci.yml", "at": ["functions", {"object key": "fibonacci:"}]}
                },
                "compute": {
                    "fibonacci:": 10
                }
            }, {
                "from": "examples/fibonacci.yml", "at": ["compute", {"object key": "fibonacci:"}]
            }]),
            json!([55, 10]),
        );
    }

    #[test]
    fn test_heterogenous_array() {
        assert_compute_result(
            json!([1, "string", ["array"], {"object": 4}]),
            json!([1, "string", ["array"], {"object": 4}]),
        );
    }

    #[test]
    fn test_heterogenous_branching() {
        assert_compute_result(
            json!({
                "match": {"is sorted": [1, 2, 3]},
                "cases": [
                    [true, 1],
                    [false, "string"]
                ]
            }),
            json!(1),
        );
    }

    #[test]
    fn test_match_by_type_refined_branch() {
        assert_compute_result(
            json!({
                "match": {"parse yaml": "0x1A"},
                "cases": [
                    ["number", true],
                    ["string", "it's a string"],
                    ["any", "it's something else"]
                ]
            }),
            json!(true),
        );
    }

    #[test]
    fn test_match_by_type_any_branch() {
        assert_compute_result(
            json!({
                "match": {"parse yaml": "[]"},
                "cases": [
                    ["number", "it's a number"],
                    ["string", "it's a string"],
                    ["any", true]
                ]
            }),
            json!(true),
        );
    }

    #[test]
    fn test_match_by_type_without_any_branch() {
        assert_compute_result(
            json!({
                "match": {
                    "match": {"parse yaml": "[]"},
                    "as": "_",
                    "cases": [
                        ["number", "_"],
                        ["string", "_"],
                        ["any", null]
                    ]
                },
                "cases": [
                    ["number", "it's a number"],
                    ["string", "it's a string"],
                    ["null", true]
                ]
            }),
            json!(true),
        );
    }

    #[test]
    fn test_match_by_value() {
        assert_compute_result(
            json!({
                "match": {"parse yaml": "[1, 2, 3]"},
                "cases": [
                    ["number", "it's a number"],
                    ["string", "it's a string"],
                    [[1, {"sum": [1, 1]}, 3], true],
                    ["any", "it's something else"]
                ]
            }),
            json!(true),
        );
    }

    #[test]
    fn test_null() {
        assert_compute_result(json!(null), json!(null));
    }
}
