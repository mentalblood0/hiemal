use anyhow::{anyhow, Context, Result};

use hiemal::{IncludesCache, Interpreter};

fn main() -> Result<()> {
    let mut includes_cache = IncludesCache::default();
    match std::env::args()
        .nth(1)
        .ok_or(anyhow!(
            "Can not automatically detect input program serialization format, please specify it \
             in the first command line argument, available options are: yaml, json"
        ))?
        .as_str()
    {
        "yaml" => serde_saphyr::to_io_writer(
            &mut std::io::stdout(),
            &Interpreter::default().compute(
                &serde_saphyr::from_reader(std::io::stdin())
                    .context("Can not parse the program")?,
                &mut includes_cache,
            )?,
        )
        .context("Can not output result of the program computation"),
        "json" => serde_json::to_writer(
            &mut std::io::stdout(),
            &Interpreter::default().compute(
                &serde_json::from_reader(std::io::stdin()).context("Can not parse the program")?,
                &mut includes_cache,
            )?,
        )
        .context("Can not output result of the program computation"),
        format => Err(anyhow!(
            "Can not process input program of specified serialization format {format:?}, \
             available options are: yaml, json"
        )),
    }
}
