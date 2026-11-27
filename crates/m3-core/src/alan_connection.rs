//! Canonical Alan Connection Store management for Project M3.
//!
//! Reads, writes, signs in, and tests connection profiles in Alan's canonical
//! System Store (`services/connections/connections.toml`) and Host Store
//! (`credentials/secrets.toml` & `auth.json`) so in-app setup is 100% compatible with
//! `alan` and [`alan_os_host::LlmSource::SharedProductCredentials`].

use std::path::Path;
use std::time::{Duration, Instant};

use alan_agent_engine::{Config, LlmProvider};
use alan_auth::browser::open_browser;
use alan_auth::google::{
    GoogleAuthError, GoogleAuthManager, gcloud_default_project, gcloud_is_available,
    resolve_adc_credentials_path, resolve_vertex_access_token, run_gcloud_adc_login_streaming,
};
use alan_auth::openrouter::{OpenRouterOAuthConfig, PendingOpenRouterBrowserLogin};
use alan_auth::{AuthStorage, BrowserLoginOptions, ChatgptAuthConfig, ChatgptAuthManager};
use alan_os_host::{HostStorePaths, SecretStore, SystemStorePaths, apply_profile_to_config};
use alan_service_manager::{
    ConnectionCredential, ConnectionProfile, ConnectionsFile, CredentialKind,
    default_credential_backend, normalize_profile_settings, sanitize_identifier,
    validate_profile_settings,
};
use anyhow::{Context, Result, bail, ensure};
use chrono::Utc;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(10);
const BROWSER_LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// All Alan first-party connection providers directly configurable through M3 Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlanConnectionProvider {
    #[default]
    OpenRouter,
    OpenAiResponses,
    Chatgpt,
    #[serde(alias = "google_gemini_generate_content")]
    GoogleVertex,
    Magpie,
}

impl AlanConnectionProvider {
    /// Ordered list of all first-party Alan providers supported in M3 Settings.
    pub const ALL: [Self; 5] = [
        Self::OpenRouter,
        Self::OpenAiResponses,
        Self::Chatgpt,
        Self::GoogleVertex,
        Self::Magpie,
    ];

    /// Canonical provider identifier stored in Alan's `connections.toml`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::OpenAiResponses => "openai_responses",
            Self::Chatgpt => "chatgpt",
            Self::GoogleVertex => "google_gemini_generate_content",
            Self::Magpie => "magpie",
        }
    }

    /// Human-readable provider label for UI controls.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::OpenRouter => "OpenRouter",
            Self::OpenAiResponses => "OpenAI",
            Self::Chatgpt => "ChatGPT",
            Self::GoogleVertex => "Vertex AI",
            Self::Magpie => "Magpie",
        }
    }

    /// Suggested default profile ID for this provider.
    pub const fn default_profile_id(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter-main",
            Self::OpenAiResponses => "openai-main",
            Self::Chatgpt => "chatgpt-main",
            Self::GoogleVertex => "vertex-main",
            Self::Magpie => "magpie-main",
        }
    }

    /// Whether this provider supports interactive browser OAuth/ADC login in Settings.
    pub const fn supports_browser_login(self) -> bool {
        matches!(self, Self::Chatgpt | Self::OpenRouter | Self::GoogleVertex)
    }

    /// Whether a SecretString API key is strictly mandatory when creating a profile.
    pub const fn requires_api_key(self) -> bool {
        matches!(self, Self::OpenRouter | Self::OpenAiResponses)
    }

    /// Whether this provider accepts a `base_url` setting in Alan's Connection schema.
    pub const fn uses_base_url(self) -> bool {
        !matches!(self, Self::GoogleVertex)
    }

    /// Canonical default region/location (`"global"` for Vertex AI, empty otherwise).
    pub const fn default_location(self) -> &'static str {
        if matches!(self, Self::GoogleVertex) {
            "global"
        } else {
            ""
        }
    }

    /// Canonical default model slug from Alan's provider catalog.
    pub fn default_model(self) -> &'static str {
        let descriptor = ConnectionsFile::profile_descriptor(self.to_llm_provider());
        descriptor
            .default_settings
            .iter()
            .find(|(key, _)| *key == "model")
            .map(|(_, value)| *value)
            .unwrap_or(match self {
                Self::OpenRouter => "moonshotai/kimi-k2.6",
                Self::OpenAiResponses => "gpt-5.4",
                Self::Chatgpt => "gpt-5.3-codex",
                Self::GoogleVertex => "gemini-2.5-flash",
                Self::Magpie => "default",
            })
    }

    /// Canonical default API base URL from Alan's provider catalog (`""` when unused).
    pub fn default_base_url(self) -> &'static str {
        if !self.uses_base_url() {
            return "";
        }
        let descriptor = ConnectionsFile::profile_descriptor(self.to_llm_provider());
        descriptor
            .default_settings
            .iter()
            .find(|(key, _)| *key == "base_url")
            .map(|(_, value)| *value)
            .unwrap_or(match self {
                Self::OpenRouter => "https://openrouter.ai/api/v1",
                Self::OpenAiResponses => "https://api.openai.com/v1",
                Self::Chatgpt => "https://chatgpt.com/backend-api/codex",
                Self::Magpie => "http://127.0.0.1:3425/v1",
                Self::GoogleVertex => "",
            })
    }

    /// Convert to Alan's [`LlmProvider`] enum.
    pub const fn to_llm_provider(self) -> LlmProvider {
        match self {
            Self::OpenRouter => LlmProvider::OpenRouter,
            Self::OpenAiResponses => LlmProvider::OpenAiResponses,
            Self::Chatgpt => LlmProvider::Chatgpt,
            Self::GoogleVertex => LlmProvider::GoogleGeminiGenerateContent,
            Self::Magpie => LlmProvider::Magpie,
        }
    }

    /// Convert from Alan's [`LlmProvider`] when supported by in-app setup.
    pub const fn from_llm_provider(provider: LlmProvider) -> Option<Self> {
        match provider {
            LlmProvider::OpenRouter => Some(Self::OpenRouter),
            LlmProvider::OpenAiResponses => Some(Self::OpenAiResponses),
            LlmProvider::Chatgpt => Some(Self::Chatgpt),
            LlmProvider::GoogleGeminiGenerateContent => Some(Self::GoogleVertex),
            LlmProvider::Magpie => Some(Self::Magpie),
            _ => None,
        }
    }

    /// Parse a provider identifier string or alias.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Ok(Self::OpenRouter),
            "openai_responses" | "openai" => Ok(Self::OpenAiResponses),
            "chatgpt" | "codex" => Ok(Self::Chatgpt),
            "google_gemini_generate_content" | "vertex" | "vertex_gemini" | "gemini" => {
                Ok(Self::GoogleVertex)
            }
            "magpie" => Ok(Self::Magpie),
            other => bail!("Unsupported Alan connection provider `{other}`"),
        }
    }
}

