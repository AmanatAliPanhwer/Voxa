use crate::error::{ErrorInfo, ErrorKind};
use sha1::{Digest, Sha1};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub size_mb: u64,
    pub sha1: &'static str,
}

const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "tiny.en",
        name: "Tiny (English)",
        file: "ggml-tiny.en.bin",
        size_mb: 75,
        sha1: "c78c86eb1a8faa21b369bcd33207cc90d64ae9df",
    },
    ModelSpec {
        id: "base.en",
        name: "Base (English)",
        file: "ggml-base.en.bin",
        size_mb: 142,
        sha1: "137c40403d78fd54d454da0f9bd998f78703390c",
    },
    ModelSpec {
        id: "small.en",
        name: "Small (English)",
        file: "ggml-small.en.bin",
        size_mb: 466,
        sha1: "db8a495a91d927739e50b3fc1cc4c6b8f6c2d022",
    },
    ModelSpec {
        id: "medium.en",
        name: "Medium (English)",
        file: "ggml-medium.en.bin",
        size_mb: 1530,
        sha1: "8c30f0e44ce9560643ebd10bbe50cd20eafd3723",
    },
];

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|spec| spec.id == id)
}

pub fn all() -> &'static [ModelSpec] {
    MODELS
}

pub fn file_path(models_dir: &Path, spec: &ModelSpec) -> PathBuf {
    models_dir.join(spec.file)
}

pub fn downloaded(models_dir: &Path, spec: &ModelSpec) -> bool {
    file_path(models_dir, spec).is_file()
}

pub fn ensure(
    models_dir: &Path,
    spec: &ModelSpec,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ErrorInfo> {
    fs::create_dir_all(models_dir)
        .map_err(|err| dl_err(format!("cannot create models dir: {err}")))?;
    let final_path = file_path(models_dir, spec);
    if verify_file(&final_path, spec.sha1).unwrap_or(false) {
        progress(100.0);
        return Ok(final_path);
    }
    let _ = fs::remove_file(&final_path);
    let part_path = models_dir.join(format!("{}.part", spec.file));
    download(spec, &part_path, &mut progress)?;
    if !verify_file(&part_path, spec.sha1).unwrap_or(false) {
        let _ = fs::remove_file(&part_path);
        return Err(dl_err(format!("checksum mismatch for {}", spec.file)));
    }
    fs::rename(&part_path, &final_path)
        .map_err(|err| dl_err(format!("cannot finalize model file: {err}")))?;
    progress(100.0);
    Ok(final_path)
}

fn download(
    spec: &ModelSpec,
    part_path: &Path,
    progress: &mut dyn FnMut(f32),
) -> Result<(), ErrorInfo> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|err| dl_err(format!("http client failed: {err}")))?;
    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        spec.file
    );
    let started = fs::metadata(part_path).map(|meta| meta.len()).unwrap_or(0);
    let mut request = client.get(&url);
    if started > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={started}-"));
    }
    let mut response = request
        .send()
        .map_err(|err| dl_err(format!("download failed: {err}")))?;
    let mut resumed = started > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() && !resumed {
        if started > 0 {
            let _ = fs::remove_file(part_path);
            let fresh_response = client
                .get(&url)
                .send()
                .map_err(|err| dl_err(format!("download retry failed: {err}")))?;
            if !fresh_response.status().is_success() {
                return Err(dl_err(format!("download failed with HTTP {}", fresh_response.status())));
            }
            response = fresh_response;
            resumed = false;
        } else {
            return Err(dl_err(format!("download failed with HTTP {}", response.status())));
        }
    }
    let mut file = if resumed {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(part_path)
            .map_err(|err| dl_err(format!("cannot open part file: {err}")))?
    } else {
        fs::File::create(part_path)
            .map_err(|err| dl_err(format!("cannot create part file: {err}")))?
    };
    let mut downloaded = if resumed { started } else { 0 };
    let total = (spec.size_mb * 1024 * 1024).max(1) as f32;
    let mut last_progress = Instant::now();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buf)
            .map_err(|err| dl_err(format!("download interrupted: {err}")))?;
        if read == 0 {
            break;
        }
        file.write_all(&buf[..read])
            .map_err(|err| dl_err(format!("cannot write part file: {err}")))?;
        downloaded += read as u64;
        if last_progress.elapsed().as_millis() >= 150 {
            progress(((downloaded as f32 / total).min(1.0)) * 100.0);
            last_progress = Instant::now();
        }
    }
    let _ = file.sync_all();
    Ok(())
}

fn verify_file(path: &Path, expected: &str) -> io::Result<bool> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    let mut hasher = Sha1::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex(&hasher.finalize()) == expected.to_lowercase())
}

#[cfg(test)]
pub fn sha1_hex(bytes: &[u8]) -> String {
    hex(&Sha1::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn dl_err(detail: impl Into<String>) -> ErrorInfo {
    ErrorInfo::new(ErrorKind::Transcribe, true, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("voxa-model-test-{}", std::process::id()))
    }

    #[test]
    fn registry_lists_the_four_english_models() {
        assert_eq!(all().len(), 4);
        assert!(all().iter().all(|spec| spec.id.ends_with(".en")));
    }

    #[test]
    fn every_checksum_is_a_sha1_hex_string() {
        for spec in all() {
            assert_eq!(spec.sha1.len(), 40);
            assert!(spec.sha1.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn find_resolves_known_ids_and_rejects_unknown() {
        assert_eq!(find("small.en").map(|spec| spec.file), Some("ggml-small.en.bin"));
        assert!(find("large-v3").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn verify_file_matches_streamed_sha1() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("probe.bin");
        fs::write(&path, b"hello voxa").unwrap();
        let digest = sha1_hex(b"hello voxa");
        assert!(verify_file(&path, &digest).unwrap());
        let wrong = "0000000000000000000000000000000000000000";
        assert!(!verify_file(&path, wrong).unwrap());
        assert!(!verify_file(&dir.join("missing.bin"), &digest).unwrap());
        fs::remove_dir_all(&dir).ok();
    }
}