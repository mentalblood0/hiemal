use criterion::{Criterion, criterion_group, criterion_main};
use dashu::Rational;
use hiemal::{
    includes_cache::IncludesCache,
    interpreter::Interpreter,
    value::{Value, ValueWithIncludes},
};
use serde_json::json;

fn benchmarks(bencher_context: &mut Criterion) {
    let interpreter = Interpreter::default();
    let mut includes_cache = IncludesCache::default();

    {
        for number in [20, 21, 22] {
            let program = serde_json::from_value::<ValueWithIncludes>(json!({
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
                    "fibonacci": number
                }
            }))
            .unwrap();
            bencher_context.bench_function(&format!("fibonacci_{number}"), |b| {
                b.iter(|| interpreter.compute(&program, &mut includes_cache).unwrap())
            });
        }
    }
    {
        let program = serde_json::from_value::<ValueWithIncludes>(json!({
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
                "20": {
                    "fibonacci": 20
                },
                "21": {
                    "fibonacci": 21
                },
                "22": {
                    "fibonacci": 22
                }
            }
        }))
        .unwrap();
        bencher_context.bench_function(&format!("fibonacci_object"), |b| {
            b.iter(|| interpreter.compute(&program, &mut includes_cache).unwrap())
        });
    }
    {
        let program = serde_json::from_value::<ValueWithIncludes>(json!({
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
                "with": {
                    "constants": {
                        "x": {"fibonacci": 20},
                        "y": {"fibonacci": 21},
                        "z": {"fibonacci": 22}
                    }
                },
                "compute": [
                    {"constant": "x"},
                    {"constant": "y"},
                    {"constant": "z"}
                ]
            }
        }))
        .unwrap();
        bencher_context.bench_function(&format!("fibonacci_constants"), |b| {
            b.iter(|| interpreter.compute(&program, &mut includes_cache).unwrap())
        });
    }
    {
        let program = serde_json::from_value::<ValueWithIncludes>(json!({
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
                "map": [20, 21, 22],
                "through": {"fibonacci": "_"}
            }
        }))
        .unwrap();
        bencher_context.bench_function(&format!("fibonacci_map"), |b| {
            b.iter(|| interpreter.compute(&program, &mut includes_cache).unwrap())
        });
    }
    {
        let program = serde_json::from_value::<ValueWithIncludes>(json!({
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
                "filter": [20, 21, 22],
                "through": {
                    "is sorted": [0, {"fibonacci": "_"}]
                }
            }
        }))
        .unwrap();
        bencher_context.bench_function(&format!("fibonacci_filter"), |b| {
            b.iter(|| interpreter.compute(&program, &mut includes_cache).unwrap())
        });
    }
    {
        let number = 20u64;
        let correct_raw: u64 = (1..=number).product();
        let correct = Value::Number(Rational::from(correct_raw));
        let program = serde_json::from_value::<ValueWithIncludes>(json!({
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
                "factorial": number
            }
        }))
        .unwrap();
        bencher_context.bench_function(&format!("factorial_{number}"), |b| {
            b.iter(|| {
                assert_eq!(
                    interpreter.compute(&program, &mut includes_cache).unwrap(),
                    correct
                )
            })
        });
    }
}

criterion_group!(group, benchmarks);
criterion_main!(group);
