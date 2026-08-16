//! Shared, Tauri-independent AI settings and OpenAI-compatible request policy.
//!
//! API keys remain inside trusted Rust code. They may be written through an
//! update, but snapshots, mutations, probe results, action results, errors,
//! and debug output never contain the secret-bearing profile representation.

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::encryption::{decrypt_value, encrypt_value, ENCRYPT_PREFIX};

pub const MAX_PROFILES: usize = 20;
pub const MAX_INPUT_CHARS: usize = 10_000;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const MIN_THINKING_BUDGET: i32 = 1_024;
pub const MAX_THINKING_BUDGET: i32 = 10_000;

const KEY_ENABLED: &str = "ai_enabled";
const KEY_PROFILES: &str = "ai_profiles";
const KEY_ASSIGNED_TASK: &str = "ai_assigned_profile_task";
const KEY_ASSIGNED_MOUTHPIECE: &str = "ai_assigned_profile_mouthpiece";
const KEY_ASSIGNED_TRANSLATE: &str = "ai_assigned_profile_translate";
const KEY_TARGET_LANG: &str = "ai_target_lang";
const KEY_THINKING_BUDGET: &str = "ai_thinking_budget";
const LEGACY_PLAIN_PREFIX: &str = "plain:";
const REQUEST_TIMEOUT_SECS: u64 = 120;
const CONNECT_TIMEOUT_SECS: u64 = 10;

const STORED_KEYS: &[&str] = &[
    KEY_ENABLED,
    KEY_PROFILES,
    KEY_ASSIGNED_TASK,
    KEY_ASSIGNED_MOUTHPIECE,
    KEY_ASSIGNED_TRANSLATE,
    KEY_TARGET_LANG,
    KEY_THINKING_BUDGET,
];

