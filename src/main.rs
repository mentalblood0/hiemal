use std::process::exit;

use anyhow::Context;
use url::Url;

use hiemal::{IncludesCache, Interpreter, ValueWithIncludes};

enum Source {
    Url,
    File,
}

enum Format {
    Yaml,
    Json,
}

fn main() {
    if let Some(target) = std::env::args().nth(1) {
        let (source, extension, optional_url) = if let Ok(url) = Url::parse(&target) {
            (
                Source::Url,
                std::path::Path::new(url.path())
                    .extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .map(|extension| extension.to_lowercase()),
                Some(url),
            )
        } else {
            (
                Source::File,
                std::path::Path::new(&target)
                    .extension()
                    .and_then(|extension| Some(extension.to_str().unwrap().to_string())),
                None,
            )
        };
        let format = if let Some(extension) = extension {
            match extension.as_str() {
                "yaml" | "yml" => Format::Yaml,
                "json" => Format::Json,
                _ => {
                    println!("No language but YAML or JSON");
                    exit(3);
                }
            }
        } else {
            println!("And then to extend and deepen");
            exit(2);
        };
        let mut includes_cache = IncludesCache::default();
        let program_text = match source {
            Source::File => std::fs::read_to_string(target)
                .context("Can not read file")
                .unwrap(),
            Source::Url => includes_cache.get(&optional_url.unwrap()).unwrap(),
        };
        let program: ValueWithIncludes = match format {
            Format::Yaml => serde_saphyr::from_str(&program_text)
                .context("Can not parse program")
                .unwrap(),
            Format::Json => serde_json::from_str(&program_text)
                .context("Can not parse program")
                .unwrap(),
        };
        serde_saphyr::to_io_writer(
            &mut std::io::stdout(),
            &Interpreter::default()
                .compute(&program, &mut includes_cache)
                .context("Can not compute program")
                .unwrap(),
        )
        .context("Can not output result of the program computation")
        .unwrap();
    } else {
        println!("The path is the goal");
        exit(1);
    }
}
