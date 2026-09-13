use crate::error::{ErrorInfo, ErrorKind};
use crate::session::Cleaner;
use std::sync::{Arc, Mutex};

pub const KEYSTORE_SERVICE: &str = "com.voxa.app";
pub const KEYSTORE_USER: &str = "api-key";
const BASE_URL: &str = "https://api.groq.com/openai/v1";
const CLEANUP_MODEL: &str = "llama-3.3-70b-versatile";

pub struct GroqCleaner {
    preset: Arc<Mutex<String>>,
}

impl GroqCleaner {
    pub fn new(preset: Arc<Mutex<String>>) -> Self {
        Self { preset }
    }
}

impl Cleaner for GroqCleaner {
    fn clean(&self, raw: &str) -> String {
        let Some(key) = load_key().ok().flatten() else {
            return raw.to_string();
        };
        let preset = self
            .preset
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match clean_with_groq(&key, &preset, raw) {
            Ok(cleaned) if !cleaned.trim().is_empty() => cleaned,
            _ => raw.to_string(),
        }
    }
}

pub fn load_key() -> Result<Option<String>, ErrorInfo> {
    load_key_at(KEYSTORE_SERVICE, KEYSTORE_USER)
}

pub fn save_key(key: &str) -> Result<(), ErrorInfo> {
    save_key_at(KEYSTORE_SERVICE, KEYSTORE_USER, key)
}

pub fn delete_key() -> Result<(), ErrorInfo> {
    delete_key_at(KEYSTORE_SERVICE, KEYSTORE_USER)
}

fn load_key_at(service: &str, user: &str) -> Result<Option<String>, ErrorInfo> {
    let entry = match keyring::v1::Entry::new(service, user) {
        Ok(entry) => entry,
        Err(err) => return Err(keystore_err(err)),
    };
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::v1::Error::NoEntry) => Ok(None),
        Err(err) => Err(keystore_err(err)),
    }
}

fn save_key_at(service: &str, user: &str, key: &str) -> Result<(), ErrorInfo> {
    let entry = keyring::v1::Entry::new(service, user).map_err(keystore_err)?;
    entry.set_password(key).map_err(keystore_err)
}

fn delete_key_at(service: &str, user: &str) -> Result<(), ErrorInfo> {
    let entry = keyring::v1::Entry::new(service, user).map_err(keystore_err)?;
    entry.delete_credential().map_err(keystore_err)
}

pub fn test_key(key: &str) -> Result<bool, ErrorInfo> {
    let client = groq_client()?;
    match client
        .get(format!("{BASE_URL}/models"))
        .bearer_auth(key)
        .send()
    {
        Ok(response) if response.status().is_success() => Ok(true),
        Ok(response) => Err(ErrorInfo::new(
            ErrorKind::Cleanup,
            true,
            format!("cleanup provider rejected the key: {}", response.status()),
        )),
        Err(err) => Err(groq_err(err)),
    }
}

pub struct TonePreset {
    pub id: &'static str,
    pub label: &'static str,
}

pub fn presets() -> &'static [TonePreset] {
    &[
        TonePreset { id: "balanced", label: "Balanced" },
        TonePreset { id: "casual", label: "Casual" },
        TonePreset { id: "formal", label: "Formal" },
        TonePreset { id: "terse", label: "Terse" },
        TonePreset { id: "email", label: "Email body" },
    ]
}

fn prompt_for(preset: &str) -> &'static str {
    match preset {
        "casual" => {
            "You are a dictation cleanup pass. Keep the natural spoken tone. Add light \
             punctuation and capitalization, and drop filler words (uh, um, like) when they \
             add nothing. Do not rephrase or formalize. Return only the cleaned text."
        }
        "formal" => {
            "You are a dictation cleanup pass. Transform the raw transcript into polished \
             formal prose: full correct punctuation, capitalization and grammar, and slightly \
             formal phrasing, while keeping the meaning and the order of ideas. Return only \
             the cleaned text."
        }
        "terse" => {
            "You are a dictation cleanup pass. Summarize the raw transcript into its key \
             points in concise, readable prose. Keep the essential facts and their order. \
             Return only the cleaned text, no lists and no preamble."
        }
        "email" => {
            "You are a dictation cleanup pass. Turn the raw transcript into a coherent email \
             body: complete sentences with corrected punctuation and grammar, ready to paste \
             as-is. No salutation and no sign-off. Return only the body."
        }
        _ => {
            "You are a dictation cleanup pass. Fix the raw transcript: add correct \
             punctuation and capitalization, fix obvious typos and light grammar slips, and \
             keep the speaker's words and ordering. Return only the cleaned text."
        }
    }
}

fn clean_with_groq(key: &str, preset: &str, raw: &str) -> Result<String, ErrorInfo> {
    let client = groq_client()?;
    let body = serde_json::json!({
        "model": CLEANUP_MODEL,
        "temperature": 0.3,
        "messages": [
            { "role": "system", "content": prompt_for(preset) },
            { "role": "user", "content": raw },
        ],
    });
    let response = client
        .post(format!("{BASE_URL}/chat/completions"))
        .bearer_auth(key)
        .json(&body)
        .send()
        .map_err(groq_err)?;
    if !response.status().is_success() {
        return Err(ErrorInfo::new(
            ErrorKind::Cleanup,
            true,
            format!(
                "cleanup request failed ({}): {}",
                response.status(),
                response.text().unwrap_or_default()
            ),
        ));
    }
    let parsed: serde_json::Value = response.json().map_err(groq_err)?;
    let content = parsed["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| {
            ErrorInfo::new(
                ErrorKind::Cleanup,
                true,
                "cleanup provider returned no text",
            )
        })?
        .to_string();
    Ok(content)
}

fn groq_client() -> Result<reqwest::blocking::Client, ErrorInfo> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(groq_err)
}

fn groq_err(err: reqwest::Error) -> ErrorInfo {
    ErrorInfo::new(ErrorKind::Cleanup, true, format!("cleanup request failed: {err}"))
}

fn keystore_err(err: keyring::v1::Error) -> ErrorInfo {
    ErrorInfo::new(
        ErrorKind::Config,
        false,
        format!("keychain unavailable: {err}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_unique_and_have_prompts() {
        let mut ids = std::collections::HashSet::new();
        for preset in presets() {
            assert!(ids.insert(preset.id));
            assert!(!prompt_for(preset.id).is_empty());
        }
    }

    #[test]
    fn unknown_preset_falls_back_to_balanced() {
        assert_eq!(prompt_for("nope"), prompt_for("balanced"));
    }

    #[test]
    fn cleaner_without_key_passes_through() {
        let preset = Arc::new(Mutex::new("balanced".to_string()));
        let cleaner = GroqCleaner::new(preset);
        assert_eq!(cleaner.clean("hello world"), "hello world");
    }

    #[test]
    fn keystore_set_get_delete_roundtrip() {
        let service = "com.voxa.test";
        let user = "roundtrip";
        let _ = delete_key_at(service, user);
        assert_eq!(load_key_at(service, user).unwrap(), None);
        save_key_at(service, user, "sekrit").unwrap();
        assert_eq!(load_key_at(service, user).unwrap(), Some("sekrit".into()));
        delete_key_at(service, user).unwrap();
        assert_eq!(load_key_at(service, user).unwrap(), None);
    }
}