static NEXT_PROFILE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AiProfileSummary {
    pub id: String,
    pub base_url: String,
    pub model: String,
    pub enable_thinking: bool,
    pub api_key_configured: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AiSettingsSnapshot {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub enabled: bool,
    pub profiles: Vec<AiProfileSummary>,
    pub assigned_profile_task: String,
    pub assigned_profile_mouthpiece: String,
    pub assigned_profile_translate: String,
    pub target_lang: String,
    pub thinking_budget: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiProfileUpdate {
    #[serde(default)]
    pub id: String,
    pub base_url: String,
    pub model: String,
    pub enable_thinking: bool,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiSettingsUpdate {
    pub enabled: bool,
    pub profiles: Vec<AiProfileUpdate>,
    pub assigned_profile_task: String,
    pub assigned_profile_mouthpiece: String,
    pub assigned_profile_translate: String,
    pub target_lang: String,
    pub thinking_budget: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AiSettingsMutation {
    #[serde(flatten)]
    pub snapshot: AiSettingsSnapshot,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AiProbeResult {
    pub reachable: bool,
    pub profile_id: String,
    pub model: String,
    pub secure_transport: bool,
    pub status_code: Option<u16>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AiActionResult {
    pub action: String,
    pub profile_id: String,
    pub model: String,
    pub content: String,
    pub input_chars: usize,
    pub input_truncated: bool,
    pub output_chars: usize,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiErrorKind {
    InvalidDatabase,
    Storage,
    Validation,
    ReadOnly,
    Network,
    Provider,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiError {
    kind: AiErrorKind,
    message: String,
}

impl AiError {
    fn new(kind: AiErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> AiErrorKind {
        self.kind
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AiError {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAiProfile {
    id: String,
    base_url: String,
    api_key: String,
    model: String,
    #[serde(default)]
    enable_thinking: bool,
}

enum AiSettingsAdapter {
    Memory(BTreeMap<String, String>),
    Sqlite {
        database_path: PathBuf,
        read_only: bool,
    },
}

pub struct AiSettings {
    adapter: AiSettingsAdapter,
    generation: u64,
}

struct ResolvedAiSettings {
    enabled: bool,
    profiles: Vec<StoredAiProfile>,
    assigned_profile_task: String,
    assigned_profile_mouthpiece: String,
    assigned_profile_translate: String,
    target_lang: String,
    thinking_budget: i32,
}

struct AiRuntimeConfig {
    profile_id: String,
    base_url: String,
    api_key: String,
    model: String,
    enable_thinking: bool,
    target_lang: String,
    thinking_budget: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiAction {
    Task,
    Mouthpiece,
    Translate,
}

impl AiAction {
    fn parse(value: &str) -> Result<Self, AiError> {
        match value.trim() {
            "task" => Ok(Self::Task),
            "mouthpiece" => Ok(Self::Mouthpiece),
            "translate" => Ok(Self::Translate),
            _ => Err(validation_error(
                "AI 动作必须是 task、mouthpiece 或 translate",
            )),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Mouthpiece => "mouthpiece",
            Self::Translate => "translate",
        }
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enable_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_budget: Option<i32>,
    max_tokens: i32,
    temperature: f32,
    presence_penalty: f32,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: Option<ChatResponseMessage>,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: Option<String>,
}

impl AiSettings {
    pub fn in_memory() -> Self {
        Self {
            adapter: AiSettingsAdapter::Memory(default_values()),
            generation: 1,
        }
    }

    pub fn open_sqlite(
        database_path: impl Into<PathBuf>,
        read_only: bool,
    ) -> Result<Self, AiError> {
        let database_path = database_path.into();
        let connection = open_connection(&database_path, read_only)?;
        connection
            .query_row("SELECT 1 FROM settings LIMIT 1", [], |_| Ok(()))
            .optional()
            .map_err(|error| storage_error("无法读取 AI 设置表", error))?;
        Ok(Self {
            adapter: AiSettingsAdapter::Sqlite {
                database_path,
                read_only,
            },
            generation: 1,
        })
    }

    pub fn snapshot(&self) -> Result<AiSettingsSnapshot, AiError> {
        let (adapter, read_only, values) = self.load_values()?;
        let resolved = resolve_values(&values)?;
        Ok(snapshot_from_resolved(
            adapter,
            read_only,
            self.generation,
            &resolved,
        ))
    }

    pub fn update(&mut self, update: AiSettingsUpdate) -> Result<AiSettingsMutation, AiError> {
        let (_, read_only, values) = self.load_values()?;
        if read_only {
            return Err(AiError::new(
                AiErrorKind::ReadOnly,
                "当前数据库为只读，不能修改 AI 设置",
            ));
        }
        let current = resolve_values(&values)?;
        let next = validate_update(update, &current.profiles)?;
        let stored = values_from_resolved(&next)?;
        self.store_values(stored)?;
        self.generation = self.generation.saturating_add(1);
        Ok(AiSettingsMutation {
            snapshot: self.snapshot()?,
            message: "AI 设置已安全保存；API Key 不会回传到界面".to_owned(),
        })
    }

    pub fn probe_profile(&self, profile_id: &str) -> Result<AiProbeResult, AiError> {
        let (_, _, values) = self.load_values()?;
        let resolved = resolve_values(&values)?;
        let profile = resolved
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id.trim())
            .ok_or_else(|| validation_error("找不到要测试的 AI 模型"))?;
        validate_profile(profile)?;
        let url = chat_completions_url(&profile.base_url)?;
        let secure_transport = url.scheme() == "https";
        let body = serde_json::to_string(&serde_json::json!({
            "model": profile.model,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1
        }))
        .map_err(|error| provider_error(format!("无法创建 AI 测试请求：{error}")))?;
        let response = send_request(&url, &profile.api_key, body)?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_status_error(status));
        }
        Ok(AiProbeResult {
            reachable: true,
            profile_id: profile.id.clone(),
            model: profile.model.clone(),
            secure_transport,
            status_code: Some(status.as_u16()),
            message: "AI 服务连接成功，密钥未离开 Rust 核心".to_owned(),
        })
    }

    pub fn run_action(&self, action: &str, content: &str) -> Result<AiActionResult, AiError> {
        let action = AiAction::parse(action)?;
        let config = self.runtime_config(action)?;
        let input_chars = content.chars().count();
        let sanitized: String = content.chars().take(MAX_INPUT_CHARS).collect();
        if sanitized.trim().is_empty() {
            return Err(validation_error("不能把空内容发送给 AI"));
        }
        let effective_target = effective_target_language(action, &config.target_lang, &sanitized);
        let thinking = config.enable_thinking;
        let max_tokens = if thinking {
            2_000.max(config.thinking_budget.saturating_add(1_500))
        } else {
            2_000
        };
        let request = ChatCompletionRequest {
            model: config.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system_prompt(action).to_owned(),
                },
                ChatMessage {
                    role: "user",
                    content: user_prompt(action, &sanitized, &effective_target),
                },
            ],
            enable_thinking: thinking.then_some(true),
            thinking_budget: thinking.then_some(config.thinking_budget),
            max_tokens,
            temperature: 0.7,
            presence_penalty: if action == AiAction::Mouthpiece {
                0.6
            } else {
                0.0
            },
        };
        let body = serde_json::to_string(&request)
            .map_err(|error| provider_error(format!("无法创建 AI 请求：{error}")))?;
        let url = chat_completions_url(&config.base_url)?;
        let response = send_request(&url, &config.api_key, body)?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_status_error(status));
        }
        let response_bytes = read_bounded_response(response)?;
        let completion = serde_json::from_slice::<ChatCompletionResponse>(&response_bytes)
            .map_err(|_| provider_error("AI 服务返回了无法识别的响应"))?;
        let raw = completion
            .choices
            .first()
            .and_then(|choice| choice.message.as_ref())
            .and_then(|message| message.content.as_deref())
            .unwrap_or_default();
        let mut result = raw.trim().to_owned();
        if action == AiAction::Mouthpiece {
            result = strip_wrapping_quotes(&result);
        }
        if result.is_empty() {
            return Err(provider_error("AI 服务返回了空结果"));
        }
        let output_chars = result.chars().count();
        Ok(AiActionResult {
            action: action.key().to_owned(),
            profile_id: config.profile_id,
            model: config.model,
            content: result,
            input_chars: input_chars.min(MAX_INPUT_CHARS),
            input_truncated: input_chars > MAX_INPUT_CHARS,
            output_chars,
            message: "AI 已生成结果；原剪贴板记录未被修改".to_owned(),
        })
    }

    fn runtime_config(&self, action: AiAction) -> Result<AiRuntimeConfig, AiError> {
        let (_, _, values) = self.load_values()?;
        let resolved = resolve_values(&values)?;
        if !resolved.enabled {
            return Err(validation_error("请先在 AI 助手中启用 AI 功能"));
        }
        let assigned = match action {
            AiAction::Task => &resolved.assigned_profile_task,
            AiAction::Mouthpiece => &resolved.assigned_profile_mouthpiece,
            AiAction::Translate => &resolved.assigned_profile_translate,
        };
        let profile = resolved
            .profiles
            .iter()
            .find(|profile| &profile.id == assigned)
            .or_else(|| resolved.profiles.first())
            .ok_or_else(|| validation_error("请先添加 AI 模型"))?;
        validate_profile(profile)?;
        Ok(AiRuntimeConfig {
            profile_id: profile.id.clone(),
            base_url: profile.base_url.clone(),
            api_key: profile.api_key.clone(),
            model: profile.model.clone(),
            enable_thinking: profile.enable_thinking,
            target_lang: resolved.target_lang,
            thinking_budget: resolved.thinking_budget,
        })
    }

    fn load_values(&self) -> Result<(&'static str, bool, BTreeMap<String, String>), AiError> {
        match &self.adapter {
            AiSettingsAdapter::Memory(values) => Ok(("memory", false, values.clone())),
            AiSettingsAdapter::Sqlite {
                database_path,
                read_only,
            } => {
                let connection = open_connection(database_path, *read_only)?;
                let mut values = default_values();
                for key in STORED_KEYS {
                    if let Some(value) = read_setting(&connection, key)? {
                        values.insert((*key).to_owned(), value);
                    }
                }
                Ok((
                    if *read_only {
                        "sqlite-read-only"
                    } else {
                        "sqlite"
                    },
                    *read_only,
                    values,
                ))
            }
        }
    }

    fn store_values(&mut self, values: BTreeMap<String, String>) -> Result<(), AiError> {
        match &mut self.adapter {
            AiSettingsAdapter::Memory(current) => {
                *current = values;
                Ok(())
            }
            AiSettingsAdapter::Sqlite {
                database_path,
                read_only: true,
            } => Err(AiError::new(
                AiErrorKind::ReadOnly,
                format!("只读数据库不能保存 AI 设置：{}", database_path.display()),
            )),
            AiSettingsAdapter::Sqlite { database_path, .. } => {
                let mut connection = open_connection(database_path, false)?;
                let transaction = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|error| storage_error("无法开始 AI 设置事务", error))?;
                for key in STORED_KEYS {
                    let value = values.get(*key).cloned().unwrap_or_default();
                    let stored = if *key == KEY_PROFILES {
                        encrypt_value(&value).ok_or_else(|| {
                            AiError::new(
                                AiErrorKind::Storage,
                                "无法使用 Windows DPAPI 保护 AI 密钥",
                            )
                        })?
                    } else {
                        value
                    };
                    transaction
                        .execute(
                            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                            rusqlite::params![key, stored],
                        )
                        .map_err(|error| storage_error("无法写入 AI 设置", error))?;
                }
                transaction
                    .commit()
                    .map_err(|error| storage_error("无法提交 AI 设置", error))?;
                Ok(())
            }
        }
    }
}

fn default_values() -> BTreeMap<String, String> {
    BTreeMap::from([
        (KEY_ENABLED.to_owned(), "false".to_owned()),
        (KEY_PROFILES.to_owned(), "[]".to_owned()),
        (KEY_ASSIGNED_TASK.to_owned(), "none".to_owned()),
        (KEY_ASSIGNED_MOUTHPIECE.to_owned(), "none".to_owned()),
        (KEY_ASSIGNED_TRANSLATE.to_owned(), "none".to_owned()),
        (KEY_TARGET_LANG.to_owned(), "zh".to_owned()),
        (
            KEY_THINKING_BUDGET.to_owned(),
            MIN_THINKING_BUDGET.to_string(),
        ),
    ])
}

fn resolve_values(values: &BTreeMap<String, String>) -> Result<ResolvedAiSettings, AiError> {
    let profiles_json =
        decode_profiles_value(values.get(KEY_PROFILES).map(String::as_str).unwrap_or("[]"))?;
    let profiles = serde_json::from_str::<Vec<StoredAiProfile>>(&profiles_json)
        .map_err(|_| validation_error("已保存的 AI 模型配置无法解析"))?;
    if profiles.len() > MAX_PROFILES {
        return Err(validation_error(format!(
            "AI 模型最多保存 {MAX_PROFILES} 个"
        )));
    }
    let ids = profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<HashSet<_>>();
    if ids.len() != profiles.len() {
        return Err(validation_error("已保存的 AI 模型 ID 重复"));
    }
    let fallback = profiles
        .first()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| "none".to_owned());
    let assignment = |key: &str| {
        values
            .get(key)
            .filter(|value| ids.contains(value.as_str()))
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    };
    let target_lang = normalize_target_language(
        values
            .get(KEY_TARGET_LANG)
            .map(String::as_str)
            .unwrap_or("zh"),
    )?;
    let thinking_budget = values
        .get(KEY_THINKING_BUDGET)
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(MIN_THINKING_BUDGET)
        .clamp(MIN_THINKING_BUDGET, MAX_THINKING_BUDGET);
    Ok(ResolvedAiSettings {
        enabled: parse_bool(
            values
                .get(KEY_ENABLED)
                .map(String::as_str)
                .unwrap_or("false"),
        ),
        profiles,
        assigned_profile_task: assignment(KEY_ASSIGNED_TASK),
        assigned_profile_mouthpiece: assignment(KEY_ASSIGNED_MOUTHPIECE),
        assigned_profile_translate: assignment(KEY_ASSIGNED_TRANSLATE),
        target_lang,
        thinking_budget,
    })
}

fn validate_update(
    update: AiSettingsUpdate,
    existing: &[StoredAiProfile],
) -> Result<ResolvedAiSettings, AiError> {
    if update.profiles.len() > MAX_PROFILES {
        return Err(validation_error(format!(
            "AI 模型最多保存 {MAX_PROFILES} 个"
        )));
    }
    let existing_by_id = existing
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect::<HashMap<_, _>>();
    let mut ids = HashSet::new();
    let mut profiles = Vec::with_capacity(update.profiles.len());
    for incoming in update.profiles {
        let id = if incoming.id.trim().is_empty() {
            next_profile_id()
        } else {
            validate_profile_id(&incoming.id)?
        };
        if !ids.insert(id.clone()) {
            return Err(validation_error("AI 模型 ID 不能重复"));
        }
        let existing_key = existing_by_id
            .get(id.as_str())
            .map(|profile| profile.api_key.clone())
            .unwrap_or_default();
        let api_key = if incoming.clear_api_key {
            String::new()
        } else if let Some(value) = incoming.api_key {
            let value = value.trim().to_owned();
            if value.len() > 4_096 {
                return Err(validation_error("AI API Key 过长"));
            }
            value
        } else {
            existing_key
        };
        let profile = StoredAiProfile {
            id,
            base_url: normalize_base_url(&incoming.base_url)?,
            api_key,
            model: normalize_model(&incoming.model)?,
            enable_thinking: incoming.enable_thinking,
        };
        validate_profile_definition(&profile)?;
        profiles.push(profile);
    }
    if update.enabled && profiles.is_empty() {
        return Err(validation_error("启用 AI 前至少需要添加一个模型"));
    }
    let valid_ids = profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<HashSet<_>>();
    let fallback = profiles
        .first()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| "none".to_owned());
    let assignment = |value: String| {
        if valid_ids.contains(value.trim()) {
            value.trim().to_owned()
        } else {
            fallback.clone()
        }
    };
    if !(MIN_THINKING_BUDGET..=MAX_THINKING_BUDGET).contains(&update.thinking_budget) {
        return Err(validation_error(format!(
            "思考预算必须在 {MIN_THINKING_BUDGET} 到 {MAX_THINKING_BUDGET} 之间"
        )));
    }
    Ok(ResolvedAiSettings {
        enabled: update.enabled,
        profiles,
        assigned_profile_task: assignment(update.assigned_profile_task),
        assigned_profile_mouthpiece: assignment(update.assigned_profile_mouthpiece),
        assigned_profile_translate: assignment(update.assigned_profile_translate),
        target_lang: normalize_target_language(&update.target_lang)?,
        thinking_budget: update.thinking_budget,
    })
}

fn values_from_resolved(
    settings: &ResolvedAiSettings,
) -> Result<BTreeMap<String, String>, AiError> {
    let profiles = serde_json::to_string(&settings.profiles)
        .map_err(|error| storage_error("无法编码 AI 模型配置", error))?;
    Ok(BTreeMap::from([
        (KEY_ENABLED.to_owned(), settings.enabled.to_string()),
        (KEY_PROFILES.to_owned(), profiles),
        (
            KEY_ASSIGNED_TASK.to_owned(),
            settings.assigned_profile_task.clone(),
        ),
        (
            KEY_ASSIGNED_MOUTHPIECE.to_owned(),
            settings.assigned_profile_mouthpiece.clone(),
        ),
        (
            KEY_ASSIGNED_TRANSLATE.to_owned(),
            settings.assigned_profile_translate.clone(),
        ),
        (KEY_TARGET_LANG.to_owned(), settings.target_lang.clone()),
        (
            KEY_THINKING_BUDGET.to_owned(),
            settings.thinking_budget.to_string(),
        ),
    ]))
}

fn snapshot_from_resolved(
    adapter: &'static str,
    read_only: bool,
    generation: u64,
    settings: &ResolvedAiSettings,
) -> AiSettingsSnapshot {
    AiSettingsSnapshot {
        adapter,
        read_only,
        generation,
        enabled: settings.enabled,
        profiles: settings
            .profiles
            .iter()
            .map(|profile| AiProfileSummary {
                id: profile.id.clone(),
                base_url: profile.base_url.clone(),
                model: profile.model.clone(),
                enable_thinking: profile.enable_thinking,
                api_key_configured: !profile.api_key.is_empty(),
            })
            .collect(),
        assigned_profile_task: settings.assigned_profile_task.clone(),
        assigned_profile_mouthpiece: settings.assigned_profile_mouthpiece.clone(),
        assigned_profile_translate: settings.assigned_profile_translate.clone(),
        target_lang: settings.target_lang.clone(),
        thinking_budget: settings.thinking_budget,
    }
}

fn validate_profile(profile: &StoredAiProfile) -> Result<(), AiError> {
    validate_profile_definition(profile)?;
    if profile.api_key.trim().is_empty() {
        return Err(validation_error(format!(
            "模型“{}”尚未配置 API Key",
            profile.model
        )));
    }
    Ok(())
}

fn validate_profile_definition(profile: &StoredAiProfile) -> Result<(), AiError> {
    validate_profile_id(&profile.id)?;
    normalize_model(&profile.model)?;
    chat_completions_url(&profile.base_url)?;
    if profile.api_key.len() > 4_096 {
        return Err(validation_error("AI API Key 过长"));
    }
    Ok(())
}

fn validate_profile_id(value: &str) -> Result<String, AiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return Err(validation_error("AI 模型 ID 无效"));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(validation_error(
            "AI 模型 ID 只能包含字母、数字、点、横线和下划线",
        ));
    }
    Ok(value.to_owned())
}

fn next_profile_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = NEXT_PROFILE_ID.fetch_add(1, Ordering::Relaxed);
    format!("native-{timestamp:x}-{sequence:x}")
}

fn normalize_model(value: &str) -> Result<String, AiError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 200 || value.contains(['\r', '\n', '\0']) {
        return Err(validation_error(
            "AI 模型名称不能为空、换行或超过 200 个字符",
        ));
    }
    Ok(value.to_owned())
}

