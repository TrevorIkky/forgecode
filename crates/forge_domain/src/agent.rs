use std::borrow::Cow;

use derive_more::derive::Display;
use derive_setters::Setters;
use merge::Merge;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::{Display as StrumDisplay, EnumString};

use crate::{
    Compact, Error, EventContext, MaxTokens, Model, ModelId, ProviderId, Result, SystemContext,
    Temperature, Template, ToolDefinition, ToolName, TopK, TopP,
};

// Unique identifier for an agent
#[derive(Debug, Display, Eq, PartialEq, Hash, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct AgentId(Cow<'static, str>);

impl From<&str> for AgentId {
    fn from(value: &str) -> Self {
        AgentId(Cow::Owned(value.to_string()))
    }
}

impl AgentId {
    // Creates a new agent ID from a string-like value
    pub fn new(id: impl ToString) -> Self {
        Self(Cow::Owned(id.to_string()))
    }

    // Returns the agent ID as a string reference
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }

    pub const FORGE: AgentId = AgentId(Cow::Borrowed("forge"));
    pub const MUSE: AgentId = AgentId(Cow::Borrowed("muse"));
    pub const SAGE: AgentId = AgentId(Cow::Borrowed("sage"));
}

impl Default for AgentId {
    fn default() -> Self {
        AgentId::FORGE
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, Merge, Setters, JsonSchema, PartialEq)]
#[setters(strip_option)]
#[merge(strategy = merge::option::overwrite_none)]
pub struct ReasoningConfig {
    /// Controls the effort level of the agent's reasoning
    /// supported by openrouter and forge provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,

    /// Controls how many tokens the model can spend thinking.
    /// supported by openrouter, anthropic and forge provider
    /// should be greater then 1024 but less than overall max_tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,

    /// Model thinks deeply, but the reasoning is hidden from you.
    /// supported by openrouter and forge provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,

    /// Enables reasoning at the "medium" effort level with no exclusions.
    /// supported by openrouter, anthropic and forge provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, StrumDisplay, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum Effort {
    /// No reasoning; skips the thinking step entirely.
    None,
    /// Minimal reasoning; fastest and cheapest.
    Minimal,
    /// Low reasoning effort.
    Low,
    /// Medium reasoning effort; the default for most providers.
    Medium,
    /// High reasoning effort.
    High,
    /// Extra-high reasoning effort (OpenAI / OpenRouter).
    XHigh,
    /// Maximum reasoning effort; only available on select Anthropic models.
    Max,
}

/// Estimates the token count from a string representation
/// This is a simple estimation that should be replaced with a more accurate
/// tokenizer.
///
/// Uses chars/3 (not the prose-calibrated chars/4) because forge contexts
/// are dominated by code, JSON, and markdown — content that tokenizes denser.
pub fn estimate_token_count(count: usize) -> usize {
    count / 3
}

/// Runtime agent representation with required model and provider
#[derive(Debug, Clone, PartialEq, Setters, Serialize, Deserialize, JsonSchema)]
#[setters(strip_option, into)]
pub struct Agent {
    /// Unique identifier for the agent
    pub id: AgentId,

    /// Human-readable title for the agent
    pub title: Option<String>,

    /// Human-readable description of the agent's purpose
    pub description: Option<String>,

    /// Flag to enable/disable tool support for this agent.
    pub tool_supported: Option<bool>,

    /// Path to the agent definition file, if loaded from a file
    pub path: Option<String>,

    /// Required provider for the agent
    pub provider: ProviderId,

    /// Required language model ID to be used by this agent
    pub model: ModelId,

    /// Template for the system prompt provided to the agent
    pub system_prompt: Option<Template<SystemContext>>,

    /// Template for the user prompt provided to the agent
    pub user_prompt: Option<Template<EventContext>>,

    /// Tools that the agent can use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolName>>,

    /// Maximum number of turns the agent can take
    pub max_turns: Option<u64>,

    /// Configuration for automatic context compaction
    pub compact: Compact,

    /// A set of custom rules that the agent should follow
    pub custom_rules: Option<String>,

    /// Temperature used for agent
    pub temperature: Option<Temperature>,

    /// Top-p (nucleus sampling) used for agent
    pub top_p: Option<TopP>,

