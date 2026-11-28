use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use verity_core::json::{parse_json, to_json_string, JsonValue};
use verity_core::provider::{validate_external_sign_in_url, validate_provider_url};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static STORE_WRITE_LOCK: Mutex<()> = Mutex::new(());

pub const VERITY_PI_EXTENSION_FILENAME: &str = "verity-extension.ts";
pub const VERITY_PI_EXTENSION_SOURCE: &str = include_str!("../resources/verity-extension.ts");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderApi {
    OpenAiCompletions,
    OpenAiResponses,
    AnthropicMessages,
    GoogleGenerativeAi,
}

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

#[derive(Clone, Deserialize)]
pub struct ProviderModelScanInput {
    pub provider_id: Option<String>,
    pub api: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

const MODEL_SCAN_BODY_LIMIT: u64 = 1024 * 1024;

fn models_url(base_url: &str) -> Result<reqwest::Url, String> {
    validate_provider_url(base_url).map_err(|_| "Invalid provider base URL".to_string())?;
    let mut url =
        reqwest::Url::parse(base_url).map_err(|_| "Invalid provider base URL".to_string())?;
    let path = url.path().trim_end_matches('/');
    let path = if path.is_empty() {
        "/v1/models".to_string()
    } else {
        format!("{path}/models")
    };
    url.set_path(&path);
    Ok(url)
}

fn scan_models_at(
    store_path: &Path,
    input: ProviderModelScanInput,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let api = parse_provider_api(&input.api).map_err(|_| "Unsupported provider API".to_string())?;
    if api == ProviderApi::GoogleGenerativeAi {
        return Err(
            "Model scanning is not supported for Google Generative AI; add model IDs manually"
                .to_string(),
        );
    }
    let url = models_url(&input.base_url)?;
    let key = match input.api_key {
        Some(key) => Some(key),
        None if input.provider_id.is_none() => None,
        None => {
            let _guard = STORE_WRITE_LOCK
                .lock()
                .map_err(|_| "Provider store unavailable")?;
            let store = ProviderStore::load(store_path)
                .map_err(|_| "Could not read saved provider credentials")?;
            store
                .profiles
                .iter()
                .find(|profile| {
                    Some(&profile.id) == input.provider_id.as_ref()
                        && profile.api == api
                        && models_url(&profile.base_url).ok().as_ref() == Some(&url)
                })
                .and_then(|profile| profile.api_key.clone())
        }
    };
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|_| "Could not initialize model scan")?;
    let mut request = client.get(url);
    if api == ProviderApi::AnthropicMessages {
        request = request.header("anthropic-version", "2023-06-01");
        if let Some(key) = key.filter(|key| !key.is_empty()) {
            request = request.header("x-api-key", key);
        }
    } else if let Some(key) = key.filter(|key| !key.is_empty()) {
        request = request.bearer_auth(key);
    }
    let response = request.send().map_err(|error| {
        if error.is_timeout() {
            "Model scan timed out; try again"
        } else {
            "Could not connect to model catalog; check the endpoint and credentials"
        }
        .to_string()
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "Model catalog returned HTTP {}; check the endpoint and credentials",
            response.status().as_u16()
        ));
    }
    let mut body = Vec::new();
    response
        .take(MODEL_SCAN_BODY_LIMIT + 1)
        .read_to_end(&mut body)
        .map_err(|_| "Could not read model catalog; try again")?;
    if body.len() as u64 > MODEL_SCAN_BODY_LIMIT {
        return Err("Model catalog exceeded the 1 MiB limit".to_string());
    }
    #[derive(Deserialize)]
    struct Catalog {
        data: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }
    let catalog: Catalog = serde_json::from_slice(&body)
        .map_err(|_| "Invalid model catalog: expected data containing model IDs")?;
    let mut models: Vec<String> = catalog
        .data
        .into_iter()
        .map(|model| model.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

#[tauri::command]
pub async fn scan_provider_models(input: ProviderModelScanInput) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        scan_models_at(&default_store_path(), input, Duration::from_secs(10))
    })
    .await
    .map_err(|_| "Model scan task failed".to_string())?
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
        ProviderApi::GoogleGenerativeAi => "google-generative-ai",
    }
}