fn normalize_base_url(value: &str) -> Result<String, AiError> {
    let mut url = validated_base_url(value)?;
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn validated_base_url(value: &str) -> Result<Url, AiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 2_048 {
        return Err(validation_error("AI API 地址不能为空或超过 2048 个字符"));
    }
    let url = Url::parse(value).map_err(|_| validation_error("AI API 地址格式无效"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(validation_error("AI API 地址不能嵌入用户名或密码"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(validation_error("AI API 地址不能包含查询参数或片段"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| validation_error("AI API 地址缺少主机名"))?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(host) => {}
        "http" => return Err(validation_error("远程 AI 服务必须使用 HTTPS")),
        _ => return Err(validation_error("AI API 地址只支持 HTTPS 或本机回环 HTTP")),
    }
    Ok(url)
}

fn chat_completions_url(base_url: &str) -> Result<Url, AiError> {
    let mut url = validated_base_url(base_url)?;
    let current = url.path().trim_end_matches('/');
    if !current.ends_with("/chat/completions") && current != "chat/completions" {
        let next = if current.is_empty() || current == "/" {
            "/chat/completions".to_owned()
        } else {
            format!("{current}/chat/completions")
        };
        url.set_path(&next);
    }
    Ok(url)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

fn normalize_target_language(value: &str) -> Result<String, AiError> {
    match value.trim() {
        "auto_zh_en" | "zh" | "en" | "ja" | "de" | "fr" => Ok(value.trim().to_owned()),
        _ => Err(validation_error("翻译目标语言无效")),
    }
}

fn effective_target_language(action: AiAction, configured: &str, content: &str) -> String {
    if action == AiAction::Translate && configured == "auto_zh_en" {
        if content
            .chars()
            .any(|value| matches!(value, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}'))
        {
            "en".to_owned()
        } else {
            "zh".to_owned()
        }
    } else {
        configured.to_owned()
    }
}

fn system_prompt(action: AiAction) -> &'static str {
    match action {
        AiAction::Task => {
            "你是一个专注于任务解决的智能助手。直接提供需要的结果，结构清晰、逻辑严密、专业可靠，不要过度寒暄。"
        }
        AiAction::Mouthpiece => {
            "你是用户的社交嘴替。只替用户回复对方内容，语气自然、边界清晰、简短有力；不要输出分析、理由、引号或 Emoji。"
        }
        AiAction::Translate => {
            "你是专业同传译员。只输出地道、自然的译文，不要添加解释、标题或其他内容。"
        }
    }
}

fn user_prompt(action: AiAction, content: &str, target_lang: &str) -> String {
    match action {
        AiAction::Task => content.to_owned(),
        AiAction::Mouthpiece => {
            format!("这是对方的内容或消息：\n\n“{content}”\n\n请只针对这段内容进行回复。")
        }
        AiAction::Translate => {
            let language = match target_lang {
                "en" => "English",
                "ja" => "Japanese",
                "de" => "German",
                "fr" => "French",
                _ => "Chinese",
            };
            format!("请将以下内容翻译为 {language}：\n\n{content}")
        }
    }
}

fn send_request(
    url: &Url,
    api_key: &str,
    body: String,
) -> Result<reqwest::blocking::Response, AiError> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .redirect(Policy::none())
        .build()
        .map_err(|_| network_error("无法创建 AI 网络客户端"))?;
    client
        .post(url.clone())
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .header("X-Title", "TieZ Clipboard")
        .body(body)
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                network_error("AI 请求超时")
            } else {
                network_error("无法连接 AI 服务")
            }
        })
}

fn read_bounded_response(response: reqwest::blocking::Response) -> Result<Vec<u8>, AiError> {
    let mut bytes = Vec::new();
    response
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| network_error("读取 AI 响应失败"))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(provider_error("AI 响应超过安全大小限制"));
    }
    Ok(bytes)
}

