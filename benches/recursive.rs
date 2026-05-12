use criterion::{criterion_group, criterion_main, Criterion};
use hiemal::{IncludesCache, Interpreter, ValueWithIncludes};
use serde_json::json;

fn fibonacci_benchmark(bencher_context: &mut Criterion) {
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
        bencher_context.bench_function(&format!("fibonacci_recursive_{number}"), |b| {
            b.iter(|| interpreter.compute(&program, &mut includes_cache).unwrap())
        });
    }
}

criterion_group!(benches, fibonacci_benchmark);
criterion_main!(benches);
