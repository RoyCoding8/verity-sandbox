use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use verity_core::json::{parse_json, to_json_string, JsonValue};
use verity_core::provider::{validate_external_sign_in_url, validate_provider_url, ProviderApi};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProviderProfile {
    pub id: String,
    pub name: String,
    pub api: ProviderApi,
    pub base_url: String,
    pub sign_in_url: Option<String>,
    pub models: Vec<String>,
    pub api_key: Option<String>,
}

/// WebView-safe projection of a provider profile.
/// Structurally omits raw `api_key` so secrets can never cross the IPC boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicProviderProfile {
    pub id: String,
    pub name: String,
    pub api: String,
    pub base_url: String,
    pub sign_in_url: Option<String>,
    pub models: Vec<String>,
    pub has_api_key: bool,
}

impl From<&StoredProviderProfile> for PublicProviderProfile {
    fn from(profile: &StoredProviderProfile) -> Self {
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            api: provider_api_str(profile.api).to_string(),
            base_url: profile.base_url.clone(),
            sign_in_url: profile.sign_in_url.clone(),
            models: profile.models.clone(),
            has_api_key: profile
                .api_key
                .as_deref()
                .is_some_and(|key| !key.is_empty()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderProfileInput {
    pub id: String,
    pub name: String,
    pub api: String,
    pub base_url: String,
    #[serde(default)]
    pub sign_in_url: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    /// `Some(non_empty)` sets/replaces the key; `Some("")` clears it; `None` preserves existing key.
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderStore {
    pub profiles: Vec<StoredProviderProfile>,
}

pub fn provider_api_str(api: ProviderApi) -> &'static str {
    match api {
        ProviderApi::OpenAiCompletions => "openai-completions",
        ProviderApi::OpenAiResponses => "openai-responses",
        ProviderApi::AnthropicMessages => "anthropic-messages",
    }
}

pub fn parse_provider_api(raw: &str) -> Result<ProviderApi, String> {
    match raw {
        "openai-completions" => Ok(ProviderApi::OpenAiCompletions),
        "openai-responses" => Ok(ProviderApi::OpenAiResponses),
        "anthropic-messages" => Ok(ProviderApi::AnthropicMessages),
        other => Err(format!(
            "unsupported provider api '{}' (expected openai-completions, openai-responses, or anthropic-messages)",
            other
        )),
    }
}

pub fn validate_provider_id(raw: &str) -> Result<String, String> {
    if raw.is_empty() || raw.len() > 64 {
        return Err("provider id must be 1..=64 ASCII characters".to_string());
    }
    if !raw
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("provider id may only contain ASCII letters, digits, '-' or '_'".to_string());
    }
    Ok(raw.to_string())
}

#[allow(dead_code)]
pub fn provider_env_key_name(id: &str) -> String {
    let suffix: String = id
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("VERITY_DESKTOP_KEY_{}", suffix)
}

pub fn default_store_path() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return PathBuf::from(xdg).join("verity").join("providers.json");
        }
    }
    if let Ok(home) = env::var("HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home)
                .join(".config")
                .join("verity")
                .join("providers.json");
        }
    }
    PathBuf::from(".verity").join("providers.json")
}

fn validate_optional_sign_in_url(raw: Option<String>) -> Result<Option<String>, String> {
    match raw {
        None => Ok(None),
        Some(url) if url.is_empty() => Ok(None),
        Some(url) => {
            validate_external_sign_in_url(&url).map_err(|e| e.to_string())?;
            Ok(Some(url))
        }
    }
}

fn sanitize_models(models: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for m in models {
        let trimmed = m.trim();
        if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn require_str<'a>(obj: &'a JsonValue, key: &str) -> Result<&'a str, String> {
    obj.get(key)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| format!("provider store missing string field '{}'", key))
}

fn optional_str(obj: &JsonValue, key: &str) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!(
            "provider store field '{}' must be string or null",
            key
        )),
    }
}

