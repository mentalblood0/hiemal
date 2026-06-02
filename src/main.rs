use std::process::exit;

use anyhow::{Context, Result};

use hiemal::{
    clause::{Clause, Include, IncludeFromAt},
    global_includes_cache, global_interpreter,
    path::Path,
    program::Program,
};

fn main() -> Result<()> {
    if let Some(target) = std::env::args().nth(1) {
        serde_saphyr::to_io_writer(
            &mut std::io::stdout(),
            &global_interpreter()
                .compute(
                    &Program::Clause(Clause::Include(Include {
                        include: IncludeFromAt {
                            from: serde_json::from_value(serde_json::Value::String(target))?,
                            at: Path(rpds::VectorSync::new_sync()),
                        },
                    })),
                    global_includes_cache(),
                )
                .context("Can not compute program")?,
        )
        .context("Can not output result of the program computation")?;
    } else {
        println!("The path is the goal");
        exit(1);
    };
    Ok(())
}