fn strip_wrapping_quotes(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    let Some(last) = characters.next_back() else {
        return value.to_owned();
    };
    if matches!((first, last), ('"', '"') | ('\'', '\'') | ('“', '”')) {
        characters.as_str().trim().to_owned()
    } else {
        value.to_owned()
    }
}

fn decode_profiles_value(value: &str) -> Result<String, AiError> {
    let mut current = value.to_owned();
    for _ in 0..4 {
        while let Some(stripped) = current.strip_prefix(LEGACY_PLAIN_PREFIX) {
            current = stripped.to_owned();
        }
        if !current.starts_with(ENCRYPT_PREFIX) {
            return Ok(current);
        }
        current = decrypt_value(&current)
            .ok_or_else(|| validation_error("当前 Windows 账户无法解密已保存的 AI 模型"))?;
    }
    if current.starts_with(ENCRYPT_PREFIX) {
        Err(validation_error("AI 模型配置的加密层级异常"))
    } else {
        Ok(current)
    }
}

fn open_connection(path: &PathBuf, read_only: bool) -> Result<Connection, AiError> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    Connection::open_with_flags(path, flags).map_err(|error| {
        AiError::new(
            AiErrorKind::InvalidDatabase,
            format!("无法打开 AI 设置数据库 {}：{error}", path.display()),
        )
    })
}

