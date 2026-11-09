use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use nanorand::{Rng, WyRand};

fn benchmarks(bencher_context: &mut Criterion) {
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
