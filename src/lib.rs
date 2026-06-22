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
        let intermediate_representation = compile(
            &serde_json::from_value(json!({
                "functions": {
                  "fibonacci:": {
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
                },
                "compute": {
                  "fibonacci:": 10
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let computer = Computer::default();
        assert_eq!(
            computer.compute(&intermediate_representation).unwrap(),
            serde_json::from_value(json!("55")).unwrap()
        );
    }

    #[test]
    fn test_recursive_big() {
        let intermediate_representation = compile(
            &serde_json::from_value(json!({
                "functions": {
                  "fibonacci_1:": {
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
                  },
                  "fibonacci_2:": {
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
                  },
                  "fibonacci_3:": {
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
                },
                "compute": [
                  {"fibonacci_1:": 10},
                  {"fibonacci_2:": 10},
                  {"fibonacci_3:": 10},
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let computer = Computer::default();
        assert_eq!(
            computer.compute(&intermediate_representation).unwrap(),
            serde_json::from_value(json!([55, 55, 55])).unwrap()
        );
    }

    #[test]
    fn test_includes() {
        let intermediate_representation = compile(
            &serde_json::from_value(json!([{
                "functions": {
                    "fibonacci:": {"from": "examples/fibonacci.yml", "at": ["functions", {"object key": "fibonacci:"}]}
                },
                "compute": {
                    "fibonacci:": 10
                }
            }, {
                "from": "examples/fibonacci.yml", "at": ["compute", {"object key": "fibonacci:"}]
            }])).unwrap(),
        )
        .unwrap();
        let computer = Computer::default();
        assert_eq!(
            computer.compute(&intermediate_representation).unwrap(),
            serde_json::from_value(json!(["55", "10"])).unwrap()
        );
    }

    #[test]
    fn test_heterogenous_array() {
        compile(&serde_json::from_value(json!([1, "string", ["array"], {"object": 4}])).unwrap())
            .unwrap();
    }

    #[test]
    fn test_heterogenous_branching() {
        compile(
            &serde_json::from_value(json!({
                "if": true,
                "then": 1,
                "else": "string"
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn test_match() {
        let intermediate_representation = compile(
            &serde_json::from_value(json!([{
                "match": {"parse yaml": "0x1A"},
                "cases": [
                    ["number", "it's a number"],
                    ["string", "it's a string"]
                ]
            }, {
                "match": {"parse yaml": "[]"},
                "cases": [
                    ["number", "it's a number"],
                    ["string", "it's a string"]
                ]
            }]))
            .unwrap(),
        )
        .unwrap();
        let computer = Computer::default();
        assert_eq!(
            computer.compute(&intermediate_representation).unwrap(),
            serde_json::from_value(json!(["it's a number", null])).unwrap()
        );
    }

    #[test]
    fn test_null() {
        let intermediate_representation =
            compile(&serde_json::from_value(json!(null)).unwrap()).unwrap();
        let computer = Computer::default();
        assert_eq!(
            computer.compute(&intermediate_representation).unwrap(),
            serde_json::from_value(json!(null)).unwrap()
        );
    }
}
