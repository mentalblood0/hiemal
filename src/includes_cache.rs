use std::collections::BTreeMap;
use std::io::Write;

use anyhow::{Context, Result, anyhow};
use glob::glob;
use url::Url;

pub struct IncludesCache {
    pub directory: std::path::PathBuf,
    pub url_hash_to_text: BTreeMap<String, String>,
}

impl Default for IncludesCache {
    fn default() -> IncludesCache {
        Self {
            directory: dirs::cache_dir().unwrap().join("hiemal"),
            url_hash_to_text: BTreeMap::new(),
        }
    }
}

impl IncludesCache {
    fn url_hash(&self, url: &Url) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(xxhash_rust::xxh3::xxh3_128(url.to_string().as_bytes()).to_be_bytes())
    }

    fn remove_from_disk(&self, url_hash: &str) -> Result<()> {
        if let Some(Ok(path)) = glob(&format!(
            "{}.*",
            self.directory.join(url_hash).to_str().unwrap()
        ))?
        .next()
        {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    fn add_cached(
        &mut self,
        text: String,
        url_hash: &str,
        etag: &str,
        extension: &str,
    ) -> Result<()> {
        self.remove_from_disk(url_hash)?;
        self.url_hash_to_text
            .insert(url_hash.to_string(), text.clone());
        let path = self
            .directory
            .join(&format!("{url_hash}.{etag}.{extension}"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(path)?
            .write_all(text.as_bytes())?;
        Ok(())
    }

    fn get_from_disk(&mut self, url_hash: &str) -> Result<Option<String>> {
        if let Some(Ok(path)) = glob(&format!(
            "{}.*",
            self.directory.join(url_hash).to_str().unwrap()
        ))?
        .next()
        {
            let result = std::fs::read_to_string(path)?;
            self.url_hash_to_text
                .insert(url_hash.to_string(), result.clone());
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    pub fn get(&mut self, url: &Url) -> Result<String> {
        let url_hash = self.url_hash(url);
        if let Some(result) = self.url_hash_to_text.get(&url_hash) {
            return Ok(result.clone());
        } else {
            let extension = std::path::Path::new(url.path())
                .extension()
                .unwrap()
                .to_str()
                .unwrap();
            let glob_pattern = format!("{}/{url_hash}.*.*", self.directory.to_str().unwrap());
            let (response, etag) = if let Some(Ok(path_with_etag)) = glob(&glob_pattern)?.next() {
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
                            return Ok(self.get_from_disk(&url_hash)?.unwrap());
                        }
                        (response, etag)
                    }
                    Err(
                        ureq::Error::ConnectionFailed
                        | ureq::Error::Timeout(_)
                        | ureq::Error::BodyStalled,
                    ) => {
                        return Ok(self.get_from_disk(&url_hash)?.unwrap());
                    }
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("Can not download include from {url}"));
                    }
                }
            } else {
                let response = ureq::get(url.as_str()).call()?;
                let headers = response.headers();
                let etag = headers["ETag"]
                    .to_str()?
                    .split("\"")
                    .nth(1)
                    .unwrap()
                    .to_string(); // etag can be W/"<etag_value>" or "<etag_value>"
                (response, etag)
            };
            if response.status().is_success() {
                let result = response
                    .into_body()
                    .read_to_string()
                    .with_context(|| "Can not read body of response from {url}")?;
                self.add_cached(result.clone(), &url_hash, &etag, extension)?;
                Ok(result)
            } else {
                Err(anyhow!("Can not download included file from {url}"))
            }
        }
    }
}