/// Non-secret summary of a single Alan connection profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlanConnectionProfileSummary {
    pub id: String,
    pub provider: String,
    pub model: Option<String>,
    pub base_url: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    pub has_secret: bool,
    pub masked_hint: Option<String>,
    pub is_default: bool,
}

/// Non-secret summary of all configured Alan connection profiles.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AlanConnectionStatus {
    pub default_profile: Option<String>,
    pub profiles: Vec<AlanConnectionProfileSummary>,
}

impl AlanConnectionStatus {
    /// Returns the currently selected default profile summary, if present.
    pub fn active_profile(&self) -> Option<&AlanConnectionProfileSummary> {
        let default_id = self.default_profile.as_deref()?;
        self.profiles.iter().find(|p| p.id == default_id)
    }
}

/// Request payload to create or update an Alan connection profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveAlanConnectionRequest {
    pub provider: AlanConnectionProvider,
    pub profile_id: String,
    /// Model slug; if blank, falls back to the provider's canonical default model.
    pub model: String,
    /// Optional custom base URL; when `None` or blank, resets to the provider's default URL.
    pub base_url: Option<String>,
    /// Optional API key; required for new OpenRouter/OpenAI profiles, optional for Magpie/updating existing secrets/Vertex Express.
    pub api_key: Option<String>,
    /// Optional Google Cloud Project ID for Vertex AI ADC/OAuth profiles.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Optional Google Cloud region/location for Vertex AI (`"global"` by default).
    #[serde(default)]
    pub location: Option<String>,
    pub set_as_default: bool,
}

/// Request payload to launch a browser OAuth/ADC login flow from M3 Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserLoginAlanConnectionRequest {
    pub provider: AlanConnectionProvider,
    pub profile_id: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    pub set_as_default: bool,
}

/// Result of a completed browser OAuth/ADC sign-in and profile persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlanConnectionBrowserLoginResult {
    pub profile: AlanConnectionProfileSummary,
    pub message: String,
}

/// Outcome of validating and probing an Alan connection profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlanConnectionTestResult {
    pub profile_id: String,
    pub provider: String,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub status_code: u16,
    pub latency_ms: u64,
    pub message: String,
}

/// Mask a secret into a short non-reversible hint for UI display.
///
/// Never returns the full secret; short values (<= 8 chars) collapse to `"••••"`.
pub fn mask_secret_hint(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 8 {
        return Some("••••".to_owned());
    }
    if chars.len() <= 12 {
        let suffix: String = chars[chars.len() - 4..].iter().collect();
        return Some(format!("••••{suffix}"));
    }
    let prefix: String = chars[..4].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    Some(format!("{prefix}••••{suffix}"))
}

/// Read non-secret connection status from Alan's canonical System & Host Stores.
pub fn list_alan_connection_status() -> Result<AlanConnectionStatus> {
    let system = SystemStorePaths::detect().context("detect Alan System Store paths")?;
    let host = HostStorePaths::detect().context("detect Alan Host Store paths")?;
    list_status_with_paths(&system, &host)
}

/// Read non-secret connection status from explicit System Store and Host Store roots (for tests).
pub fn list_alan_connection_status_in(
    system_root: &Path,
    host_root: &Path,
) -> Result<AlanConnectionStatus> {
    let system = SystemStorePaths::from_root(system_root)?;
    let host = HostStorePaths::from_root(host_root)?;
    list_status_with_paths(&system, &host)
}

/// Save or update an Alan connection profile in Alan's canonical System & Host Stores.
pub fn save_alan_connection(
    req: &SaveAlanConnectionRequest,
) -> Result<AlanConnectionProfileSummary> {
    let system = SystemStorePaths::detect().context("detect Alan System Store paths")?;
    let host = HostStorePaths::detect().context("detect Alan Host Store paths")?;
    save_with_paths(&system, &host, req)
}

/// Save or update an Alan connection profile inside explicit System Store and Host Store roots (for tests).
pub fn save_alan_connection_in(
    system_root: &Path,
    host_root: &Path,
    req: &SaveAlanConnectionRequest,
) -> Result<AlanConnectionProfileSummary> {
    let system = SystemStorePaths::from_root(system_root)?;
    let host = HostStorePaths::from_root(host_root)?;
    save_with_paths(&system, &host, req)
}

/// Run an interactive browser login flow (`Chatgpt`, `OpenRouter`, or `GoogleVertex`),
/// persist credentials in Alan's Host Store, and save the profile as default.
pub fn browser_login_alan_connection(
    req: &BrowserLoginAlanConnectionRequest,
) -> Result<AlanConnectionBrowserLoginResult> {
    let system = SystemStorePaths::detect().context("detect Alan System Store paths")?;
    let host = HostStorePaths::detect().context("detect Alan Host Store paths")?;
    browser_login_with_paths(&system, &host, req)
}

/// Validate and probe an Alan connection profile using Alan's canonical System & Host Stores.
pub fn test_alan_connection(profile_id: Option<&str>) -> Result<AlanConnectionTestResult> {
    let system = SystemStorePaths::detect().context("detect Alan System Store paths")?;
    let host = HostStorePaths::detect().context("detect Alan Host Store paths")?;
    test_with_paths(&system, &host, profile_id)
}

/// Validate and probe an Alan connection profile inside explicit System Store and Host Store roots (for tests).
pub fn test_alan_connection_in(
    system_root: &Path,
    host_root: &Path,
    profile_id: Option<&str>,
) -> Result<AlanConnectionTestResult> {
    let system = SystemStorePaths::from_root(system_root)?;
    let host = HostStorePaths::from_root(host_root)?;
    test_with_paths(&system, &host, profile_id)
}

fn non_empty_trimmed(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn build_current_thread_runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build async runtime for Alan connection operation")
}

fn run_on_current_thread_runtime<T, Fut>(fut: Fut) -> Result<T>
where
    T: Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        let handle = std::thread::Builder::new()
            .name("m3-alan-conn-rt".to_owned())
            .spawn(move || {
                let rt = build_current_thread_runtime()?;
                rt.block_on(fut)
            })
            .context("spawn dedicated thread for Alan connection async operation")?;
        match handle.join() {
            Ok(res) => res,
            Err(_) => bail!("Alan connection async worker thread panicked"),
        }
    } else {
        let rt = build_current_thread_runtime()?;
        rt.block_on(fut)
    }
}

