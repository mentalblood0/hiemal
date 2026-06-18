use criterion::{Criterion, criterion_group, criterion_main};
use hiemal::{compiler::compile, computer::Computer, program::Program};
use serde_json::json;

fn benchmarks(bencher_context: &mut Criterion) {
    let computer = Computer::default();
    let computer_with_caching = Computer {
        user_functions_caching: true,
    };
    {
        for number in [22, 23, 24] {
            let program = serde_json::from_value::<Program>(json!({
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
                  "fibonacci:": number
                }
            }))
            .unwrap();
            let intermediate_representation = compile(&program).unwrap();
            bencher_context.bench_function(&format!("fibonacci_{number}"), |b| {
                b.iter(|| computer.compute(&intermediate_representation).unwrap())
            });
            bencher_context.bench_function(&format!("fibonacci_{number}_with_caching"), |b| {
                b.iter(|| {
                    computer_with_caching
                        .compute(&intermediate_representation)
                        .unwrap()
                })
            });
        }
    }
}

criterion_group!(group, benchmarks);
criterion_main!(group);
