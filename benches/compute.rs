use criterion::{criterion_group, criterion_main, Criterion};
use hiemal::{IncludesCache, Interpreter, Value, ValueWithIncludes};
use serde_json::json;

fn fibonacci(bencher_context: &mut Criterion) {
    let interpreter = Interpreter::default();
    let mut includes_cache = IncludesCache::default();
    for number in [22, 23, 24] {
        let program = serde_json::from_value::<ValueWithIncludes>(json!({
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
                                        "FIBONACCI": {
                                            "SUM": [
                                                "x",
                                                -1
                                            ]
                                        }
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
                "FIBONACCI": number
            }
        }))
        .unwrap();
        bencher_context.bench_function(&format!("fibonacci_{number}"), |b| {
            b.iter(|| interpreter.compute(&program, &mut includes_cache).unwrap())
        });
    }
}

fn factorial(bencher_context: &mut Criterion) {
    let interpreter = Interpreter::default();
    let mut includes_cache = IncludesCache::default();
    let number = 20u64;
    let correct_raw: u64 = (1..=number).product();
    dbg!(correct_raw);
    let correct = Value::Number(correct_raw as f64);
    let program = serde_json::from_value::<ValueWithIncludes>(json!({
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
            "FACTORIAL": number
        }
    }))
    .unwrap();
    bencher_context.bench_function(&format!("factorial_{number}"), |b| {
        b.iter(|| {
            assert_eq!(
                *interpreter.compute(&program, &mut includes_cache).unwrap(),
                correct
            )
        })
    });
}

criterion_group!(benches, factorial);
criterion_main!(benches);
