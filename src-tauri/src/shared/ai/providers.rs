// Provider × Model registry.
//
// Each entry is the single source of truth for a (provider, model) pair:
// API URL, API kind, model id, token caps, capability flags, and which
// settings key holds the API key. Adding a new pair is one entry here and
// zero edits to call sites in `commands/ai/*`.
//
// Behaviour parity note: the values below are the previously hardcoded
// constants from `commands/ai/{anthropic,openai,mod}.rs`, lifted into data.

use serde::{Deserialize, Serialize};

/// Identifies a provider. Serde representation matches the string the
/// frontend sends (`ai_provider` in settings) and uses for the
/// `ai_api_key_<provider>` settings key suffix.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum ProviderId {
    #[serde(rename = "claude")]
    Claude,
    #[serde(rename = "groq")]
    Groq,
    #[serde(rename = "mistral")]
    Mistral,
    #[serde(rename = "openai_gh")]
    OpenAIGitHub,
    #[serde(rename = "nvidia")]
    Nvidia,
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "openai_direct")]
    OpenAI,
    #[serde(rename = "gemini")]
    Gemini,
}

impl ProviderId {
    /// String slug used in settings keys / frontend dropdown values.
    #[allow(dead_code)]
    pub fn as_slug(self) -> &'static str {
        match self {
            ProviderId::Claude => "claude",
            ProviderId::Groq => "groq",
            ProviderId::Mistral => "mistral",
            ProviderId::OpenAIGitHub => "openai_gh",
            ProviderId::Nvidia => "nvidia",
            ProviderId::OpenRouter => "openrouter",
            ProviderId::OpenAI => "openai_direct",
            ProviderId::Gemini => "gemini",
        }
    }

    /// Parse a slug back into a `ProviderId`. Unknown slugs return `None`.
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "claude" => Some(ProviderId::Claude),
            "groq" => Some(ProviderId::Groq),
            "mistral" => Some(ProviderId::Mistral),
            "openai_gh" => Some(ProviderId::OpenAIGitHub),
            "nvidia" => Some(ProviderId::Nvidia),
            "openrouter" => Some(ProviderId::OpenRouter),
            "openai_direct" => Some(ProviderId::OpenAI),
            "gemini" => Some(ProviderId::Gemini),
            _ => None,
        }
    }
}

/// Wire format / client kind. Drives which streaming client formats the
/// request and parses the response.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApiKind {
    /// Native Anthropic /v1/messages SSE.
    AnthropicMessages,
    /// OpenAI-compatible /chat/completions SSE (Groq, Mistral, OpenRouter,
    /// NVIDIA NIM, GitHub Models, Gemini's OpenAI-compat endpoint, etc.).
    OpenAICompat,
}

/// Full configuration for one (provider, model) pair.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub provider_id: ProviderId,
    pub model_id: &'static str,
    pub display_name: &'static str,
    pub api_url: &'static str,
    pub api_kind: ApiKind,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub default_temperature: f32,
    pub supports_caching: bool,
    pub supports_parallel_tools: bool,
    pub supports_thinking: bool,
    pub daily_token_budget: Option<u64>,
    pub key_setting_name: &'static str,
    /// Anthropic API version header (only meaningful for `AnthropicMessages`).
    pub anthropic_version: Option<&'static str>,
    /// Required key prefix used by `test_ai_key` for client-side validation
    /// (e.g. `sk-ant-` for Claude, `gsk_` for Groq, `sk-` for OpenAI).
    /// `None` means the provider has no enforced prefix.
    pub key_prefix: Option<&'static str>,
}

