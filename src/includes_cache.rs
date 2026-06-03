use std::collections::BTreeMap;
use std::io::Write;

use anyhow::{Context, Result, anyhow};
use glob::glob;
use url::Url;

use crate::{clause::IncludeFrom, program::Program};

pub type SourceHash = [u8; 16];

pub struct IncludesCache {
    pub directory: std::path::PathBuf,
    pub source_hash_to_program: BTreeMap<SourceHash, Program>,
}

impl Default for IncludesCache {
    fn default() -> IncludesCache {
        Self {
            directory: dirs::cache_dir().unwrap().join("hiemal"),
            source_hash_to_program: BTreeMap::new(),
        }
    }
}

impl IncludesCache {
    fn url_hash(&self, url: &Url) -> SourceHash {
        xxhash_rust::xxh3::xxh3_128(url.to_string().as_bytes()).to_be_bytes()
    }

    fn path_hash(&self, path: &std::path::PathBuf) -> SourceHash {
        xxhash_rust::xxh3::xxh3_128(path.to_str().unwrap().as_bytes()).to_be_bytes()
    }

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

    fn get_from_disk(&mut self, source_hash: SourceHash) -> Result<Option<Program>> {
        if let Some(Ok(path)) = glob(&format!(
            "{}.*",
            self.directory
                .join(hex::encode(source_hash))
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
                "json" => serde_json::from_str::<Program>(&result_text)?,
                "yml" | "yaml" => serde_saphyr::from_str::<Program>(&result_text)?,
                _ => return Ok(None),
            };
            self.source_hash_to_program
                .insert(source_hash, result.clone());
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    pub fn get(&mut self, from: &IncludeFrom) -> Result<Program> {
        match from {
            IncludeFrom::Url(url) => {
                match std::path::Path::new(url.path())
                    .extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .map(|extension| extension.to_lowercase())
                {
                    Some(extension)
                        if extension == "yaml" || extension == "yml" || extension == "json" =>
                    {
                        let source_hash = self.url_hash(url);
                        let source_hash_hex = hex::encode(source_hash);
                        if let Some(result) = self.source_hash_to_program.get(&source_hash) {
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
                            let (response, etag) = if let Some(Ok(path_with_etag)) =
                                glob(&glob_pattern)?.next()
                            {
                                let file_name_splitted = path_with_etag
                                    .file_name()
                                    .unwrap()
                                    .to_str()
                                    .unwrap()
                                    .splitn(3, '.') // url hash, etag, extension
                                    .collect::<Vec<_>>();
                                let etag = file_name_splitted[1].to_string();
                                match ureq::get(url.as_str())
                                    .header("If-None-Match", format!("\"{etag}\", W/\"{etag}\""))
                                    .call()
                                {
                                    Ok(response) => {
                                        if response.status() == 304 {
                                            return Ok(self.get_from_disk(source_hash)?.unwrap());
                                        }
                                        (response, etag)
                                    }
                                    Err(
                                        ureq::Error::ConnectionFailed
                                        | ureq::Error::Timeout(_)
                                        | ureq::Error::BodyStalled,
                                    ) => {
                                        return Ok(self.get_from_disk(source_hash)?.unwrap());
                                    }
                                    Err(error) => {
                                        return Err(error).with_context(|| {
                                            format!("Can not download include from {url}")
                                        });
                                    }
                                }
                            } else {
                                let response = ureq::get(url.as_str()).call()?;
                                let headers = response.headers();
                                let etag = headers["etag"]
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
                                    "json" => serde_json::from_str::<Program>(&result_text)?,
                                    "yaml" | "yml" => {
                                        serde_saphyr::from_str::<Program>(&result_text)?
                                    }
                                    unsupported_extension => {
                                        return Err(anyhow!(
                                            "Can not parse {unsupported_extension} program \
                                             downloaded from {url}"
                                        ));
                                    }
                                };
                                self.remove_from_disk(&source_hash_hex)?;
                                self.source_hash_to_program
                                    .insert(source_hash, result.clone());
                                let path = self
                                    .directory
                                    .join(&format!("{}.{etag}.{extension}", source_hash_hex));
                                if let Some(parent) = path.parent() {
                                    std::fs::create_dir_all(parent)?;
                                }
                                std::fs::OpenOptions::new()
                                    .create(true)
                                    .write(true)
                                    .open(path)?
                                    .write_all(result_text.as_bytes())?;
                                Ok(result)
                            } else {
                                Err(anyhow!("Can not download included file from {url}"))
                            }
                        }
                    }
                    extension => {
                        return Err(anyhow!(
                            "Unsupported include file extension {extension:?} in url {url:?}"
                        ));
                    }
                }
            }
            IncludeFrom::File(path) => {
                let source_hash = self.path_hash(path);
                if let Some(result) = self.source_hash_to_program.get(&source_hash) {
                    Ok(result.clone())
                } else {
                    match path.extension() {
                        Some(ext) if ext == "yaml" || ext == "yml" => serde_saphyr::from_reader(
                            std::io::BufReader::new(std::fs::File::open(path.clone())?),
                        )
                        .with_context(|| format!("Can not parse included file at {path:?}")),
                        Some(ext) if ext == "json" => serde_json::from_reader(
                            std::io::BufReader::new(std::fs::File::open(path.clone())?),
                        )
                        .with_context(|| format!("Can not parse included file at {path:?}")),
                        extension => {
                            return Err(anyhow!(
                                "Unsupported include file extension {extension:?} in file path \
                                 {path:?}"
                            ));
                        }
                    }
                }
            }
        }
    }
}
