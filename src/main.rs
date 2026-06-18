use anyhow::{Context, Result};

use hiemal::{
    compiler::compile,
    computer::Computer,
    program::{IncludeFromAt, Path, Program},
};

fn main() -> Result<()> {
    if let Some(target) = std::env::args().nth(1) {
        let wrapped_program = Program::Include {
            include: IncludeFromAt {
                from: serde_json::from_value(serde_json::Value::String(target))?,
                at: Path::default(),
            },
        };
        let intermediate_representation = compile(&wrapped_program)?;
        serde_saphyr::to_io_writer(
            &mut std::io::stdout(),
            &Computer::default()
                .compute(&intermediate_representation)
                .context("Can not compute program")?,
        )
        .context("Can not output result of the program computation")?;
    }
    Ok(())
}
