#[macro_export]
macro_rules! time_it {
    ($action:expr, $code:block) => {{
        let start = std::time::Instant::now();
        let result = $code;
        eprintln!("{} in {:?}", $action, start.elapsed());
        result
    }};
}

