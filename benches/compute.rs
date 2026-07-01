use criterion::{Criterion, criterion_group, criterion_main};
use hiemal::{compiler::Compiler, computer::Computer, program::Program};
use serde_json::json;

fn benchmarks(bencher_context: &mut Criterion) {
    let computer = Computer::default();
    let compiler = Compiler {
        metaprograms_computer: computer.clone(),
    };
    let computer_with_caching = Computer {
        user_functions_caching: true,
    };
    {
        for number in [22, 23, 24] {
            let program = serde_json::from_value::<Program>(json!({
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
                  "fibonacci:": number
                }
            }))
            .unwrap();
            let intermediate_representation = compiler.compile(&program).unwrap();
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
