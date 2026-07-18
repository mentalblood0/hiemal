use std::io::Write;
use std::{hash::Hash, sync::Arc};

use anyhow::{Context, Result, anyhow};
use glob::glob;
use gxhash::HashMap;

use crate::program::{DefaultArgument, From, Program};

pub struct IncludesCache {
    pub directory: std::path::PathBuf,
    pub source_to_program: HashMap<From, Arc<Program>>,
}

impl Default for IncludesCache {
    fn default() -> IncludesCache {
        Self {
            directory: dirs::cache_dir().unwrap().join("hiemal"),
            source_to_program: HashMap::default(),
        }
    }
}

impl IncludesCache {
    fn remove_from_disk(&self, source_hash_hex: &str) -> Result<()> {
        if let Some(Ok(path)) = glob(&format!(
            "{}.*",
            self.directory.join(source_hash_hex).to_str().unwrap()
        ))?
        .next()
        {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    fn get_from_disk(&mut self, from: &From) -> Result<Option<Arc<Program>>> {
        let source_hash = self.source_hash(from);
        if let Some(Ok(path)) = glob(&format!(
            "{}.*",
            self.directory
                .join(format!("{:x}", source_hash))
                .to_str()
                .unwrap()
        ))?
        .next()
        {
            let result_text = std::fs::read_to_string(&path)?;
            let result = match path
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .map(|extension| extension.to_lowercase())
                .unwrap()
                .as_str()
            {
                "yml" | "yaml" => serde_saphyr::from_str::<Arc<Program>>(&result_text)?,
                _ => return Ok(None),
            };
            self.source_to_program.insert(from.clone(), result.clone());
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    fn source_hash<T>(&self, source: T) -> u128
    where
        T: Hash,
    {
        let mut hasher = gxhash::GxHasher::default();
        source.hash(&mut hasher);
        hasher.finish_u128()
    }

    pub fn get(&mut self, from: &From) -> Result<Arc<Program>> {
        match from {
            From::DefaultArgument(_) => Ok(Arc::new(Program::DefaultArgument(
                DefaultArgument::Underline,
            ))),
            From::Url(url) => {
                match std::path::Path::new(url.path())
                    .extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .map(|extension| extension.to_lowercase())
                {
                    Some(extension) if extension == "yaml" || extension == "yml" => {
                        let source_hash = self.source_hash(url);
                        let source_hash_hex = format!("{source_hash:x}");
                        if let Some(result) = self.source_to_program.get(from) {
                            Ok(result.clone())
                        } else {
                            let extension = std::path::Path::new(url.path())
                                .extension()
                                .unwrap()
                                .to_str()
                                .unwrap();
                            let glob_pattern = format!(
                                "{}/{source_hash_hex}.*.*",
                                self.directory.to_str().unwrap()
                            );
                            let (response, etag) =
                                if let Some(Ok(path_with_etag)) = glob(&glob_pattern)?.next() {
                                    let file_name_splitted = path_with_etag
                                        .file_name()
                                        .unwrap()
                                        .to_str()
                                        .unwrap()
                                        .splitn(3, '.') // url hash, etag, extension
                                        .collect::<Vec<_>>();
                                    let current_etag = file_name_splitted[1].to_string();
                                    match ureq::get(url.as_str())
                                        .header(
                                            "If-None-Match",
                                            format!("\"{current_etag}\", W/\"{current_etag}\""),
                                        )
                                        .call()
                                    {
                                        Ok(response) => {
                                            if response.status() == 304 {
                                                return Ok(self.get_from_disk(from)?.unwrap());
                                            }
                                            let new_etag = response.headers()["etag"]
                                                .to_str()?
                                                .split("\"")
                                                .nth(1)
                                                .unwrap()
                                                .to_string(); // etag can be W/"<etag_value>" or "<etag_value>"
                                            (response, new_etag)
                                        }
                                        Err(
                                            ureq::Error::ConnectionFailed
                                            | ureq::Error::Timeout(_)
                                            | ureq::Error::BodyStalled,
                                        ) => {
                                            return Ok(self.get_from_disk(from)?.unwrap());
                                        }
                                        Err(error) => {
                                            return Err(error).with_context(|| {
                                                format!("Can not download include from {url}")
                                            });
                                        }
                                    }
                                } else {
                                    let response = ureq::get(url.as_str()).call()?;
                                    let etag = response.headers()["etag"]
                                        .to_str()?
                                        .split("\"")
                                        .nth(1)
                                        .unwrap()
                                        .to_string(); // etag can be W/"<etag_value>" or "<etag_value>"
                                    (response, etag)
                                };
                            if response.status().is_success() {
                                let result_text = response
                                    .into_body()
                                    .read_to_string()
                                    .with_context(|| "Can not read body of response from {url}")?;
                                let result = match extension {
                                    "yaml" | "yml" => {
                                        serde_saphyr::from_str::<Arc<Program>>(&result_text)?
                                    }
                                    unsupported_extension => {
                                        return Err(anyhow!(
                                            "Can not parse {unsupported_extension} program \
                                             downloaded from {url}"
                                        ));
                                    }
                                };
                                self.remove_from_disk(&source_hash_hex)?;
                                self.source_to_program.insert(from.clone(), result.clone());
                                let path = self
                                    .directory
                                    .join(format!("{}.{etag}.{extension}", source_hash_hex));
                                if let Some(parent) = path.parent() {
                                    std::fs::create_dir_all(parent)?;
                                }
                                std::fs::OpenOptions::new()
                                    .create(true)
                                    .truncate(true)
                                    .write(true)
                                    .open(path)?
                                    .write_all(result_text.as_bytes())?;
                                Ok(result)
                            } else {
                                Err(anyhow!("Can not download included file from {url}"))
                            }
                        }
                    }
                    extension => Err(anyhow!(
                        "Unsupported include file extension {extension:?} in url {url:?}"
                    )),
                }
            }
            From::File(path) => {
                if let Some(result) = self.source_to_program.get(from) {
                    Ok(result.clone())
                } else {
                    match path.extension() {
                        Some(ext) if ext == "yaml" || ext == "yml" => serde_saphyr::from_reader(
                            std::io::BufReader::new(std::fs::File::open(path.clone())?),
                        )
                        .with_context(|| format!("Can not parse included file at {path:?}")),
                        extension => Err(anyhow!(
                            "Unsupported include file extension {extension:?} in file path \
                             {path:?}"
                        )),
                    }
                }
            }
            From::Program(program) => Ok(program.clone()),
        }
    }
}
