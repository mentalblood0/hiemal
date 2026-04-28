use criterion::{criterion_group, criterion_main, Criterion};
use hiemal::{Interpreter, Value};
use serde_json::json;
use std::sync::Arc;

fn fibonacci_benchmark(bencher_context: &mut Criterion) {
    let interpreter = Interpreter::default();
    let program: Arc<Value> = Arc::new(
        serde_json::from_value(json!({
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
                "FIBONACCI": 22
            }
        }))
        .unwrap(),
    );

    bencher_context.bench_function("fibonacci_recursive_10", |b| {
        b.iter(|| interpreter.compute(program.clone()).unwrap())
    });
}

criterion_group!(benches, fibonacci_benchmark);
criterion_main!(benches);
