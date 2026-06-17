use criterion::{Criterion, criterion_group, criterion_main};
use hiemal::{compiler::compile, computer::compute, program::Program};
use serde_json::json;

fn benchmarks(bencher_context: &mut Criterion) {
    {
        for number in [22, 23, 24] {
            let program = serde_json::from_value::<Program>(json!({
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
                  "fibonacci:": number
                }
              }
            }))
            .unwrap();
            bencher_context.bench_function(&format!("fibonacci_{number}"), |b| {
                b.iter(|| compute(&compile(&program).unwrap()).unwrap())
            });
        }
    }
}

criterion_group!(group, benchmarks);
criterion_main!(group);
