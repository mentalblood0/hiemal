use criterion::{criterion_group, criterion_main, Criterion};
use hiemal::{IncludesCache, Interpreter, ValueWithIncludes};
use serde_json::json;

fn fibonacci_benchmark(bencher_context: &mut Criterion) {
    let interpreter = Interpreter::default();
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
            "FIBONACCI": 22
        }
    }))
    .unwrap();

    let mut includes_cache = IncludesCache::default();

    bencher_context.bench_function("fibonacci_recursive_10", |b| {
        b.iter(|| interpreter.compute(&program, &mut includes_cache).unwrap())
    });
}

criterion_group!(benches, fibonacci_benchmark);
criterion_main!(benches);