fn list_status_with_paths(
    system: &SystemStorePaths,
    host: &HostStorePaths,
) -> Result<AlanConnectionStatus> {
    let metadata_path = system.connections_metadata()?;
    let (connections, _) = ConnectionsFile::load_from_path(&metadata_path)?;
    let secret_store = SecretStore::from_directory(&host.credentials)?;

    let profiles = connections
        .profiles
        .iter()
        .map(|(id, profile)| {
            build_profile_summary(
                id,
                profile,
                connections.default_profile.as_deref() == Some(id.as_str()),
                &secret_store,
                host,
            )
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(AlanConnectionStatus {
        default_profile: connections.default_profile,
        profiles,
    })
}

fn build_profile_summary(
    id: &str,
    profile: &ConnectionProfile,
    is_default: bool,
    secret_store: &SecretStore,
    host: &HostStorePaths,
) -> Result<AlanConnectionProfileSummary> {
    let normalized = normalize_profile_settings(profile.provider, &profile.settings);
    let model = normalized
        .get("model")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let base_url = normalized
        .get("base_url")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let project_id = normalized
        .get("project_id")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let location = normalized
        .get("location")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    let auth_store = AuthStorage::new(host.managed_auth.clone())
        .ok()
        .and_then(|storage| storage.load().ok());

    let (has_secret, masked_hint) = match profile.provider {
        LlmProvider::Chatgpt => {
            if let Some(chatgpt) = auth_store.as_ref().and_then(|st| st.chatgpt.as_ref()) {
                let hint = chatgpt
                    .email
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .unwrap_or("OAuth")
                    .to_owned();
                (true, Some(hint))
            } else {
                (false, None)
            }
        }
        LlmProvider::GoogleGeminiGenerateContent => {
            if let Some(cred_id) = profile.credential_id.as_deref() {
                match secret_store.load(cred_id)? {
                    Some(secret) => match mask_secret_hint(&secret) {
                        Some(hint) => (true, Some(hint)),
                        None => (false, None),
                    },
                    None => (false, None),
                }
            } else {
                let managed_google = auth_store.as_ref().and_then(|st| st.google.as_ref());
                let adc_ready =
                    managed_google.is_some() || resolve_adc_credentials_path().is_some();
                let hint = match (
                    managed_google
                        .and_then(|g| g.email.as_deref())
                        .map(str::trim)
                        .filter(|v| !v.is_empty()),
                    project_id.as_deref(),
                ) {
                    (Some(email), Some(pid)) => Some(format!("{email} · {pid}")),
                    (Some(email), None) => Some(email.to_owned()),
                    (None, Some(pid)) => Some(format!("ADC · {pid}")),
                    (None, None) => Some("ADC".to_owned()),
                };
                (adc_ready && project_id.is_some(), hint)
            }
        }
        LlmProvider::Magpie => {
            if let Some(cred_id) = profile.credential_id.as_deref() {
                match secret_store.load(cred_id)? {
                    Some(secret) => match mask_secret_hint(&secret) {
                        Some(hint) => (true, Some(hint)),
                        None => (true, Some("Local".to_owned())),
                    },
                    None => (true, Some("Local".to_owned())),
                }
            } else {
                (true, Some("Local".to_owned()))
            }
        }
        _ => {
            if let Some(cred_id) = profile.credential_id.as_deref() {
                match secret_store.load(cred_id)? {
                    Some(secret) => match mask_secret_hint(&secret) {
                        Some(hint) => (true, Some(hint)),
                        None => (false, None),
                    },
                    None => (false, None),
                }
            } else {
                (false, None)
            }
        }
    };

    Ok(AlanConnectionProfileSummary {
        id: id.to_owned(),
        provider: profile.provider.as_str().to_owned(),
        model,
        base_url,
        project_id,
        location,
        has_secret,
        masked_hint,
        is_default,
    })
}

fn save_with_paths(
    system: &SystemStorePaths,
    host: &HostStorePaths,
    req: &SaveAlanConnectionRequest,
) -> Result<AlanConnectionProfileSummary> {
    let raw_profile_id = if req.profile_id.trim().is_empty() {
        req.provider.default_profile_id()
    } else {
        req.profile_id.as_str()
    };
    let profile_id = sanitize_identifier(raw_profile_id).ok_or_else(|| {
        anyhow::anyhow!(
            "Profile ID `{}` must use only ASCII letters, digits, '-' or '_'",
            req.profile_id.trim()
        )
    })?;
    let llm_provider = req.provider.to_llm_provider();
    let descriptor = ConnectionsFile::profile_descriptor(llm_provider);

    let metadata_path = system.connections_metadata()?;
    let (mut connections, _) = ConnectionsFile::load_from_path(&metadata_path)?;
    let secret_store = SecretStore::from_directory(&host.credentials)?;

    let existing_profile = connections.profiles.get(&profile_id).cloned();
    let base_cred_id = existing_profile
        .as_ref()
        .and_then(|profile| profile.credential_id.as_deref())
        .and_then(sanitize_identifier)
        .unwrap_or_else(|| profile_id.clone());

    let incoming_secret = non_empty_trimmed(req.api_key.as_deref());
    let same_provider_existing_secret = if existing_profile
        .as_ref()
        .is_some_and(|p| p.provider == llm_provider)
    {
        secret_store
            .load(&base_cred_id)?
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    } else {
        None
    };
    let effective_secret = incoming_secret.clone().or(same_provider_existing_secret);

    let mut raw_settings = existing_profile
        .as_ref()
        .filter(|profile| profile.provider == llm_provider)
        .map(|profile| profile.settings.clone())
        .unwrap_or_default();

    let model = non_empty_trimmed(Some(req.model.as_str()))
        .unwrap_or_else(|| req.provider.default_model().to_owned());
    raw_settings.insert("model".to_owned(), model);

    if req.provider.uses_base_url() {
        match non_empty_trimmed(req.base_url.as_deref()) {
            Some(raw_url) => {
                let parsed = reqwest::Url::parse(&raw_url)
                    .with_context(|| format!("Invalid base URL `{raw_url}`"))?;
                ensure!(
                    matches!(parsed.scheme(), "http" | "https"),
                    "Base URL `{raw_url}` must start with http:// or https://"
                );
                raw_settings.insert(
                    "base_url".to_owned(),
                    raw_url.trim_end_matches('/').to_owned(),
                );
            }
            None => {
                raw_settings.remove("base_url");
            }
        }
    } else {
        raw_settings.remove("base_url");
    }

    if llm_provider == LlmProvider::GoogleGeminiGenerateContent {
        let loc = non_empty_trimmed(req.location.as_deref()).unwrap_or_else(|| "global".to_owned());
        raw_settings.insert("location".to_owned(), loc);
        match non_empty_trimmed(req.project_id.as_deref()) {
            Some(pid) => {
                raw_settings.insert("project_id".to_owned(), pid);
            }
            None => {
                raw_settings.remove("project_id");
            }
        }
    }

    let settings = normalize_profile_settings(llm_provider, &raw_settings);
    validate_profile_settings(llm_provider, &settings)?;

    let (credential_id, secret_to_write): (Option<String>, Option<String>) = match req.provider {
        AlanConnectionProvider::OpenRouter | AlanConnectionProvider::OpenAiResponses => {
            let secret = effective_secret.ok_or_else(|| {
                anyhow::anyhow!("API key is required for new connection profile `{profile_id}`")
            })?;
            connections.credentials.insert(
                base_cred_id.clone(),
                ConnectionCredential {
                    kind: CredentialKind::SecretString,
                    provider_family: llm_provider,
                    label: format!("{} credential", descriptor.display_name),
                    backend: default_credential_backend(CredentialKind::SecretString).to_owned(),
                },
            );
            (Some(base_cred_id), Some(secret))
        }
        AlanConnectionProvider::Chatgpt => {
            connections.credentials.insert(
                base_cred_id.clone(),
                ConnectionCredential {
                    kind: CredentialKind::ManagedOauth,
                    provider_family: llm_provider,
                    label: format!("{} credential", descriptor.display_name),
                    backend: default_credential_backend(CredentialKind::ManagedOauth).to_owned(),
                },
            );
            (Some(base_cred_id), None)
        }
        AlanConnectionProvider::GoogleVertex => {
            if let Some(secret) = effective_secret {
                connections.credentials.insert(
                    base_cred_id.clone(),
                    ConnectionCredential {
                        kind: CredentialKind::SecretString,
                        provider_family: llm_provider,
                        label: format!("{} credential", descriptor.display_name),
                        backend: default_credential_backend(CredentialKind::SecretString)
                            .to_owned(),
                    },
                );
                (Some(base_cred_id), Some(secret))
            } else {
                let has_project = settings
                    .get("project_id")
                    .is_some_and(|pid| !pid.trim().is_empty());
                ensure!(
                    has_project,
                    "Vertex AI requires a GCP Project ID (or a Vertex Express API key) for `{profile_id}`"
                );
                (None, None)
            }
        }
        AlanConnectionProvider::Magpie => {
            if let Some(secret) = effective_secret {
                connections.credentials.insert(
                    base_cred_id.clone(),
                    ConnectionCredential {
                        kind: CredentialKind::SecretString,
                        provider_family: llm_provider,
                        label: format!("{} credential", descriptor.display_name),
                        backend: default_credential_backend(CredentialKind::SecretString)
                            .to_owned(),
                    },
                );
                (Some(base_cred_id), Some(secret))
            } else {
                (None, None)
            }
        }
    };

    let now = Utc::now();
    let created_at = existing_profile
        .as_ref()
        .map(|profile| profile.created_at)
        .unwrap_or(now);
    let label = existing_profile
        .as_ref()
        .and_then(|profile| profile.label.clone());
    let source = existing_profile
        .as_ref()
        .map(|profile| profile.source.clone())
        .unwrap_or_else(|| "managed".to_owned());

    let updated_profile = ConnectionProfile {
        provider: llm_provider,
        label,
        credential_id: credential_id.clone(),
        created_at,
        updated_at: now,
        source,
        settings,
    };
    connections
        .profiles
        .insert(profile_id.clone(), updated_profile);

    if req.set_as_default || connections.default_profile.is_none() {
        connections.default_profile = Some(profile_id.clone());
    }

    // Validate full Alan provider projection before writing secrets or metadata to disk.
    let mut validation_config = Config::default();
    connections.apply_profile_metadata_to_config(Some(&profile_id), &mut validation_config)?;
    if let Some(secret) = secret_to_write.as_ref() {
        match llm_provider {
            LlmProvider::OpenRouter => {
                validation_config.openrouter_api_key = Some(secret.clone());
            }
            LlmProvider::OpenAiResponses => {
                validation_config.openai_responses_api_key = Some(secret.clone());
            }
            LlmProvider::GoogleGeminiGenerateContent => {
                validation_config.google_gemini_generate_content_api_key = Some(secret.clone());
            }
            LlmProvider::Magpie => {
                validation_config.magpie_api_key = Some(secret.clone());
            }
            _ => {}
        }
    }
    let _ = validation_config.to_provider_config()?;

    if let (Some(cred_id), Some(secret_to_save)) =
        (credential_id.as_deref(), incoming_secret.as_deref())
    {
        secret_store.save(cred_id, secret_to_save)?;
    }
    connections.save_to_path(&metadata_path)?;

    let saved_profile = connections
        .profiles
        .get(&profile_id)
        .ok_or_else(|| anyhow::anyhow!("Saved profile `{profile_id}` missing after save"))?;
    build_profile_summary(
        &profile_id,
        saved_profile,
        connections.default_profile.as_deref() == Some(profile_id.as_str()),
        &secret_store,
        host,
    )
}

fn browser_login_with_paths(
    system: &SystemStorePaths,
    host: &HostStorePaths,
    req: &BrowserLoginAlanConnectionRequest,
) -> Result<AlanConnectionBrowserLoginResult> {
    let rt = build_current_thread_runtime()?;
    match req.provider {
        AlanConnectionProvider::Chatgpt => {
            let manager = ChatgptAuthManager::new(ChatgptAuthConfig::with_storage_path(
                host.managed_auth.clone(),
            ))?;
            let success = rt.block_on(async {
                let pending = manager.begin_browser_login(BrowserLoginOptions::default())?;
                open_browser(&pending.auth_url)
                    .with_context(|| format!("open browser to {}", pending.auth_url))?;
                let mut receipt = manager.wait_for_browser_callback(&pending).await?;
                let result = manager
                    .complete_browser_login(&pending, receipt.completion.clone())
                    .await;
                let _ = manager
                    .write_browser_login_result(&mut receipt.stream, result.as_ref())
                    .await;
                result.map_err(anyhow::Error::from)
            })?;
            let saved = save_with_paths(
                system,
                host,
                &SaveAlanConnectionRequest {
                    provider: AlanConnectionProvider::Chatgpt,
                    profile_id: req.profile_id.clone(),
                    model: req.model.clone(),
                    base_url: req.base_url.clone(),
                    api_key: None,
                    project_id: None,
                    location: None,
                    set_as_default: req.set_as_default,
                },
            )?;
            let message = match success.email {
                Some(email) => format!("ChatGPT OAuth signed in ({email})"),
                None => "ChatGPT OAuth signed in".to_owned(),
            };
            Ok(AlanConnectionBrowserLoginResult {
                profile: saved,
                message,
            })
        }
        AlanConnectionProvider::OpenRouter => {
            let success = rt.block_on(async {
                let pending =
                    PendingOpenRouterBrowserLogin::start(OpenRouterOAuthConfig::default()).await?;
                if let Err(err) = pending.open_browser() {
                    bail!("open browser for OpenRouter OAuth: {err}");
                }
                pending
                    .wait_and_exchange(BROWSER_LOGIN_TIMEOUT)
                    .await
                    .map_err(anyhow::Error::from)
            })?;
            let saved = save_with_paths(
                system,
                host,
                &SaveAlanConnectionRequest {
                    provider: AlanConnectionProvider::OpenRouter,
                    profile_id: req.profile_id.clone(),
                    model: req.model.clone(),
                    base_url: req.base_url.clone(),
                    api_key: Some(success.api_key),
                    project_id: None,
                    location: None,
                    set_as_default: req.set_as_default,
                },
            )?;
            Ok(AlanConnectionBrowserLoginResult {
                profile: saved,
                message: "OpenRouter connected via PKCE OAuth".to_owned(),
            })
        }
        AlanConnectionProvider::GoogleVertex => {
            let managed_auth = host.managed_auth.clone();
            let (resolved, detected_default_proj) = rt.block_on(async {
                let token = match resolve_vertex_access_token(Some(&managed_auth)).await {
                    Ok(token) => token,
                    Err(_) => {
                        let manager = GoogleAuthManager::with_storage_path(managed_auth.clone())?;
                        match manager.start_browser_login().await {
                            Ok(pending) => {
                                if let Err(err) = pending.open_browser() {
                                    bail!("open browser for Google Cloud login: {err}");
                                }
                                pending.wait_and_exchange(BROWSER_LOGIN_TIMEOUT).await?;
                                resolve_vertex_access_token(Some(&managed_auth)).await?
                            }
                            Err(GoogleAuthError::OAuthClientNotConfigured) => {
                                if gcloud_is_available() {
                                    run_gcloud_adc_login_streaming(|url| {
                                        let _ = open_browser(&url);
                                    })
                                    .await?;
                                    resolve_vertex_access_token(Some(&managed_auth)).await?
                                } else {
                                    bail!(
                                        "No `gcloud` CLI or Google Desktop OAuth client found; paste a Vertex Express API key or install Google Cloud SDK"
                                    );
                                }
                            }
                            Err(err) => return Err(err.into()),
                        }
                    }
                };
                let default_proj = gcloud_default_project().await;
                Ok::<_, anyhow::Error>((token, default_proj))
            })?;

            let project_id = non_empty_trimmed(req.project_id.as_deref())
                .or(detected_default_proj)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Google Cloud signed in ({}), now enter your GCP Project ID and save",
                        resolved.source.label()
                    )
                })?;

            let saved = save_with_paths(
                system,
                host,
                &SaveAlanConnectionRequest {
                    provider: AlanConnectionProvider::GoogleVertex,
                    profile_id: req.profile_id.clone(),
                    model: req.model.clone(),
                    base_url: None,
                    api_key: None,
                    project_id: Some(project_id.clone()),
                    location: req.location.clone(),
                    set_as_default: req.set_as_default,
                },
            )?;
            Ok(AlanConnectionBrowserLoginResult {
                profile: saved,
                message: format!(
                    "Vertex AI connected (project={project_id}, {})",
                    resolved.source.label()
                ),
            })
        }
        other => bail!(
            "Provider `{}` does not support browser OAuth login",
            other.display_name()
        ),
    }
}

