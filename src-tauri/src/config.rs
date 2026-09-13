use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", default)]
pub struct Config {
    pub hotkey: String,
    pub mic_device: Option<String>,
    pub model_id: String,
    pub compute_device: String,
    pub tone_preset: String,
    pub launch_at_login: bool,
    pub sounds: bool,
    pub first_run: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Shift+Space".into(),
            mic_device: None,
            model_id: "small.en".into(),
            compute_device: "auto".into(),
            tone_preset: "balanced".into(),
            launch_at_login: false,
            sounds: false,
            first_run: true,
        }
    }
}

pub fn path(app_data: &PathBuf) -> PathBuf {
    app_data.join("config.json")
}

pub fn load(app_data: &PathBuf) -> Config {
    let path = path(app_data);
    fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(app_data: &PathBuf, cfg: &Config) -> io::Result<()> {
    fs::create_dir_all(app_data)?;
    let path = path(app_data);
    let tmp = path.with_extension("json.tmp");
    let raw = serde_json::to_string_pretty(cfg)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(&tmp, raw)?;
    fs::rename(&tmp, &path)
}