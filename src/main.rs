use std::{hash::Hash, io::BufReader};

use anyhow::{Context, Result};
use gxhash::GxHasher;

use hiemal::{compiler::compile, computer::Computer, includes_cache::IncludesCache, program::From};

fn main() -> Result<()> {
    if let Some(target) = std::env::args().nth(1) {
        let include_from = serde_json::from_value::<From>(serde_json::Value::String(target))?;
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
        } else {
            let result = compile(&program)?;
            std::fs::create_dir_all(cached_intermediate_representation_path.parent().unwrap())?;
            let mut encoder = lz4_flex::frame::FrameEncoder::new(
                std::fs::OpenOptions::new()
                    .create(true)
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
        };
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