fn read_setting(connection: &Connection, key: &str) -> Result<Option<String>, AiError> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|error| storage_error("无法读取 AI 设置", error))
}

fn parse_bool(value: &str) -> bool {
    value.eq_ignore_ascii_case("true") || value == "1"
}

fn http_status_error(status: StatusCode) -> AiError {
    AiError::new(
        AiErrorKind::Provider,
        format!(
            "AI 服务返回 HTTP {}；响应正文已隐藏以保护剪贴板内容",
            status.as_u16()
        ),
    )
}

fn validation_error(message: impl Into<String>) -> AiError {
    AiError::new(AiErrorKind::Validation, message)
}

fn storage_error(message: impl Into<String>, error: impl fmt::Display) -> AiError {
    AiError::new(AiErrorKind::Storage, format!("{}：{error}", message.into()))
}

fn network_error(message: impl Into<String>) -> AiError {
    AiError::new(AiErrorKind::Network, message)
}

fn provider_error(message: impl Into<String>) -> AiError {
    AiError::new(AiErrorKind::Provider, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    fn profile(api_key: Option<&str>) -> AiProfileUpdate {
        AiProfileUpdate {
            id: "primary".to_owned(),
            base_url: "https://example.com/v1".to_owned(),
            model: "test-model".to_owned(),
            enable_thinking: true,
            api_key: api_key.map(str::to_owned),
            clear_api_key: false,
        }
    }

    fn update(profile: AiProfileUpdate) -> AiSettingsUpdate {
        AiSettingsUpdate {
            enabled: true,
            profiles: vec![profile],
            assigned_profile_task: "primary".to_owned(),
            assigned_profile_mouthpiece: "primary".to_owned(),
            assigned_profile_translate: "primary".to_owned(),
            target_lang: "auto_zh_en".to_owned(),
            thinking_budget: 2_048,
        }
    }

    #[test]
    fn snapshots_never_serialize_api_keys_and_blank_updates_preserve_them() {
        let mut settings = AiSettings::in_memory();
        let mutation = settings
            .update(update(profile(Some("secret-token"))))
            .unwrap();
        let serialized = serde_json::to_string(&mutation).unwrap();
        assert!(!serialized.contains("secret-token"));
        assert!(!serialized.contains("api_key\""));
        assert!(serialized.contains("api_key_configured"));

        let preserved = settings.update(update(profile(None))).unwrap();
        assert!(preserved.snapshot.profiles[0].api_key_configured);
        assert_eq!(
            settings.runtime_config(AiAction::Task).unwrap().api_key,
            "secret-token"
        );
    }

    #[test]
    fn unconfigured_profiles_can_be_saved_but_cannot_make_requests() {
        let mut settings = AiSettings::in_memory();
        let mutation = settings.update(update(profile(None))).unwrap();
        assert!(!mutation.snapshot.profiles[0].api_key_configured);
        let error = match settings.runtime_config(AiAction::Task) {
            Ok(_) => panic!("unconfigured profile must not make a request"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), AiErrorKind::Validation);
    }

    #[test]
    fn endpoints_require_https_except_for_loopback() {
        assert!(normalize_base_url("https://api.example.com/v1").is_ok());
        assert!(normalize_base_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(normalize_base_url("http://[::1]:11434/v1").is_ok());
        assert!(normalize_base_url("http://api.example.com/v1").is_err());
        assert!(normalize_base_url("https://user:secret@example.com/v1").is_err());
        assert!(normalize_base_url("https://example.com/v1?token=secret").is_err());
    }

    #[test]
    fn invalid_actions_and_oversized_budgets_fail_closed() {
        let mut settings = AiSettings::in_memory();
        settings
            .update(update(profile(Some("secret-token"))))
            .unwrap();
        assert_eq!(
            settings
                .run_action("summarize", "hello")
                .unwrap_err()
                .kind(),
            AiErrorKind::Validation
        );
        let mut invalid = update(profile(Some("secret-token")));
        invalid.thinking_budget = MAX_THINKING_BUDGET + 1;
        assert_eq!(
            settings.update(invalid).unwrap_err().kind(),
            AiErrorKind::Validation
        );
    }

    #[test]
    fn sqlite_adapter_preserves_tauri_profile_shape_and_read_only_rules() {
        let path = std::env::temp_dir().join(format!("tiez-ai-{}.db", next_profile_id()));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
                [],
            )
            .unwrap();
        drop(connection);

        let mut settings = AiSettings::open_sqlite(&path, false).unwrap();
        settings
            .update(update(profile(Some("sqlite-secret"))))
            .unwrap();
        let connection = Connection::open(&path).unwrap();
        let stored: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'ai_profiles'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        #[cfg(windows)]
        assert!(stored.starts_with(ENCRYPT_PREFIX));
        #[cfg(not(windows))]
        assert!(stored.contains("apiKey"));
        drop(connection);

        let mut read_only = AiSettings::open_sqlite(&path, true).unwrap();
        assert!(read_only.snapshot().unwrap().profiles[0].api_key_configured);
        assert_eq!(
            read_only.update(update(profile(None))).unwrap_err().kind(),
            AiErrorKind::ReadOnly
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn local_openai_compatible_response_runs_without_mutating_input() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = vec![0_u8; 16 * 1024];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
            assert!(
                request.contains("authorization: Bearer local-secret")
                    || request.contains("Authorization: Bearer local-secret")
            );
            assert!(request.contains("  保留空白  "));
            let body = serde_json::json!({
                "choices": [{"message": {"content": "\"reply\""}}]
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        let mut settings = AiSettings::in_memory();
        let mut local = profile(Some("local-secret"));
        local.base_url = format!("http://{address}/v1");
        settings.update(update(local)).unwrap();
        let result = settings.run_action("mouthpiece", "  保留空白  ").unwrap();
        assert_eq!(result.content, "reply");
        assert_eq!(result.input_chars, 8);
        assert!(!result.input_truncated);
        server.join().unwrap();
    }
}
