use std::{
    hash::Hash,
    io::{BufReader, Write},
};

use anyhow::{Context, Result};
use gxhash::GxHasher;

use hiemal::{
    compiler::Compiler, computer::Computer, includes_cache::IncludesCache, program::From,
};

macro_rules! time_it {
    ($action:expr, $code:block) => {{
        let start = std::time::Instant::now();
        let result = $code;
        eprintln!("{} in {:?}", $action, start.elapsed());
        result
    }};
}

pub fn run(target_option: &Option<String>, output_writer: &mut impl Write) -> Result<()> {
    let computer = Computer::default();
    if let Some(target) = target_option {
        let include_from = serde_saphyr::from_str::<From>(target)?;
        let program = IncludesCache::default().get(&include_from)?;
        let program_hash = {
            let mut hasher = GxHasher::default();
            program.hash(&mut hasher);
            hasher.finish_u128()
        };
        let cached_intermediate_representation_path = dirs::cache_dir()
            .unwrap()
            .join("hiemal")
            .join(format!("{program_hash:x}.bin"));
        let intermediate_representation = if let Ok(cached_intermediate_representation_file) =
            std::fs::File::open(&cached_intermediate_representation_path)
        {
            time_it!("loaded from cache", {
                bincode::serde::decode_from_std_read(
                    &mut lz4_flex::frame::FrameDecoder::new(&mut BufReader::new(
                        cached_intermediate_representation_file,
                    )),
                    bincode::config::standard(),
                )
                .with_context(|| {
                    format!(
                        "Can not decode cached intermediate representation from \
                         {cached_intermediate_representation_path:?}"
                    )
                })?
            })
        } else {
            let compiler = Compiler {
                metaprograms_computer: computer.clone(),
            };
            let result = time_it!("compiled", { compiler.compile(&program)? });
            time_it!("cached", {
                if let Some(parent) = cached_intermediate_representation_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut encoder = lz4_flex::frame::FrameEncoder::new(
                    std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .open(&cached_intermediate_representation_path)?,
                );
                bincode::serde::encode_into_std_write(
                    &result,
                    &mut encoder,
                    bincode::config::standard(),
                )
                .with_context(|| {
                    format!(
                        "Can not encode intermediate representation to \
                         {cached_intermediate_representation_path:?}"
                    )
                })?;
                encoder.finish().with_context(|| {
                    format!(
                        "Can not finish compress-write of intermediate representation to \
                         {cached_intermediate_representation_path:?}"
                    )
                })?;
                result
            })
        };
        serde_saphyr::to_io_writer(
            output_writer,
            &time_it!("computed", {
                computer
                    .compute(&intermediate_representation)
                    .context("Can not compute program")?
            }),
        )
        .context("Can not output result of the program computation")?;
    }
    Ok(())
}

fn main() -> Result<()> {
    run(&std::env::args().nth(1), &mut std::io::stdout())
}

#[cfg(test)]
mod tests {
    use std::io::{BufWriter, Write};

    use anyhow::Result;
    use pretty_assertions::assert_eq;

    use super::run;

    #[test]
    fn test_native() -> Result<()> {
        let mut output_writer = BufWriter::new(Vec::new());
        run(&Some("examples/tests.yml".to_string()), &mut output_writer)?;
        output_writer.flush()?;
        assert_eq!(String::from_utf8(output_writer.into_inner()?)?, "ok\n");
        Ok(())
    }
}