fn test_with_paths(
    system: &SystemStorePaths,
    host: &HostStorePaths,
    profile_id: Option<&str>,
) -> Result<AlanConnectionTestResult> {
    let metadata_path = system.connections_metadata()?;
    let (connections, _) = ConnectionsFile::load_from_path(&metadata_path)?;

    let target_id = profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| connections.default_profile.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No Alan connection profile is configured or selected. Save a profile first."
            )
        })?;

    let secret_store = SecretStore::from_directory(&host.credentials)?;
    let mut config = Config::default();
    let resolved =
        apply_profile_to_config(&connections, Some(&target_id), &secret_store, &mut config)?;
    let _ = config.to_provider_config()?;

    let model = resolved
        .settings
        .get("model")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let base_url = resolved
        .settings
        .get("base_url")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    if resolved.provider == LlmProvider::TypesafeEvaluation {
        return Ok(AlanConnectionTestResult {
            profile_id: target_id,
            provider: resolved.provider.as_str().to_owned(),
            model,
            base_url,
            status_code: 200,
            latency_ms: 0,
            message: "Validated local TypeSafe evaluation profile".to_owned(),
        });
    }

    let target_id_owned = target_id.clone();
    let provider = resolved.provider;
    let config_owned = config.clone();
    let managed_auth = host.managed_auth.clone();

    let (status, latency_ms) = run_on_current_thread_runtime(async move {
        let client = reqwest::Client::builder()
            .timeout(CONNECTION_TEST_TIMEOUT)
            .build()
            .context("build HTTP client for Alan connection test")?;

        let req = build_probe_request(
            &client,
            &target_id_owned,
            provider,
            &config_owned,
            &managed_auth,
        )
        .await?;

        let started = Instant::now();
        let response = req
            .send()
            .await
            .with_context(|| format!("connection test failed for profile `{target_id_owned}`"))?;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok::<_, anyhow::Error>((response.status(), elapsed_ms))
    })?;

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        bail!("Authentication failed for profile `{target_id}` (HTTP {status})");
    }
    if !status.is_success() {
        bail!("Connection test failed for profile `{target_id}` (HTTP {status})");
    }

    Ok(AlanConnectionTestResult {
        profile_id: target_id,
        provider: resolved.provider.as_str().to_owned(),
        model,
        base_url,
        status_code: status.as_u16(),
        latency_ms,
        message: format!("HTTP {} OK ({latency_ms}ms)", status.as_u16()),
    })
}