    /// Top-k used for agent
    pub top_k: Option<TopK>,

    /// Maximum number of tokens the model can generate
    pub max_tokens: Option<MaxTokens>,

    /// Reasoning configuration for the agent.
    pub reasoning: Option<ReasoningConfig>,

    /// Maximum number of times a tool can fail before sending the response back
    pub max_tool_failure_per_turn: Option<usize>,

    /// Maximum number of requests that can be made in a single turn
    pub max_requests_per_turn: Option<usize>,
}

/// Lightweight metadata about an agent, used for listing without requiring a
/// configured provider or model.
#[derive(Debug, Default, Clone, PartialEq, Setters, Serialize, Deserialize, JsonSchema)]
#[setters(strip_option, into)]
pub struct AgentInfo {
    /// Unique identifier for the agent
    pub id: AgentId,

    /// Human-readable title for the agent
    pub title: Option<String>,

    /// Human-readable description of the agent's purpose
    pub description: Option<String>,
}

impl Agent {
    /// Create a new Agent with required provider and model
    pub fn new(id: impl Into<AgentId>, provider: ProviderId, model: ModelId) -> Self {
        Self {
            id: id.into(),
            title: Default::default(),
            description: Default::default(),
            provider,
            model,
            tool_supported: Default::default(),
            system_prompt: Default::default(),
            user_prompt: Default::default(),
            tools: Default::default(),
            max_turns: Default::default(),
            compact: Compact::default(),
            custom_rules: Default::default(),
            temperature: Default::default(),
            top_p: Default::default(),
            top_k: Default::default(),
            max_tokens: Default::default(),
            reasoning: Default::default(),
            max_tool_failure_per_turn: Default::default(),
            max_requests_per_turn: Default::default(),
            path: Default::default(),
        }
    }

    /// Creates a ToolDefinition from this agent
    ///
    /// # Errors
    ///
    /// Returns an error if the agent has no description
    pub fn tool_definition(&self) -> Result<ToolDefinition> {
        if self.description.is_none() || self.description.as_ref().is_none_or(|d| d.is_empty()) {
            return Err(Error::MissingAgentDescription(self.id.clone()));
        }
        Ok(ToolDefinition::new(self.id.as_str().to_string())
            .description(self.description.clone().unwrap()))
    }

    /// Sets the model in compaction config if not already set
    pub fn set_compact_model_if_none(mut self) -> Self {
        if self.compact.model.is_none() {
            self.compact.model = Some(self.model.clone());
        }
        self
    }

    /// Applies a safe `token_threshold` by taking the minimum of an absolute
    /// token cap and a percentage-based context-window cap.
    ///
    /// The absolute cap comes from `compact.token_threshold`, or falls back to
    /// a default of 800,000 tokens — essentially a worst-case ceiling that
    /// only matters when the model has no reported context length. The
    /// context-window cap comes from `compact.token_threshold_percentage`,
    /// or falls back to 90% of the selected model's *usable* context window
    /// (model context length minus an output reserve of 8K tokens to preserve
    /// headroom for the completion and any follow-up tool output). If model
    /// metadata is unavailable, a default 200K context window is used (the
    /// current floor for every shipping Claude model). The lower of the two
    /// values is used.
    ///
    /// # Arguments
    /// * `selected_model` - The model that will be used for this agent
    ///
    /// # Returns
    /// The agent with a safe token_threshold configured
    pub fn compaction_threshold(mut self, selected_model: Option<&Model>) -> Self {
        const DEFAULT_CONTEXT_WINDOW: usize = 200_000;
        // High enough to never bind for any current model (Opus-1M peaks at
        // ~892k usable). Acts purely as a sanity ceiling for the case where
        // a provider misreports a huge context length.
        const DEFAULT_TOKEN_THRESHOLD: usize = 5_000_000;
        const DEFAULT_CONTEXT_WINDOW_PERCENTAGE: f64 = 0.9;
        // Tokens reserved for the upcoming completion (and any tool output it
        // streams back) before the percentage is applied. Matches Claude
        // Code's "subtract output budget first" pattern.
        const OUTPUT_RESERVE_TOKENS: usize = 8_000;

        let context_window = selected_model
            .and_then(|model| model.context_length)
            .and_then(|context_window| usize::try_from(context_window).ok())
            .unwrap_or(DEFAULT_CONTEXT_WINDOW);

        let usable_context_window = context_window.saturating_sub(OUTPUT_RESERVE_TOKENS);

        let configured_threshold = self
            .compact
            .token_threshold
            .unwrap_or(DEFAULT_TOKEN_THRESHOLD);
        let context_window_percentage = self
            .compact
            .token_threshold_percentage
            .unwrap_or(DEFAULT_CONTEXT_WINDOW_PERCENTAGE);
        let context_window_threshold =
            ((usable_context_window as f64) * context_window_percentage).floor() as usize;

        self.compact.token_threshold = Some(configured_threshold.min(context_window_threshold));

        self
    }