impl ProviderStore {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .map_err(|e| format!("failed to read provider store {}: {}", path.display(), e))?;
        Self::from_json(&raw)
    }

    pub fn from_json(raw: &str) -> Result<Self, String> {
        let root = parse_json(raw).map_err(|e| format!("invalid provider store json: {}", e))?;
        let arr = root
            .get("profiles")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| "provider store missing 'profiles' array".to_string())?;

        let mut profiles = Vec::with_capacity(arr.len());
        for item in arr {
            let id = validate_provider_id(require_str(item, "id")?)?;
            let name = require_str(item, "name")?.trim().to_string();
            if name.is_empty() {
                return Err(format!("provider '{}' has empty name", id));
            }
            let api = parse_provider_api(require_str(item, "api")?)?;
            let base_url = require_str(item, "base_url")?.to_string();
            validate_provider_url(&base_url).map_err(|e| e.to_string())?;
            let sign_in_url = validate_optional_sign_in_url(optional_str(item, "sign_in_url")?)?;

            let models = match item.get("models") {
                None | Some(JsonValue::Null) => Vec::new(),
                Some(JsonValue::Array(entries)) => {
                    let mut list = Vec::with_capacity(entries.len());
                    for entry in entries {
                        let model = entry
                            .as_str()
                            .ok_or_else(|| "provider models must be strings".to_string())?;
                        list.push(model.to_string());
                    }
                    sanitize_models(list)
                }
                Some(_) => return Err("provider 'models' must be an array".to_string()),
            };

            let api_key = optional_str(item, "api_key")?.filter(|k| !k.is_empty());

            profiles.push(StoredProviderProfile {
                id,
                name,
                api,
                base_url,
                sign_in_url,
                models,
                api_key,
            });
        }

        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Self { profiles })
    }

    pub fn to_json(&self) -> String {
        let items: Vec<JsonValue> = self
            .profiles
            .iter()
            .map(|p| {
                let mut obj = BTreeMap::new();
                obj.insert("id".to_string(), JsonValue::String(p.id.clone()));
                obj.insert("name".to_string(), JsonValue::String(p.name.clone()));
                obj.insert(
                    "api".to_string(),
                    JsonValue::String(provider_api_str(p.api).to_string()),
                );
                obj.insert(
                    "base_url".to_string(),
                    JsonValue::String(p.base_url.clone()),
                );
                obj.insert(
                    "sign_in_url".to_string(),
                    match &p.sign_in_url {
                        Some(u) => JsonValue::String(u.clone()),
                        None => JsonValue::Null,
                    },
                );
                obj.insert(
                    "models".to_string(),
                    JsonValue::Array(p.models.iter().cloned().map(JsonValue::String).collect()),
                );
                obj.insert(
                    "api_key".to_string(),
                    match &p.api_key {
                        Some(k) if !k.is_empty() => JsonValue::String(k.clone()),
                        _ => JsonValue::Null,
                    },
                );
                JsonValue::Object(obj)
            })
            .collect();

        let mut root = BTreeMap::new();
        root.insert("version".to_string(), JsonValue::Number(1.0));
        root.insert("profiles".to_string(), JsonValue::Array(items));
        to_json_string(&JsonValue::Object(root))
    }

    /// Atomically persists the store using a `0o600` temp file in the target
    /// directory followed by `rename` so secrets are never world-readable and
    /// interrupted writes never corrupt the existing file.
    pub fn save_atomic(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("provider store path has no parent: {}", path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create directory {}: {}", parent.display(), e))?;

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("invalid provider store filename: {}", path.display()))?;
        let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = parent.join(format!(".{}.tmp.{}.{}", file_name, std::process::id(), seq));

        let write_result = (|| -> Result<(), String> {
            let mut opts = OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }

            let mut file = opts
                .open(&tmp_path)
                .map_err(|e| format!("failed to create temp file {}: {}", tmp_path.display(), e))?;
            let payload = self.to_json();
            file.write_all(payload.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|e| format!("failed to write temp file {}: {}", tmp_path.display(), e))?;
            drop(file);

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600)).map_err(|e| {
                    format!(
                        "failed to enforce 0o600 permissions on {}: {}",
                        tmp_path.display(),
                        e
                    )
                })?;
            }

            fs::rename(&tmp_path, path).map_err(|e| {
                format!(
                    "failed to rename {} -> {}: {}",
                    tmp_path.display(),
                    path.display(),
                    e
                )
            })
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        write_result
    }

    pub fn public_profiles(&self) -> Vec<PublicProviderProfile> {
        self.profiles
            .iter()
            .map(PublicProviderProfile::from)
            .collect()
    }

    pub fn upsert(&mut self, input: ProviderProfileInput) -> Result<PublicProviderProfile, String> {
        let id = validate_provider_id(&input.id)?;
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err("provider name cannot be empty".to_string());
        }
        let api = parse_provider_api(&input.api)?;
        validate_provider_url(&input.base_url).map_err(|e| e.to_string())?;
        let sign_in_url = validate_optional_sign_in_url(input.sign_in_url)?;
        let models = sanitize_models(input.models);

        let existing_key = self
            .profiles
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.api_key.clone());

        let api_key = match input.api_key {
            Some(k) if k.is_empty() => None,
            Some(k) => Some(k),
            None => existing_key,
        };

        let profile = StoredProviderProfile {
            id: id.clone(),
            name,
            api,
            base_url: input.base_url,
            sign_in_url,
            models,
            api_key,
        };

        let public = PublicProviderProfile::from(&profile);
        if let Some(slot) = self.profiles.iter_mut().find(|p| p.id == id) {
            *slot = profile;
        } else {
            self.profiles.push(profile);
            self.profiles.sort_by(|a, b| a.id.cmp(&b.id));
        }

        Ok(public)
    }

    pub fn remove(&mut self, id: &str) -> Result<bool, String> {
        let id = validate_provider_id(id)?;
        let before = self.profiles.len();
        self.profiles.retain(|p| p.id != id);
        Ok(self.profiles.len() < before)
    }
}