fn vertex_catalog_probe_url(location: &str) -> String {
    let loc = location.trim();
    let host = if loc.is_empty() || loc.eq_ignore_ascii_case("global") {
        "us-central1-aiplatform.googleapis.com".to_owned()
    } else {
        format!("{loc}-aiplatform.googleapis.com")
    };
    format!("https://{host}/v1beta1/publishers/google/models?pageSize=1")
}

fn vertex_express_probe_url(model: &str) -> String {
    let trimmed = model.trim();
    let slug = trimmed
        .strip_prefix("publishers/google/models/")
        .or_else(|| trimmed.strip_prefix("models/"))
        .unwrap_or(trimmed);
    let effective = if slug.is_empty() {
        "gemini-2.5-flash"
    } else {
        slug
    };
    format!("https://aiplatform.googleapis.com/v1/publishers/google/models/{effective}:countTokens")
}

async fn build_probe_request(
    client: &reqwest::Client,
    profile_id: &str,
    provider: LlmProvider,
    config: &Config,
    managed_auth_path: &Path,
) -> Result<reqwest::RequestBuilder> {
    fn non_empty(value: &Option<String>) -> Option<&str> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    fn require_base<'a>(profile_id: &str, raw: &'a str) -> Result<&'a str> {
        let trimmed = raw.trim();
        ensure!(
            !trimmed.is_empty(),
            "Profile `{profile_id}` is missing a base URL"
        );
        Ok(trimmed)
    }

    match provider {
        LlmProvider::Chatgpt => {
            let manager = ChatgptAuthManager::new(ChatgptAuthConfig::with_storage_path(
                managed_auth_path.to_path_buf(),
            ))?;
            let auth = manager
                .request_auth_for_account(config.chatgpt_account_id.as_deref())
                .await
                .map_err(|err| {
                    anyhow::anyhow!("ChatGPT profile `{profile_id}` is not signed in: {err}")
                })?;
            let base = require_base(profile_id, &config.chatgpt_base_url)?;
            let mut url = reqwest::Url::parse(&format!("{}/models", base.trim_end_matches('/')))
                .with_context(|| format!("invalid ChatGPT base URL for `{profile_id}`"))?;
            url.query_pairs_mut()
                .append_pair("client_version", "0.159.0");
            Ok(client
                .get(url)
                .bearer_auth(&auth.access_token)
                .header("chatgpt-account-id", &auth.account_id))
        }
        LlmProvider::Magpie
        | LlmProvider::OpenAiResponses
        | LlmProvider::OpenAiChatCompletions
        | LlmProvider::OpenAiChatCompletionsCompatible
        | LlmProvider::OpenRouter => {
            let (raw_base, key) = match provider {
                LlmProvider::Magpie => (&config.magpie_base_url, non_empty(&config.magpie_api_key)),
                LlmProvider::OpenAiResponses => (
                    &config.openai_responses_base_url,
                    non_empty(&config.openai_responses_api_key),
                ),
                LlmProvider::OpenAiChatCompletions => (
                    &config.openai_chat_completions_base_url,
                    non_empty(&config.openai_chat_completions_api_key),
                ),
                LlmProvider::OpenAiChatCompletionsCompatible => (
                    &config.openai_chat_completions_compatible_base_url,
                    non_empty(&config.openai_chat_completions_compatible_api_key),
                ),
                LlmProvider::OpenRouter => (
                    &config.openrouter_base_url,
                    non_empty(&config.openrouter_api_key),
                ),
                _ => unreachable!(),
            };
            let base = require_base(profile_id, raw_base)?;
            let url = format!("{}/models", base.trim_end_matches('/'));
            let mut req = client.get(&url);
            if let Some(secret) = key {
                req = req.bearer_auth(secret);
            } else if !matches!(
                provider,
                LlmProvider::Magpie | LlmProvider::OpenAiChatCompletionsCompatible
            ) {
                bail!("Profile `{profile_id}` has no API key configured");
            }
            if provider == LlmProvider::OpenRouter {
                if let Some(referer) = non_empty(&config.openrouter_http_referer) {
                    req = req.header("HTTP-Referer", referer);
                }
                if let Some(title) = non_empty(&config.openrouter_x_title) {
                    req = req.header("X-Title", title);
                }
            }
            Ok(req)
        }
        LlmProvider::AnthropicMessages => {
            let base = require_base(profile_id, &config.anthropic_messages_base_url)?;
            let trimmed = base.trim_end_matches('/');
            let url = if trimmed.ends_with("/v1") {
                format!("{trimmed}/models")
            } else {
                format!("{trimmed}/v1/models")
            };
            let key = non_empty(&config.anthropic_messages_api_key).ok_or_else(|| {
                anyhow::anyhow!("Profile `{profile_id}` has no API key configured")
            })?;
            let mut req = client
                .get(&url)
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01");
            if let Some(ua) = non_empty(&config.anthropic_messages_user_agent) {
                req = req.header("User-Agent", ua);
            }
            Ok(req)
        }
        LlmProvider::GoogleGeminiGenerateContent => {
            if let Some(key) = non_empty(&config.google_gemini_generate_content_api_key) {
                let url = vertex_express_probe_url(&config.google_gemini_generate_content_model);
                Ok(client
                    .post(&url)
                    .header("x-goog-api-key", key)
                    .json(&serde_json::json!({
                        "contents": [{
                            "role": "user",
                            "parts": [{ "text": "ping" }]
                        }]
                    })))
            } else {
                let resolved = resolve_vertex_access_token(Some(managed_auth_path))
                    .await
                    .map_err(|err| {
                        anyhow::anyhow!(
                            "Google Vertex credentials unavailable for profile `{profile_id}`: {err}"
                        )
                    })?;
                let url = vertex_catalog_probe_url(&config.google_gemini_generate_content_location);
                let mut req = client.get(&url).bearer_auth(&resolved.access_token);
                if let Some(project_id) =
                    non_empty(&config.google_gemini_generate_content_project_id)
                {
                    req = req.header("x-goog-user-project", project_id);
                }
                Ok(req)
            }
        }
        LlmProvider::TypesafeEvaluation => {
            bail!("Provider `typesafe` does not use HTTP probe")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    struct TempStores {
        root: PathBuf,
        system: PathBuf,
        host: PathBuf,
    }

    impl TempStores {
        fn new(label: &str) -> Self {
            let seq = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir()
                .join(format!("m3-alan-conn-{label}-{}-{seq}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let system = root.join("System Store");
            let host = root.join("Host Store");
            std::fs::create_dir_all(&system).unwrap();
            std::fs::create_dir_all(&host).unwrap();
            Self { root, system, host }
        }
    }

    impl Drop for TempStores {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn spawn_single_response_server(
        status_line: &str,
        body: &str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let status_owned = status_line.to_owned();
        let body_owned = body.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let resp = format!(
                "HTTP/1.1 {status_owned}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_owned}",
                body_owned.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
            req
        });
        (format!("http://{addr}/v1"), handle)
    }

    #[test]
    fn mask_secret_hint_never_exposes_raw_secret() {
        assert_eq!(mask_secret_hint(""), None);
        assert_eq!(mask_secret_hint("   "), None);
        assert_eq!(mask_secret_hint("sk-123").as_deref(), Some("••••"));
        assert_eq!(mask_secret_hint("12345678").as_deref(), Some("••••"));
        assert_eq!(mask_secret_hint("1234567890").as_deref(), Some("••••7890"));
        let long = "sk-or-v1-abcdef1234567890wxyz";
        let masked = mask_secret_hint(long).unwrap();
        assert_eq!(masked, "sk-o••••wxyz");
        assert!(!masked.contains("abcdef"));
        assert!(!masked.contains(long));
    }

    #[test]
    fn empty_store_returns_unconfigured_status() {
        let stores = TempStores::new("empty");
        let status = list_alan_connection_status_in(&stores.system, &stores.host).unwrap();
        assert_eq!(status.default_profile, None);
        assert!(status.profiles.is_empty());
        assert!(status.active_profile().is_none());
    }

    #[test]
    fn save_openrouter_and_openai_profiles_are_canonical_and_mask_secrets() {
        let stores = TempStores::new("save-canonical");

        let saved_or = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenRouter,
                profile_id: "openrouter-main".to_owned(),
                model: "anthropic/claude-sonnet-4.6".to_owned(),
                base_url: None,
                api_key: Some("sk-or-v1-secret-value-9876".to_owned()),
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap();

        assert_eq!(saved_or.id, "openrouter-main");
        assert_eq!(saved_or.provider, "openrouter");
        assert_eq!(
            saved_or.model.as_deref(),
            Some("anthropic/claude-sonnet-4.6")
        );
        assert_eq!(
            saved_or.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert!(saved_or.has_secret);
        assert_eq!(saved_or.masked_hint.as_deref(), Some("sk-o••••9876"));
        assert!(saved_or.is_default);

        // Save second profile without overwriting default.
        let saved_oai = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenAiResponses,
                profile_id: "openai-main".to_owned(),
                model: "gpt-5.4".to_owned(),
                base_url: Some("https://custom.openai.example/v1/".to_owned()),
                api_key: Some("sk-proj-openai-secret-1234".to_owned()),
                project_id: None,
                location: None,
                set_as_default: false,
            },
        )
        .unwrap();
        assert!(!saved_oai.is_default);
        assert_eq!(
            saved_oai.base_url.as_deref(),
            Some("https://custom.openai.example/v1")
        );

        let status = list_alan_connection_status_in(&stores.system, &stores.host).unwrap();
        assert_eq!(status.default_profile.as_deref(), Some("openrouter-main"));
        assert_eq!(status.profiles.len(), 2);
        let active = status.active_profile().unwrap();
        assert_eq!(active.id, "openrouter-main");

        // Ensure raw secrets never leak into summary serialization.
        let serialized_status = serde_json::to_string(&status).unwrap();
        assert!(!serialized_status.contains("sk-or-v1-secret-value-9876"));
        assert!(!serialized_status.contains("sk-proj-openai-secret-1234"));

        // Verify 100% compatibility with Alan's own ConnectionsFile + apply_profile_to_config.
        let system_paths = SystemStorePaths::from_root(&stores.system).unwrap();
        let host_paths = HostStorePaths::from_root(&stores.host).unwrap();
        let (connections, _) =
            ConnectionsFile::load_from_path(&system_paths.connections_metadata().unwrap()).unwrap();
        let secret_store = SecretStore::from_directory(&host_paths.credentials).unwrap();

        let mut or_cfg = Config::default();
        apply_profile_to_config(&connections, None, &secret_store, &mut or_cfg).unwrap();
        assert_eq!(or_cfg.llm_provider, LlmProvider::OpenRouter);
        assert_eq!(
            or_cfg.openrouter_api_key.as_deref(),
            Some("sk-or-v1-secret-value-9876")
        );
        assert_eq!(or_cfg.openrouter_model, "anthropic/claude-sonnet-4.6");
        assert!(or_cfg.to_provider_config().is_ok());

        let mut oai_cfg = Config::default();
        apply_profile_to_config(
            &connections,
            Some("openai-main"),
            &secret_store,
            &mut oai_cfg,
        )
        .unwrap();
        assert_eq!(oai_cfg.llm_provider, LlmProvider::OpenAiResponses);
        assert_eq!(
            oai_cfg.openai_responses_api_key.as_deref(),
            Some("sk-proj-openai-secret-1234")
        );
        assert_eq!(
            oai_cfg.openai_responses_base_url,
            "https://custom.openai.example/v1"
        );
        assert_eq!(oai_cfg.openai_responses_model, "gpt-5.4");
        assert!(oai_cfg.to_provider_config().is_ok());
    }

    #[test]
    fn save_chatgpt_vertex_and_magpie_profiles_match_alan_schema() {
        let stores = TempStores::new("all-first-party");

        // 1. ChatGPT OAuth profile (uses ManagedOauth in Host auth.json)
        let saved_chatgpt = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::Chatgpt,
                profile_id: "chatgpt-main".to_owned(),
                model: "gpt-5.3-codex".to_owned(),
                base_url: None,
                api_key: None,
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap();
        assert_eq!(saved_chatgpt.provider, "chatgpt");
        assert!(!saved_chatgpt.has_secret);
        assert_eq!(saved_chatgpt.masked_hint, None);

        // 2. Vertex AI ADC profile (`project_id` + `location`, no API key required)
        let saved_vertex_adc = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::GoogleVertex,
                profile_id: "vertex-adc".to_owned(),
                model: "gemini-2.5-flash".to_owned(),
                base_url: None,
                api_key: None,
                project_id: Some("m3-research-prod".to_owned()),
                location: Some("us-central1".to_owned()),
                set_as_default: false,
            },
        )
        .unwrap();
        assert_eq!(saved_vertex_adc.provider, "google_gemini_generate_content");
        assert_eq!(
            saved_vertex_adc.project_id.as_deref(),
            Some("m3-research-prod")
        );
        assert_eq!(saved_vertex_adc.location.as_deref(), Some("us-central1"));

        // 3. Vertex AI Express API Key profile (optional project_id)
        let saved_vertex_express = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::GoogleVertex,
                profile_id: "vertex-express".to_owned(),
                model: "gemini-2.5-pro".to_owned(),
                base_url: None,
                api_key: Some("AIzaSyDummyExpressKey9999".to_owned()),
                project_id: None,
                location: None,
                set_as_default: false,
            },
        )
        .unwrap();
        assert!(saved_vertex_express.has_secret);
        assert_eq!(
            saved_vertex_express.masked_hint.as_deref(),
            Some("AIza••••9999")
        );
        assert_eq!(saved_vertex_express.location.as_deref(), Some("global"));

        // 4. Magpie local router without API key (optional secret)
        let (magpie_base, magpie_handle) = spawn_single_response_server("200 OK", r#"{"data":[]}"#);
        let saved_magpie = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::Magpie,
                profile_id: "magpie-main".to_owned(),
                model: "default".to_owned(),
                base_url: Some(magpie_base),
                api_key: None,
                project_id: None,
                location: None,
                set_as_default: false,
            },
        )
        .unwrap();
        assert_eq!(saved_magpie.provider, "magpie");
        assert!(saved_magpie.has_secret);
        assert_eq!(saved_magpie.masked_hint.as_deref(), Some("Local"));

        let magpie_test =
            test_alan_connection_in(&stores.system, &stores.host, Some("magpie-main")).unwrap();
        let req = magpie_handle.join().unwrap();
        assert_eq!(magpie_test.status_code, 200);
        assert!(req.starts_with("GET /v1/models "));
    }

    #[test]
    fn updating_existing_profile_without_api_key_preserves_secret() {
        let stores = TempStores::new("preserve-secret");

        save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenRouter,
                profile_id: "main".to_owned(),
                model: "moonshotai/kimi-k2.6".to_owned(),
                base_url: None,
                api_key: Some("sk-or-v1-keep-me-4321".to_owned()),
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap();

        let updated = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenRouter,
                profile_id: "main".to_owned(),
                model: "openai/gpt-5.4".to_owned(),
                base_url: None,
                api_key: Some("   ".to_owned()),
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap();

        assert!(updated.has_secret);
        assert_eq!(updated.masked_hint.as_deref(), Some("sk-o••••4321"));
        assert_eq!(updated.model.as_deref(), Some("openai/gpt-5.4"));

        let host_paths = HostStorePaths::from_root(&stores.host).unwrap();
        let secret_store = SecretStore::from_directory(&host_paths.credentials).unwrap();
        assert_eq!(
            secret_store.load("main").unwrap().as_deref(),
            Some("sk-or-v1-keep-me-4321")
        );
    }

    #[test]
    fn save_rejects_invalid_inputs_without_corrupting_store() {
        let stores = TempStores::new("validation");

        // Invalid profile ID
        let err = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenRouter,
                profile_id: "../bad id".to_owned(),
                model: "moonshotai/kimi-k2.6".to_owned(),
                base_url: None,
                api_key: Some("sk-test-123456789".to_owned()),
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("Profile ID"));

        // Missing API key on new profile
        let err = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenRouter,
                profile_id: "main".to_owned(),
                model: "moonshotai/kimi-k2.6".to_owned(),
                base_url: None,
                api_key: None,
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("API key is required"));

        // Vertex AI missing both API key and project_id
        let err = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::GoogleVertex,
                profile_id: "vertex-empty".to_owned(),
                model: "gemini-2.5-flash".to_owned(),
                base_url: None,
                api_key: None,
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("GCP Project ID"));

        // Invalid base URL
        let err = save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenRouter,
                profile_id: "main".to_owned(),
                model: "moonshotai/kimi-k2.6".to_owned(),
                base_url: Some("ftp://invalid.example".to_owned()),
                api_key: Some("sk-test-123456789".to_owned()),
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("http:// or https://"));

        // Store must remain completely empty after rejected saves
        let status = list_alan_connection_status_in(&stores.system, &stores.host).unwrap();
        assert!(status.profiles.is_empty());
    }

    #[test]
    fn test_alan_connection_handles_200_and_401_and_missing_profile() {
        let stores = TempStores::new("probe");

        let missing_err = test_alan_connection_in(&stores.system, &stores.host, None).unwrap_err();
        assert!(
            missing_err
                .to_string()
                .contains("No Alan connection profile")
        );

        let (base_ok, handle_ok) = spawn_single_response_server("200 OK", r#"{"data":[]}"#);
        save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenRouter,
                profile_id: "ok-prof".to_owned(),
                model: "moonshotai/kimi-k2.6".to_owned(),
                base_url: Some(base_ok.clone()),
                api_key: Some("sk-or-v1-probe-key-1111".to_owned()),
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap();

        let res = test_alan_connection_in(&stores.system, &stores.host, None).unwrap();
        let captured_req = handle_ok.join().unwrap();
        assert_eq!(res.profile_id, "ok-prof");
        assert_eq!(res.status_code, 200);
        assert!(captured_req.starts_with("GET /v1/models "));
        assert!(
            captured_req
                .to_ascii_lowercase()
                .contains("authorization: bearer sk-or-v1-probe-key-1111")
        );

        let (base_err, handle_err) =
            spawn_single_response_server("401 Unauthorized", r#"{"error":"bad key"}"#);
        save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenAiResponses,
                profile_id: "bad-prof".to_owned(),
                model: "gpt-5.4".to_owned(),
                base_url: Some(base_err),
                api_key: Some("sk-bad-key-2222".to_owned()),
                project_id: None,
                location: None,
                set_as_default: false,
            },
        )
        .unwrap();

        let auth_err =
            test_alan_connection_in(&stores.system, &stores.host, Some("bad-prof")).unwrap_err();
        let _ = handle_err.join().unwrap();
        assert!(auth_err.to_string().contains("Authentication failed"));
        assert!(auth_err.to_string().contains("401"));
    }

    #[test]
    fn vertex_probe_urls_map_global_and_regional_catalog_hosts() {
        assert_eq!(
            vertex_catalog_probe_url("global"),
            "https://us-central1-aiplatform.googleapis.com/v1beta1/publishers/google/models?pageSize=1"
        );
        assert_eq!(
            vertex_catalog_probe_url(""),
            "https://us-central1-aiplatform.googleapis.com/v1beta1/publishers/google/models?pageSize=1"
        );
        assert_eq!(
            vertex_catalog_probe_url("europe-west4"),
            "https://europe-west4-aiplatform.googleapis.com/v1beta1/publishers/google/models?pageSize=1"
        );
        assert_eq!(
            vertex_express_probe_url("gemini-2.5-pro"),
            "https://aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-pro:countTokens"
        );
        assert_eq!(
            vertex_express_probe_url("publishers/google/models/gemini-2.5-flash"),
            "https://aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-flash:countTokens"
        );
        assert_eq!(
            vertex_express_probe_url("   "),
            "https://aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-flash:countTokens"
        );
    }

    #[tokio::test]
    async fn test_alan_connection_runs_cleanly_inside_tokio_runtime() {
        let stores = TempStores::new("inside-tokio");
        let (base_ok, handle_ok) = spawn_single_response_server("200 OK", r#"{"data":[]}"#);
        save_alan_connection_in(
            &stores.system,
            &stores.host,
            &SaveAlanConnectionRequest {
                provider: AlanConnectionProvider::OpenRouter,
                profile_id: "tokio-prof".to_owned(),
                model: "moonshotai/kimi-k2.6".to_owned(),
                base_url: Some(base_ok),
                api_key: Some("sk-or-v1-tokio-probe-1234".to_owned()),
                project_id: None,
                location: None,
                set_as_default: true,
            },
        )
        .unwrap();

        let res = test_alan_connection_in(&stores.system, &stores.host, None).unwrap();
        let captured_req = handle_ok.join().unwrap();
        assert_eq!(res.profile_id, "tokio-prof");
        assert_eq!(res.status_code, 200);
        assert!(captured_req.starts_with("GET /v1/models "));
    }
}