    /// Gets the tool ordering for this agent, derived from the tools list
    pub fn tool_order(&self) -> crate::ToolOrder {
        self.tools
            .as_ref()
            .map(|tools| crate::ToolOrder::from_tool_list(tools))
            .unwrap_or_default()
    }
}

impl From<Agent> for ToolDefinition {
    fn from(value: Agent) -> Self {
        let description = value.description.unwrap_or_default();
        let name = ToolName::new(value.id);
        ToolDefinition {
            name,
            description,
            input_schema: schemars::schema_for!(crate::AgentInput),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{InputModality, Model};

    fn model_fixture(id: &str, context_length: Option<u64>) -> Model {
        Model {
            id: ModelId::new(id),
            name: Some(id.to_string()),
            description: None,
            context_length,
            tools_supported: Some(true),
            supports_parallel_tool_calls: Some(true),
            supports_reasoning: Some(true),
            input_modalities: vec![InputModality::Text],
        }
    }

    #[test]
    fn test_cap_compact_token_threshold_by_context_window_caps_when_threshold_exceeds_context_window()
     {
        let fixture = Agent::new(
            AgentId::new("test"),
            ProviderId::OPENAI,
            ModelId::new("selected-model"),
        )
        .compact(Compact::new().token_threshold(100_000_usize));

        let selected_model = model_fixture("selected-model", Some(80_000));

        let actual = fixture.compaction_threshold(Some(&selected_model));
        // usable = 80_000 - 8_000 = 72_000; 90% × 72_000 = 64_800
        let expected = Some(64_800);

        assert_eq!(actual.compact.token_threshold, expected);
    }

    #[test]
    fn test_cap_compact_token_threshold_caps_to_safe_margin_when_within_context_window() {
        // Configured 60K is below the 90%-of-usable cap (64.8K), so it stays.
        let fixture = Agent::new(
            AgentId::new("test"),
            ProviderId::OPENAI,
            ModelId::new("selected-model"),
        )
        .compact(Compact::new().token_threshold(60_000_usize));

        let selected_model = model_fixture("selected-model", Some(80_000));

        let actual = fixture.compaction_threshold(Some(&selected_model));
        let expected = Some(60_000);

        assert_eq!(actual.compact.token_threshold, expected);
    }

    #[test]
    fn test_compaction_threshold_uses_configured_context_window_percentage_cap() {
        let fixture = Agent::new(
            AgentId::new("test"),
            ProviderId::OPENAI,
            ModelId::new("selected-model"),
        )
        .compact(
            Compact::new()
                .token_threshold(100_000_usize)
                .token_threshold_percentage(0.5_f64),
        );

        let selected_model = model_fixture("selected-model", Some(80_000));

        let actual = fixture.compaction_threshold(Some(&selected_model));
        // usable = 72_000; 50% × 72_000 = 36_000
        let expected = Some(36_000);

        assert_eq!(actual.compact.token_threshold, expected);
    }

    #[test]
    fn test_compaction_threshold_uses_hardcoded_cap_when_context_window_cap_is_higher() {
        let fixture = Agent::new(
            AgentId::new("test"),
            ProviderId::OPENAI,
            ModelId::new("selected-model"),
        );

        let selected_model = model_fixture("selected-model", Some(200_000));

        let actual = fixture.compaction_threshold(Some(&selected_model));
        // No configured threshold → DEFAULT_TOKEN_THRESHOLD (800K).
        // usable = 192_000; 90% × 192_000 = 172_800 (this wins).
        let expected = Some(172_800);

        assert_eq!(actual.compact.token_threshold, expected);
    }

    #[test]
    fn test_compaction_threshold_opus_1m_uses_percentage() {
        // 1M-context Opus variant: percentage drives a much higher threshold,
        // so the user actually gets to use the window.
        let fixture = Agent::new(
            AgentId::new("test"),
            ProviderId::ANTHROPIC,
            ModelId::new("claude-opus-4-7"),
        );

        let selected_model = model_fixture("claude-opus-4-7", Some(1_000_000));

        let actual = fixture.compaction_threshold(Some(&selected_model));
        // usable = 992_000; 90% × 992_000 = 892_800
        let expected = Some(892_800);

        assert_eq!(actual.compact.token_threshold, expected);
    }

    #[test]
    fn test_cap_compact_token_threshold_uses_default_when_selected_model_is_missing() {
        // Without model info, falls back to DEFAULT_CONTEXT_WINDOW = 200K.
        let fixture = Agent::new(
            AgentId::new("test"),
            ProviderId::OPENAI,
            ModelId::new("selected-model"),
        )
        .compact(Compact::new().token_threshold(100_000_usize));

        let actual = fixture.compaction_threshold(None);
        // usable = 192_000; 90% × 192_000 = 172_800; min(100_000, 172_800) = 100_000
        let expected = Some(100_000);

        assert_eq!(actual.compact.token_threshold, expected);
    }

    /// compaction_threshold must set a non-None threshold even when the agent
    /// hasn't configured one — otherwise unbounded context growth happens.
    #[test]
    fn test_compaction_threshold_should_set_default_when_token_threshold_is_none() {
        let fixture = Agent::new(
            AgentId::new("test"),
            ProviderId::OPENAI,
            ModelId::new("gpt-5.3-codex-spark"),
        );
        assert_eq!(fixture.compact.token_threshold, None);

        let selected_model = model_fixture("gpt-5.3-codex-spark", Some(128_000));

        let actual = fixture.compaction_threshold(Some(&selected_model));

        // usable = 120_000; 90% × 120_000 = 108_000
        let expected_threshold = Some(108_000);
        assert_eq!(
            actual.compact.token_threshold, expected_threshold,
            "compaction_threshold should set 90% of (context_window - 8K reserve) when no threshold is configured."
        );
    }

    /// codex-spark (128K window) keeps a configured 100K threshold because
    /// it sits below the 90%-of-usable cap (108K). 20K headroom (reserve + 10%
    /// percentage gap) is enough for typical tool outputs.
    #[test]
    fn test_compaction_threshold_keeps_configured_when_under_cap_for_codex_spark() {
        let fixture = Agent::new(
            AgentId::new("test"),
            ProviderId::OPENAI,
            ModelId::new("gpt-5.3-codex-spark"),
        )
        .compact(Compact::new().token_threshold(100_000_usize));

        let selected_model = model_fixture("gpt-5.3-codex-spark", Some(128_000));

        let actual = fixture.compaction_threshold(Some(&selected_model));

        let expected_threshold = Some(100_000);
        assert_eq!(
            actual.compact.token_threshold, expected_threshold,
            "Configured 100K sits under the 108K cap (90% of 120K usable), so it stays."
        );
    }

    /// BUG 3: Agent with no compact config and no model info should still work,
    /// but currently compaction_threshold does nothing and context grows
    /// unbounded.
    #[test]
    fn test_compaction_threshold_no_model_context_length_should_still_set_default() {
        // Agent with no compact config
        let fixture = Agent::new(
            AgentId::new("test"),
            ProviderId::OPENAI,
            ModelId::new("unknown-model"),
        );

        // Model with NO context_length info
        let selected_model = model_fixture("unknown-model", None);

        let actual = fixture.compaction_threshold(Some(&selected_model));

        // EXPECTED: Should set a reasonable default threshold (e.g., 64000 for 128K
        // default window) or at least set SOME threshold to prevent unbounded
        // growth ACTUAL BUG: Returns early with token_threshold still as None
        assert!(
            actual.compact.token_threshold.is_some(),
            "BUG: compaction_threshold should set a default threshold even when model context_length is unknown. \
             Currently returns early with None, causing unbounded context growth."
        );
    }
}
