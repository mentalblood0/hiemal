use std::sync::{Arc, LazyLock};

pub static DEFAULT_ARGUMENT_NAME: LazyLock<Arc<String>> = LazyLock::new(|| {
    Arc::new("_".to_string())
});