/// The registry. Order matters for `list_all_providers` (UI ordering).
const REGISTRY: &[ProviderConfig] = &[
    ProviderConfig {
        provider_id: ProviderId::Claude,
        model_id: "claude-haiku-4-5-20251001",
        display_name: "Claude Haiku 4.5",
        api_url: "https://api.anthropic.com/v1/messages",
        api_kind: ApiKind::AnthropicMessages,
        max_input_tokens: 200_000,
        max_output_tokens: 4096,
        default_temperature: 1.0,
        supports_caching: true,
        supports_parallel_tools: false,
        supports_thinking: true,
        daily_token_budget: None,
        key_setting_name: "ai_api_key_claude",
        anthropic_version: Some("2023-06-01"),
        key_prefix: Some("sk-ant-"),
    },
    ProviderConfig {
        provider_id: ProviderId::Groq,
        model_id: "meta-llama/llama-4-scout-17b-16e-instruct",
        display_name: "Llama 4 Scout 17B (Groq)",
        api_url: "https://api.groq.com/openai/v1/chat/completions",
        api_kind: ApiKind::OpenAICompat,
        max_input_tokens: 128_000,
        max_output_tokens: 4096,
        default_temperature: 0.1,
        supports_caching: false,
        supports_parallel_tools: true,
        supports_thinking: false,
        daily_token_budget: None,
        key_setting_name: "ai_api_key_groq",
        anthropic_version: None,
        key_prefix: Some("gsk_"),
    },
    ProviderConfig {
        provider_id: ProviderId::Mistral,
        model_id: "mistral-large-latest",
        display_name: "Mistral Large",
        api_url: "https://api.mistral.ai/v1/chat/completions",
        api_kind: ApiKind::OpenAICompat,
        max_input_tokens: 128_000,
        max_output_tokens: 4096,
        default_temperature: 0.1,
        supports_caching: false,
        supports_parallel_tools: true,
        supports_thinking: false,
        daily_token_budget: None,
        key_setting_name: "ai_api_key_mistral",
        anthropic_version: None,
        key_prefix: None,
    },
    ProviderConfig {
        provider_id: ProviderId::OpenAIGitHub,
        model_id: "gpt-4.1-mini",
        display_name: "GPT-4.1 Mini (GitHub Models)",
        api_url: "https://models.inference.ai.azure.com/chat/completions",
        api_kind: ApiKind::OpenAICompat,
        max_input_tokens: 128_000,
        max_output_tokens: 4096,
        default_temperature: 0.1,
        supports_caching: false,
        supports_parallel_tools: true,
        supports_thinking: false,
        daily_token_budget: None,
        key_setting_name: "ai_api_key_openai_gh",
        anthropic_version: None,
        key_prefix: None,
    },
    ProviderConfig {
        provider_id: ProviderId::Nvidia,
        model_id: "nvidia/nemotron-3-super-120b-a12b",
        display_name: "Nemotron 3 Super 120B (NVIDIA NIM)",
        api_url: "https://integrate.api.nvidia.com/v1/chat/completions",
        api_kind: ApiKind::OpenAICompat,
        max_input_tokens: 128_000,
        max_output_tokens: 4096,
        default_temperature: 0.1,
        supports_caching: false,
        supports_parallel_tools: true,
        supports_thinking: false,
        daily_token_budget: None,
        key_setting_name: "ai_api_key_nvidia",
        anthropic_version: None,
        key_prefix: None,
    },
    ProviderConfig {
        provider_id: ProviderId::OpenRouter,
        model_id: "meta-llama/llama-3.3-70b-instruct:free",
        display_name: "Llama 3.3 70B (OpenRouter)",
        api_url: "https://openrouter.ai/api/v1/chat/completions",
        api_kind: ApiKind::OpenAICompat,
        max_input_tokens: 128_000,
        max_output_tokens: 4096,
        default_temperature: 0.1,
        supports_caching: false,
        supports_parallel_tools: true,
        supports_thinking: false,
        daily_token_budget: None,
        key_setting_name: "ai_api_key_openrouter",
        anthropic_version: None,
        key_prefix: None,
    },
    ProviderConfig {
        provider_id: ProviderId::OpenAI,
        model_id: "gpt-4.1-mini",
        display_name: "GPT-4.1 Mini",
        api_url: "https://api.openai.com/v1/chat/completions",
        api_kind: ApiKind::OpenAICompat,
        max_input_tokens: 128_000,
        max_output_tokens: 4096,
        default_temperature: 0.1,
        supports_caching: false,
        supports_parallel_tools: true,
        supports_thinking: false,
        daily_token_budget: None,
        key_setting_name: "ai_api_key_openai_direct",
        anthropic_version: None,
        key_prefix: Some("sk-"),
    },
    ProviderConfig {
        provider_id: ProviderId::Gemini,
        model_id: "gemini-2.5-flash",
        display_name: "Gemini 2.5 Flash",
        api_url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
        api_kind: ApiKind::OpenAICompat,
        max_input_tokens: 1_000_000,
        max_output_tokens: 4096,
        default_temperature: 0.1,
        supports_caching: false,
        supports_parallel_tools: true,
        supports_thinking: false,
        daily_token_budget: None,
        key_setting_name: "ai_api_key_gemini",
        anthropic_version: None,
        key_prefix: None,
    },
];

/// Look up an exact (provider, model) pair.
pub fn get_provider_config(provider: ProviderId, model: &str) -> Option<&'static ProviderConfig> {
    REGISTRY
        .iter()
        .find(|c| c.provider_id == provider && c.model_id == model)
}

/// Backward-compat path: existing user settings only store a provider, not
/// a model. This returns the registry's first entry for that provider,
/// which is treated as the default.
pub fn default_model_for(provider: ProviderId) -> Option<&'static ProviderConfig> {
    REGISTRY.iter().find(|c| c.provider_id == provider)
}

/// All registered models for a given provider, in registry order.
#[allow(dead_code)]
pub fn list_models_for(provider: ProviderId) -> Vec<&'static ProviderConfig> {
    REGISTRY
        .iter()
        .filter(|c| c.provider_id == provider)
        .collect()
}

/// All providers, in registry order, deduplicated.
#[allow(dead_code)]
pub fn list_all_providers() -> Vec<ProviderId> {
    let mut seen: Vec<ProviderId> = Vec::new();
    for cfg in REGISTRY {
        if !seen.contains(&cfg.provider_id) {
            seen.push(cfg.provider_id);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_has_a_default_model() {
        for p in list_all_providers() {
            assert!(
                default_model_for(p).is_some(),
                "missing default model for {:?}",
                p
            );
        }
    }

    #[test]
    fn slug_roundtrip() {
        for p in list_all_providers() {
            assert_eq!(ProviderId::from_slug(p.as_slug()), Some(p));
        }
    }

    #[test]
    fn registry_has_expected_pairs() {
        // Behaviour parity guard: the audit catalogued 8 providers.
        assert_eq!(list_all_providers().len(), 8);
    }
}