/// Builds environment variable pairs injected strictly into the Pi child process:
/// - `VERITY_DESKTOP_PROVIDERS`: JSON array of provider metadata referencing key env var names (no raw secrets).
/// - `VERITY_DESKTOP_KEY_<ID>`: raw API key for each provider that has a configured key.
#[allow(dead_code)]
pub fn build_pi_provider_env(store: &ProviderStore) -> Vec<(String, String)> {
    let mut env_pairs = Vec::new();
    let mut metadata = Vec::with_capacity(store.profiles.len());

    for profile in &store.profiles {
        let key_env = provider_env_key_name(&profile.id);
        let has_key = profile
            .api_key
            .as_deref()
            .is_some_and(|key| !key.is_empty());

        if let Some(key) = &profile.api_key {
            if !key.is_empty() {
                env_pairs.push((key_env.clone(), key.clone()));
            }
        }

        let mut entry = BTreeMap::new();
        entry.insert("id".to_string(), JsonValue::String(profile.id.clone()));
        entry.insert("name".to_string(), JsonValue::String(profile.name.clone()));
        entry.insert(
            "api".to_string(),
            JsonValue::String(provider_api_str(profile.api).to_string()),
        );
        entry.insert(
            "base_url".to_string(),
            JsonValue::String(profile.base_url.clone()),
        );
        entry.insert(
            "models".to_string(),
            JsonValue::Array(
                profile
                    .models
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
        entry.insert(
            "api_key_env".to_string(),
            if has_key {
                JsonValue::String(key_env)
            } else {
                JsonValue::Null
            },
        );
        metadata.push(JsonValue::Object(entry));
    }

    let mut out = Vec::with_capacity(env_pairs.len() + 1);
    out.push((
        "VERITY_DESKTOP_PROVIDERS".to_string(),
        to_json_string(&JsonValue::Array(metadata)),
    ));
    out.extend(env_pairs);
    out
}

#[tauri::command]
pub fn list_providers() -> Result<Vec<PublicProviderProfile>, String> {
    let store = ProviderStore::load(&default_store_path())?;
    Ok(store.public_profiles())
}

#[tauri::command]
pub fn upsert_provider(input: ProviderProfileInput) -> Result<PublicProviderProfile, String> {
    let path = default_store_path();
    let mut store = ProviderStore::load(&path)?;
    let public = store.upsert(input)?;
    store.save_atomic(&path)?;
    Ok(public)
}

#[tauri::command]
pub fn delete_provider(id: String) -> Result<bool, String> {
    let path = default_store_path();
    let mut store = ProviderStore::load(&path)?;
    let removed = store.remove(&id)?;
    if removed {
        store.save_atomic(&path)?;
    }
    Ok(removed)
}

#[tauri::command]
pub fn validate_provider_sign_in_url(url: String) -> Result<String, String> {
    validate_external_sign_in_url(&url).map_err(|e| e.to_string())?;
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "verity-providers-{}-{}-{}",
            label,
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_persistence_enforces_0o600_and_roundtrips() {
        let dir = unique_temp_dir("atomic");
        let path = dir.join("providers.json");

        let mut store = ProviderStore::default();
        let pub_view = store
            .upsert(ProviderProfileInput {
                id: "local-vllm".to_string(),
                name: "Local vLLM".to_string(),
                api: "openai-completions".to_string(),
                base_url: "http://127.0.0.1:8000/v1".to_string(),
                sign_in_url: Some("https://auth.example.com/login".to_string()),
                models: vec!["qwen-2.5".to_string(), "qwen-2.5".to_string()],
                api_key: Some("sk-super-secret-123".to_string()),
            })
            .unwrap();

        assert!(pub_view.has_api_key);
        assert_eq!(pub_view.models, vec!["qwen-2.5"]);

        store.save_atomic(&path).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "provider store must have 0o600 permissions");
        }

        let reloaded = ProviderStore::load(&path).unwrap();
        assert_eq!(reloaded, store);

        // Idempotent update without api_key preserves existing secret.
        let mut updated = reloaded;
        let pub2 = updated
            .upsert(ProviderProfileInput {
                id: "local-vllm".to_string(),
                name: "Local vLLM Renamed".to_string(),
                api: "openai-responses".to_string(),
                base_url: "https://vllm.internal/v1".to_string(),
                sign_in_url: None,
                models: vec!["qwen-3".to_string()],
                api_key: None,
            })
            .unwrap();
        assert!(pub2.has_api_key);
        updated.save_atomic(&path).unwrap();

        let reloaded2 = ProviderStore::load(&path).unwrap();
        assert_eq!(
            reloaded2.profiles[0].api_key.as_deref(),
            Some("sk-super-secret-123")
        );

        // Idempotent delete returns true once, then false.
        assert!(updated.remove("local-vllm").unwrap());
        assert!(!updated.remove("local-vllm").unwrap());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_invalid_provider_and_sign_in_urls() {
        let mut store = ProviderStore::default();

        for bad_url in [
            "ftp://example.com/v1",
            "https://user:pass@api.example.com/v1",
            "https://api.example.com/v1?token=leak",
            "https://api.example.com/v1#frag",
            "http://example.com/v1 ",
        ] {
            let err = store
                .upsert(ProviderProfileInput {
                    id: "bad-url".to_string(),
                    name: "Bad".to_string(),
                    api: "openai-completions".to_string(),
                    base_url: bad_url.to_string(),
                    sign_in_url: None,
                    models: vec![],
                    api_key: None,
                })
                .unwrap_err();
            assert!(!err.is_empty(), "expected rejection for {}", bad_url);
        }

        // External sign-in URL strictly requires https://
        let err = store
            .upsert(ProviderProfileInput {
                id: "bad-signin".to_string(),
                name: "Bad Signin".to_string(),
                api: "anthropic-messages".to_string(),
                base_url: "https://api.anthropic.com/v1".to_string(),
                sign_in_url: Some("http://auth.example.com/login".to_string()),
                models: vec![],
                api_key: None,
            })
            .unwrap_err();
        assert!(err.contains("https://"), "unexpected error: {}", err);
    }

    #[test]
    fn public_profile_redacts_secret_and_env_injects_strictly() {
        let mut store = ProviderStore::default();
        let secret = "sk-live-secret-do-not-leak-999";

        store
            .upsert(ProviderProfileInput {
                id: "my-anthropic".to_string(),
                name: "Anthropic Proxy".to_string(),
                api: "anthropic-messages".to_string(),
                base_url: "https://proxy.example.com/v1".to_string(),
                sign_in_url: None,
                models: vec!["claude-sonnet".to_string()],
                api_key: Some(secret.to_string()),
            })
            .unwrap();
        store
            .upsert(ProviderProfileInput {
                id: "ollama".to_string(),
                name: "Ollama".to_string(),
                api: "openai-completions".to_string(),
                base_url: "http://localhost:11434/v1".to_string(),
                sign_in_url: None,
                models: vec!["llama3".to_string()],
                api_key: None,
            })
            .unwrap();

        let public = store.public_profiles();
        assert_eq!(public.len(), 2);
        let debug_repr = format!("{:?}", public);
        assert!(!debug_repr.contains(secret));
        assert!(
            public
                .iter()
                .find(|p| p.id == "my-anthropic")
                .unwrap()
                .has_api_key
        );
        assert!(
            !public
                .iter()
                .find(|p| p.id == "ollama")
                .unwrap()
                .has_api_key
        );

        let env = build_pi_provider_env(&store);
        assert_eq!(env[0].0, "VERITY_DESKTOP_PROVIDERS");
        assert!(
            !env[0].1.contains(secret),
            "VERITY_DESKTOP_PROVIDERS JSON must never contain raw api_key"
        );

        let parsed = parse_json(&env[0].1).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let anth = arr
            .iter()
            .find(|v| v.get("id").and_then(JsonValue::as_str) == Some("my-anthropic"))
            .unwrap();
        assert_eq!(
            anth.get("api_key_env").and_then(JsonValue::as_str),
            Some("VERITY_DESKTOP_KEY_MY_ANTHROPIC")
        );
        let ollama = arr
            .iter()
            .find(|v| v.get("id").and_then(JsonValue::as_str) == Some("ollama"))
            .unwrap();
        assert_eq!(ollama.get("api_key_env"), Some(&JsonValue::Null));

        assert!(env.contains(&(
            "VERITY_DESKTOP_KEY_MY_ANTHROPIC".to_string(),
            secret.to_string()
        )));
        assert!(!env.iter().any(|(k, _)| k == "VERITY_DESKTOP_KEY_OLLAMA"));
    }
}