pub fn parse_provider_api(raw: &str) -> Result<ProviderApi, String> {
    match raw {
        "openai-completions" => Ok(ProviderApi::OpenAiCompletions),
        "openai-responses" => Ok(ProviderApi::OpenAiResponses),
        "anthropic-messages" => Ok(ProviderApi::AnthropicMessages),
        "google-generative-ai" => Ok(ProviderApi::GoogleGenerativeAi),
        other => Err(format!(
            "unsupported provider api '{}' (expected openai-completions, openai-responses, anthropic-messages, or google-generative-ai)",
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

fn non_empty_env(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

pub fn resolve_verity_dir_from_env(
    home: Option<&str>,
    userprofile: Option<&str>,
    homedrive: Option<&str>,
    homepath: Option<&str>,
) -> PathBuf {
    if let Some(dir) = non_empty_env(home).or_else(|| non_empty_env(userprofile)) {
        return PathBuf::from(dir).join(".verity");
    }
    if let (Some(drive), Some(path)) = (non_empty_env(homedrive), non_empty_env(homepath)) {
        return PathBuf::from(format!("{}{}", drive, path)).join(".verity");
    }
    PathBuf::from(".verity")
}

pub fn default_verity_dir() -> PathBuf {
    let home = env::var("HOME").ok();
    let userprofile = env::var("USERPROFILE").ok();
    let homedrive = env::var("HOMEDRIVE").ok();
    let homepath = env::var("HOMEPATH").ok();
    resolve_verity_dir_from_env(
        home.as_deref(),
        userprofile.as_deref(),
        homedrive.as_deref(),
        homepath.as_deref(),
    )
}

pub fn resolve_store_path_from_env(
    home: Option<&str>,
    userprofile: Option<&str>,
    homedrive: Option<&str>,
    homepath: Option<&str>,
) -> PathBuf {
    resolve_verity_dir_from_env(home, userprofile, homedrive, homepath).join("providers.json")
}

pub fn default_store_path() -> PathBuf {
    default_verity_dir().join("providers.json")
}

pub fn resolve_pi_extension_path_from_env(
    home: Option<&str>,
    userprofile: Option<&str>,
    homedrive: Option<&str>,
    homepath: Option<&str>,
) -> PathBuf {
    resolve_verity_dir_from_env(home, userprofile, homedrive, homepath)
        .join("extensions")
        .join(VERITY_PI_EXTENSION_FILENAME)
}

pub fn default_pi_extension_path() -> PathBuf {
    default_verity_dir()
        .join("extensions")
        .join(VERITY_PI_EXTENSION_FILENAME)
}

/// Idempotently writes the embedded Verity Pi provider extension to `path`
/// only when missing or when its bytes differ. Uses atomic temp-file + rename
/// so interrupted writes never leave a partial extension on disk.
pub fn ensure_verity_pi_extension_at(path: &Path) -> Result<PathBuf, String> {
    if let Ok(existing) = fs::read(path) {
        if existing == VERITY_PI_EXTENSION_SOURCE.as_bytes() {
            return Ok(path.to_path_buf());
        }
    }

    let parent = path
        .parent()
        .ok_or_else(|| format!("extension path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| {
        format!(
            "failed to create extension directory {}: {}",
            parent.display(),
            e
        )
    })?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("invalid extension filename: {}", path.display()))?;
    let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!(".{}.tmp.{}.{}", file_name, std::process::id(), seq));

    let write_result = (|| -> Result<(), String> {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        let mut file = opts.open(&tmp_path).map_err(|e| {
            format!(
                "failed to create temp extension {}: {}",
                tmp_path.display(),
                e
            )
        })?;
        file.write_all(VERITY_PI_EXTENSION_SOURCE.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| {
                format!(
                    "failed to write temp extension {}: {}",
                    tmp_path.display(),
                    e
                )
            })?;
        drop(file);

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
    write_result?;
    Ok(path.to_path_buf())
}

/// Ensures the embedded Verity Pi provider extension is materialized at
/// `~/.verity/extensions/verity-extension.ts`.
pub fn ensure_verity_pi_extension_path() -> Result<PathBuf, String> {
    ensure_verity_pi_extension_at(&default_pi_extension_path())
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
        let env_key = provider_env_key_name(&id);
        if let Some(conflict) = self
            .profiles
            .iter()
            .find(|p| p.id != id && provider_env_key_name(&p.id) == env_key)
        {
            return Err(format!(
                "provider id '{}' collides with existing provider '{}' environment key '{}'",
                id, conflict.id, env_key
            ));
        }

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
pub async fn list_providers() -> Result<Vec<PublicProviderProfile>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let _guard = STORE_WRITE_LOCK
            .lock()
            .expect("provider store lock poisoned");
        let store = ProviderStore::load(&default_store_path())?;
        Ok(store.public_profiles())
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn upsert_provider(
    state: tauri::State<'_, crate::pi_ipc::PiAppState>,
    input: ProviderProfileInput,
) -> Result<PublicProviderProfile, String> {
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let public = {
            let _guard = STORE_WRITE_LOCK
                .lock()
                .expect("provider store lock poisoned");
            let path = default_store_path();
            let mut store = ProviderStore::load(&path)?;
            let public = store.upsert(input)?;
            store.save_atomic(&path)?;
            public
        };
        app_state.notify_providers_changed();
        Ok(public)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn delete_provider(
    state: tauri::State<'_, crate::pi_ipc::PiAppState>,
    id: String,
) -> Result<bool, String> {
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let removed = {
            let _guard = STORE_WRITE_LOCK
                .lock()
                .expect("provider store lock poisoned");
            let path = default_store_path();
            let mut store = ProviderStore::load(&path)?;
            let removed = store.remove(&id)?;
            if removed {
                store.save_atomic(&path)?;
            }
            removed
        };
        if removed {
            app_state.notify_providers_changed();
        }
        Ok(removed)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn validate_provider_sign_in_url(url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        validate_external_sign_in_url(&url).map_err(|e| e.to_string())?;
        Ok(url)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
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

    fn catalog_server(
        body: String,
        status: &str,
        delay: Duration,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let status = status.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            thread::sleep(delay);
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            String::from_utf8(request).unwrap().to_lowercase()
        });
        (base, handle)
    }

    #[test]
    fn model_scan_http_paths_credentials_and_storage_are_independent() {
        let dir = unique_temp_dir("scan");
        let path = dir.join("providers.json");
        for (suffix, expected_path, api, supplied, matched, expected_key) in [
            (
                "",
                "/v1/models",
                "openai-completions",
                None,
                true,
                Some("saved-secret"),
            ),
            (
                "/v1/",
                "/v1/models",
                "openai-responses",
                Some("override-secret"),
                true,
                Some("override-secret"),
            ),
            (
                "/deployment/api/v1",
                "/deployment/api/v1/models",
                "openai-completions",
                Some(""),
                true,
                None,
            ),
            (
                "/proxy",
                "/proxy/models",
                "openai-completions",
                None,
                false,
                None,
            ),
            (
                "/v1",
                "/v1/models",
                "anthropic-messages",
                Some("anthropic-secret"),
                true,
                Some("anthropic-secret"),
            ),
        ] {
            let (base, server) = catalog_server(
                r#"{"data":[{"id":" z "},{"id":"a"},{"id":"z"},{"id":" "}]}"#.to_string(),
                "200 OK",
                Duration::ZERO,
            );
            let base_url = format!("{base}{suffix}");
            let mut store = ProviderStore::default();
            store
                .upsert(ProviderProfileInput {
                    id: "scan".into(),
                    name: "Scan".into(),
                    api: api.into(),
                    base_url: if matched {
                        base_url.clone()
                    } else {
                        format!("{base}/other")
                    },
                    sign_in_url: None,
                    models: vec!["manual".into()],
                    api_key: Some("saved-secret".into()),
                })
                .unwrap();
            store.save_atomic(&path).unwrap();
            let before = fs::read(&path).unwrap();
            let models = scan_models_at(
                &path,
                ProviderModelScanInput {
                    provider_id: Some("scan".into()),
                    api: api.into(),
                    base_url,
                    api_key: supplied.map(str::to_string),
                },
                Duration::from_secs(2),
            )
            .unwrap();
            assert_eq!(models, ["a", "z"]);
            assert_eq!(fs::read(&path).unwrap(), before);
            let request = server.join().unwrap();
            assert!(
                request.starts_with(&format!("get {expected_path} http/1.1\r\n")),
                "{request}"
            );
            if api == "anthropic-messages" {
                assert!(request.contains("anthropic-version: 2023-06-01\r\n"));
                assert!(request.contains("x-api-key: anthropic-secret\r\n"));
                assert!(!request.contains("authorization:"));
            } else if let Some(key) = expected_key {
                assert!(request.contains(&format!("authorization: bearer {key}\r\n")));
            } else {
                assert!(!request.contains("authorization:"));
                assert!(!request.contains("saved-secret"));
            }
        }
        for (id, api) in [
            (None, "openai-completions"),
            (Some("different"), "openai-completions"),
            (Some("scan"), "openai-responses"),
        ] {
            let (base, server) = catalog_server("{\"data\":[]}".into(), "200 OK", Duration::ZERO);
            let mut store = ProviderStore::default();
            store
                .upsert(ProviderProfileInput {
                    id: "scan".into(),
                    name: "Scan".into(),
                    api: "openai-completions".into(),
                    base_url: base.clone(),
                    sign_in_url: None,
                    models: vec![],
                    api_key: Some("saved-secret".into()),
                })
                .unwrap();
            store.save_atomic(&path).unwrap();
            assert!(scan_models_at(
                &path,
                ProviderModelScanInput {
                    provider_id: id.map(str::to_string),
                    api: api.into(),
                    base_url: base,
                    api_key: None
                },
                Duration::from_secs(2)
            )
            .unwrap()
            .is_empty());
            assert!(!server.join().unwrap().contains("saved-secret"));
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn model_scan_rejects_bad_catalogs_redirects_and_excessive_responses() {
        let dir = unique_temp_dir("scan-errors");
        let path = dir.join("absent.json");
        for (body, status, delay, expected) in [
            (
                "not-json".into(),
                "200 OK",
                Duration::ZERO,
                "Invalid model catalog",
            ),
            (
                "{}".into(),
                "200 OK",
                Duration::ZERO,
                "Invalid model catalog",
            ),
            (
                "{\"data\":[{\"id\":3}]}".into(),
                "200 OK",
                Duration::ZERO,
                "Invalid model catalog",
            ),
            (
                "{\"data\":null}".into(),
                "200 OK",
                Duration::ZERO,
                "Invalid model catalog",
            ),
            (
                "secret-body".into(),
                "401 Unauthorized",
                Duration::ZERO,
                "HTTP 401",
            ),
            (
                "secret-body".into(),
                "302 Found\r\nLocation: http://127.0.0.1:1/secret-destination",
                Duration::ZERO,
                "HTTP 302",
            ),
            (
                "x".repeat(MODEL_SCAN_BODY_LIMIT as usize + 1),
                "200 OK",
                Duration::ZERO,
                "1 MiB",
            ),
            (
                "{\"data\":[]}".into(),
                "200 OK",
                Duration::from_millis(150),
                "timed out",
            ),
        ] {
            let (base, server) = catalog_server(body, status, delay);
            let error = scan_models_at(
                &path,
                ProviderModelScanInput {
                    provider_id: None,
                    api: "openai-completions".into(),
                    base_url: base.clone(),
                    api_key: Some("request-secret".into()),
                },
                if delay.is_zero() {
                    Duration::from_secs(2)
                } else {
                    Duration::from_millis(75)
                },
            )
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
            for forbidden in [
                "secret-body",
                "request-secret",
                base.as_str(),
                "secret-destination",
            ] {
                assert!(!error.contains(forbidden));
            }
            server.join().unwrap();
        }
        for base in [
            "https://example.com/v1?q=secret",
            "https://example.com/v1#secret",
            "https://user:secret@example.com/v1",
            "ftp://example.com/v1",
        ] {
            assert!(models_url(base).is_err());
        }
        let error = scan_models_at(
            &path,
            ProviderModelScanInput {
                provider_id: None,
                api: "google-generative-ai".into(),
                base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
                api_key: None,
            },
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.contains("not supported"));
        assert!(!path.exists());
        fs::remove_dir_all(dir).unwrap();
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
    fn supports_google_generative_ai_and_rejects_env_key_collisions() {
        let mut store = ProviderStore::default();
        let google = store
            .upsert(ProviderProfileInput {
                id: "my-gemini".to_string(),
                name: "Google Gemini".to_string(),
                api: "google-generative-ai".to_string(),
                base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
                sign_in_url: None,
                models: vec!["gemini-2.5-pro".to_string()],
                api_key: Some("gm-secret".to_string()),
            })
            .unwrap();
        assert_eq!(google.api, "google-generative-ai");

        // Distinct ID that normalizes to the same VERITY_DESKTOP_KEY_MY_GEMINI must be rejected.
        let err = store
            .upsert(ProviderProfileInput {
                id: "my_gemini".to_string(),
                name: "Collision".to_string(),
                api: "google-generative-ai".to_string(),
                base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
                sign_in_url: None,
                models: vec![],
                api_key: None,
            })
            .unwrap_err();
        assert!(err.contains("collides"));
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

    #[test]
    fn store_path_prefers_unix_home() {
        let path = resolve_store_path_from_env(
            Some("/Users/alice"),
            Some("C:\\Users\\alice"),
            Some("C:"),
            Some("\\Users\\bob"),
        );
        assert_eq!(
            path,
            PathBuf::from("/Users/alice")
                .join(".verity")
                .join("providers.json")
        );
    }

    #[test]
    fn store_path_uses_windows_userprofile_when_home_unset() {
        let path = resolve_store_path_from_env(
            None,
            Some("C:\\Users\\alice"),
            Some("D:"),
            Some("\\Users\\bob"),
        );
        assert_eq!(
            path,
            PathBuf::from("C:\\Users\\alice")
                .join(".verity")
                .join("providers.json")
        );
    }

    #[test]
    fn store_path_uses_windows_homedrive_and_homepath() {
        let path = resolve_store_path_from_env(None, None, Some("C:"), Some("\\Users\\alice"));
        assert_eq!(
            path,
            PathBuf::from("C:\\Users\\alice")
                .join(".verity")
                .join("providers.json")
        );
    }

    #[test]
    fn store_path_skips_blank_env_vars() {
        assert_eq!(
            resolve_store_path_from_env(Some("   "), Some("C:\\Users\\alice"), None, None),
            PathBuf::from("C:\\Users\\alice")
                .join(".verity")
                .join("providers.json")
        );
        assert_eq!(
            resolve_store_path_from_env(Some(""), Some(" \t "), Some("C:"), Some("\\Users\\alice")),
            PathBuf::from("C:\\Users\\alice")
                .join(".verity")
                .join("providers.json")
        );
        assert_eq!(
            resolve_store_path_from_env(None, None, Some("C:"), Some("   ")),
            PathBuf::from(".verity").join("providers.json")
        );
        assert_eq!(
            resolve_store_path_from_env(None, None, Some(" "), Some("\\Users\\alice")),
            PathBuf::from(".verity").join("providers.json")
        );
    }

    #[test]
    fn store_path_falls_back_when_no_home_env_is_set() {
        assert_eq!(
            resolve_store_path_from_env(None, None, None, None),
            PathBuf::from(".verity").join("providers.json")
        );
    }

    #[test]
    fn pi_extension_path_uses_verity_extensions_directory() {
        assert_eq!(
            resolve_pi_extension_path_from_env(Some("/Users/alice"), None, None, None),
            PathBuf::from("/Users/alice")
                .join(".verity")
                .join("extensions")
                .join(VERITY_PI_EXTENSION_FILENAME)
        );
        assert_eq!(
            resolve_pi_extension_path_from_env(None, Some("C:\\Users\\alice"), None, None),
            PathBuf::from("C:\\Users\\alice")
                .join(".verity")
                .join("extensions")
                .join(VERITY_PI_EXTENSION_FILENAME)
        );
    }

    #[test]
    fn ensure_verity_pi_extension_is_idempotent_and_repairs_stale_content() {
        let dir = unique_temp_dir("pi-ext");
        let ext_path = dir.join("extensions").join(VERITY_PI_EXTENSION_FILENAME);

        let first = ensure_verity_pi_extension_at(&ext_path).unwrap();
        assert_eq!(first, ext_path);
        assert_eq!(
            fs::read_to_string(&ext_path).unwrap(),
            VERITY_PI_EXTENSION_SOURCE
        );

        // Second call is an idempotent no-op when content matches.
        let second = ensure_verity_pi_extension_at(&ext_path).unwrap();
        assert_eq!(second, ext_path);

        // Stale or corrupted content is atomically reconciled.
        fs::write(&ext_path, "stale-content").unwrap();
        let repaired = ensure_verity_pi_extension_at(&ext_path).unwrap();
        assert_eq!(repaired, ext_path);
        assert_eq!(
            fs::read_to_string(&ext_path).unwrap(),
            VERITY_PI_EXTENSION_SOURCE
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
