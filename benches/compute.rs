use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use hiemal::{
    compiler::Compiler,
    computer::{Computer, ComputerConfig},
    program::Program,
};
use nanorand::{Rng, WyRand};

fn benchmarks(bencher_context: &mut Criterion) {
    {
        let computer = Computer::default();
        let compiler = Compiler {
            metaprograms_computer: computer.clone(),
        };
        let computer_with_caching = Computer {
            config: ComputerConfig {
                user_functions_caching: true,
            },
        };
        for number in [22, 23, 24] {
            let program = serde_saphyr::from_str::<Program>(
                &r#"functions:
  fibonacci::
    match: _
    cases:
      - [1, 1]
      - [2, 1]
      - - number
        - sum:
            - fibonacci::
                sum: [_, -1]
            - fibonacci::
                sum: [_, -2]
compute:
  fibonacci:: NUMBER
"#
                .replace("NUMBER", &number.to_string()),
            )
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
    {
        static SEED: u64 = 0;
        static VECTOR_SIZE: usize = 1000;
        static OPERATIONS_COUNT: usize = 10000;
        bencher_context.bench_function("set vector std", |b| {
            b.iter(|| {
                let mut rng = WyRand::new_seed(SEED);
                let mut results: Vec<Vec<Arc<usize>>> = Vec::with_capacity(OPERATIONS_COUNT);
                results.push(Vec::from_iter(
                    (0..VECTOR_SIZE).map(|_| Arc::new(rng.generate_range(0..usize::MAX))),
                ));
                for operation_index in 0..OPERATIONS_COUNT {
                    let mut new_vector =
                        results[rng.generate_range(0..operation_index + 1)].clone();
                    new_vector[rng.generate_range(0..VECTOR_SIZE)] =
                        Arc::new(rng.generate_range(0..usize::MAX));
                    results.push(new_vector);
                }
            })
        });
        bencher_context.bench_function("set vector rpds", |b| {
            b.iter(|| {
                let mut rng = WyRand::new_seed(SEED);
                let mut results: Vec<rpds::VectorSync<usize>> =
                    Vec::with_capacity(OPERATIONS_COUNT);
                results.push(rpds::VectorSync::from_iter(
                    (0..VECTOR_SIZE).map(|_| rng.generate_range(0..usize::MAX)),
                ));
                for operation_index in 0..OPERATIONS_COUNT {
                    let mut new_vector =
                        results[rng.generate_range(0..operation_index + 1)].clone();
                    new_vector.set_mut(
                        rng.generate_range(0..VECTOR_SIZE),
                        rng.generate_range(0..usize::MAX),
                    );
                    results.push(new_vector);
                }
            })
        });
        bencher_context.bench_function("insert vector std", |b| {
            b.iter(|| {
                let mut rng = WyRand::new_seed(SEED);
                let mut results: Vec<Vec<Arc<usize>>> = Vec::with_capacity(OPERATIONS_COUNT);
                results.push(Vec::from_iter(
                    (0..VECTOR_SIZE).map(|_| Arc::new(rng.generate_range(0..usize::MAX))),
                ));
                for operation_index in 0..OPERATIONS_COUNT {
                    let mut new_vector =
                        results[rng.generate_range(0..operation_index + 1)].clone();
                    new_vector.insert(
                        rng.generate_range(0..new_vector.len()),
                        Arc::new(rng.generate_range(0..usize::MAX)),
                    );
                    results.push(new_vector);
                }
            })
        });
        bencher_context.bench_function("insert vector rpds", |b| {
            b.iter(|| {
                let mut rng = WyRand::new_seed(SEED);
                let mut results: Vec<rpds::VectorSync<usize>> =
                    Vec::with_capacity(OPERATIONS_COUNT);
                results.push(rpds::VectorSync::from_iter(
                    (0..VECTOR_SIZE).map(|_| rng.generate_range(0..usize::MAX)),
                ));
                for operation_index in 0..OPERATIONS_COUNT {
                    let mut new_vector =
                        results[rng.generate_range(0..operation_index + 1)].clone();
                    let insertion_index = rng.generate_range(0..new_vector.len());
                    new_vector.push_back_mut(*new_vector.last().unwrap());
                    for index_to_shift_previous_value_to in
                        (insertion_index + 1)..(new_vector.len() - 2)
                    {
                        new_vector.set_mut(
                            index_to_shift_previous_value_to,
                            new_vector[index_to_shift_previous_value_to - 1],
                        );
                    }
                    new_vector[insertion_index] = rng.generate_range(0..usize::MAX);
                    results.push(new_vector);
                }
            })
        });
    }
}

criterion_group!(group, benchmarks);
criterion_main!(group);
