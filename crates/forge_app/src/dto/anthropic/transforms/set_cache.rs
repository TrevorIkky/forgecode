use forge_domain::Transformer;

use crate::dto::anthropic::{CacheControl, Request};

/// Transformer that places Anthropic prompt-cache breakpoints under the
/// 4-block-per-request limit:
/// - Last tool definition (anchors the tools-only prefix)
/// - Last system message (anchors tools + system prefix; one marker covers
///   every preceding system block via prefix caching)
/// - Last conversation message (rolling marker for the live tail)
/// - Falls back to the first conversation message when there is no system
///   prompt, so single-turn requests still establish a reusable prefix
///
/// Anthropic prefix-caches up to and including each marked block, in
/// canonical order tools → system → messages. Maximum 4 breakpoints per
/// request — exceeding it returns a 400. The earlier version marked
/// **every** system message individually, which pushed multi-system-prompt
/// configs (e.g., agent prompt + workflow context + skills index) past the
/// limit once the tools breakpoint was added.
pub struct SetCache;

impl Transformer for SetCache {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        let len = request.get_messages().len();
        let sys_len = request.system.as_ref().map_or(0, |msgs| msgs.len());
        let tools_len = request.tools.len();

        if len == 0 && sys_len == 0 && tools_len == 0 {
            return request;
        }

        let has_system_prompt = sys_len > 0;

        // Tools: mark only the last. Caches the entire tools block.
        // NOTE: Anthropic silently ignores cache_control when the cumulative
        // prefix is below the model's minimum (4,096 tokens for Opus 4.7),
        // so very small tool sets won't actually cache — no error, just no
        // hit.
        for (idx, tool) in request.tools.iter_mut().enumerate() {
            tool.cache_control = if idx + 1 == tools_len {
                Some(CacheControl::one_hour())
            } else {
                None
            };
        }

        // System: mark only the last. One breakpoint at the end of the
        // system array prefix-caches every system block before it.
        if let Some(system_messages) = request.system.as_mut() {
            let last_idx = system_messages.len().saturating_sub(1);
            for (idx, message) in system_messages.iter_mut().enumerate() {
                *message = std::mem::take(message).cached(idx == last_idx);
            }
        }

        // Messages: clear all, then mark the last (rolling). When there's
        // no system prompt, also mark the first so single-turn requests
        // still pin a stable prefix.
        for message in request.get_messages_mut().iter_mut() {
            *message = std::mem::take(message).cached(false);
        }

        if !has_system_prompt
            && len > 0
            && let Some(first_message) = request.get_messages_mut().first_mut()
        {
            *first_message = std::mem::take(first_message).cached(true);
        }

        if let Some(message) = request.get_messages_mut().last_mut() {
            *message = std::mem::take(message).cached(true);
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use forge_domain::{Context, ContextMessage, ModelId, Role, TextMessage};
    use pretty_assertions::assert_eq;

    use super::*;

    fn create_test_context_with_system(
        system_messages: &str,
        conversation_messages: &str,
    ) -> String {
        let mut messages = Vec::new();

        // Add system messages to the regular messages array for Anthropic format
        for c in system_messages.chars() {
            match c {
                's' => messages.push(
                    ContextMessage::Text(TextMessage::new(Role::System, c.to_string())).into(),
                ),
                _ => panic!("Invalid character in system message: {}", c),
            }
        }

        // Add conversation messages
        for c in conversation_messages.chars() {
            match c {
                'u' => messages.push(
                    ContextMessage::Text(
                        TextMessage::new(Role::User, c.to_string())
                            .model(ModelId::new("claude-3-5-sonnet-20241022")),
                    )
                    .into(),
                ),
                'a' => messages.push(
                    ContextMessage::Text(TextMessage::new(Role::Assistant, c.to_string())).into(),
                ),
                _ => panic!("Invalid character in conversation message: {}", c),
            }
        }

        let context = Context {
            conversation_id: None,
            messages,
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = Request::try_from(context).expect("Failed to convert context to request");
        let mut transformer = SetCache;
        let request = transformer.transform(request);

        let mut output = String::new();

        // The DSL renders "[ss" when the system block is cached. Under the
        // new rule only the LAST system message carries the marker, but it
        // covers the entire system prefix — so we check `last`, not
        // `first`.
        let system_cached = request
            .system
            .as_ref()
            .and_then(|sys| sys.last())
            .map(|msg| msg.is_cached())
            .unwrap_or(false);

        if system_cached {
            output.push('[');
        }
        output.push_str(system_messages);

        // Check which regular messages are cached
        let cached_indices = request
            .get_messages()
            .iter()
            .enumerate()
            .filter(|(_, m)| m.is_cached())
            .map(|(i, _)| i)
            .collect::<HashSet<usize>>();

        for (i, c) in conversation_messages.chars().enumerate() {
            if cached_indices.contains(&i) {
                output.push('[');
            }
            output.push(c);
        }

        output
    }

    fn create_test_context(message: impl ToString) -> String {
        create_test_context_with_system("", &message.to_string())
    }

    #[test]
    fn test_single_message() {
        let actual = create_test_context("u");
        let expected = "[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_two_messages() {
        let actual = create_test_context("ua");
        let expected = "[u[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_three_messages_cache_first_and_last_only() {
        let actual = create_test_context("uau");
        let expected = "[ua[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_four_messages_cache_first_and_last_only() {
        let actual = create_test_context("uaua");
        let expected = "[uau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_five_messages_cache_first_and_last_only() {
        let actual = create_test_context("uauau");
        let expected = "[uaua[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_longer_conversation_caches_first_and_last_only() {
        let actual = create_test_context("uauauauaua");
        let expected = "[uauauauau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_with_system_message_single_conversation_message() {
        let actual = create_test_context_with_system("s", "u");
        let expected = "[s[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_with_system_message_multiple_conversation_messages() {
        let actual = create_test_context_with_system("ss", "uaua");
        let expected = "[ssuau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_with_system_message_long_conversation() {
        let actual = create_test_context_with_system("s", "uauauauaua");
        let expected = "[suauauauau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_only_system_message() {
        let actual = create_test_context_with_system("s", "");
        let expected = "[s";
        assert_eq!(actual, expected);
    }

    /// Wire-shape snapshot: realistic system + tools + multi-turn
    /// conversation should produce a request with cache_control breakpoints
    /// at the documented positions:
    ///   1. last tool
    ///   2. every system message
    ///   3. last conversation message (rolling)
    /// That's 3 of Anthropic's 4 max breakpoints.
    #[test]
    fn test_full_request_cache_layout() {
        use forge_domain::{ToolDefinition, ToolName};

        let context = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::Text(TextMessage::new(Role::System, "you are an agent")).into(),
                ContextMessage::Text(TextMessage::new(Role::System, "more rules")).into(),
                ContextMessage::Text(
                    TextMessage::new(Role::User, "first user")
                        .model(ModelId::new("claude-3-5-sonnet-20241022")),
                )
                .into(),
                ContextMessage::Text(TextMessage::new(Role::Assistant, "first reply")).into(),
                ContextMessage::Text(
                    TextMessage::new(Role::User, "second user")
                        .model(ModelId::new("claude-3-5-sonnet-20241022")),
                )
                .into(),
            ],
            tools: vec![
                ToolDefinition::new(ToolName::new("read")).description("read a file"),
                ToolDefinition::new(ToolName::new("write")).description("write a file"),
                ToolDefinition::new(ToolName::new("shell")).description("run shell command"),
            ],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = Request::try_from(context).expect("convert");
        let request = SetCache.transform(request);

        // Tools: only last cached.
        let tool_cache: Vec<bool> = request.tools.iter().map(|t| t.is_cached()).collect();
        assert_eq!(tool_cache, vec![false, false, true]);

        // System: only the LAST is cached (Anthropic prefix-caches all
        // earlier blocks).
        let system_cache: Vec<bool> = request
            .system
            .as_ref()
            .unwrap()
            .iter()
            .map(|s| s.is_cached())
            .collect();
        assert_eq!(system_cache, vec![false, true]);

        // Conversation messages: only the LAST has a cache marker.
        let msg_cache: Vec<bool> = request
            .get_messages()
            .iter()
            .map(|m| m.is_cached())
            .collect();
        assert_eq!(msg_cache, vec![false, false, true]);
    }

    /// Determinism check: the same SetCache pass on equivalent inputs must
    /// produce a byte-identical wire request, or the prefix cache will miss.
    #[test]
    fn test_set_cache_is_deterministic() {
        use forge_domain::{ToolDefinition, ToolName};

        fn build_request() -> Request {
            let context = Context {
                conversation_id: None,
                messages: vec![ContextMessage::Text(
                    TextMessage::new(Role::User, "hi")
                        .model(ModelId::new("claude-3-5-sonnet-20241022")),
                )
                .into()],
                tools: vec![
                    ToolDefinition::new(ToolName::new("a")).description("A"),
                    ToolDefinition::new(ToolName::new("b")).description("B"),
                ],
                tool_choice: None,
                max_tokens: None,
                temperature: None,
                top_p: None,
                top_k: None,
                reasoning: None,
                stream: None,
                response_format: None,
                initiator: None,
            };
            let request = Request::try_from(context).expect("convert");
            SetCache.transform(request)
        }

        let a = serde_json::to_string(&build_request()).unwrap();
        let b = serde_json::to_string(&build_request()).unwrap();
        assert_eq!(a, b, "SetCache must produce byte-identical requests");
    }

    #[test]
    fn test_caches_last_tool_definition() {
        use forge_domain::{ToolDefinition, ToolName};

        let tool_a = ToolDefinition::new(ToolName::new("a"))
            .description("Tool A")
            .input_schema(schemars::schema_for!(()));
        let tool_b = ToolDefinition::new(ToolName::new("b"))
            .description("Tool B")
            .input_schema(schemars::schema_for!(()));

        let fixture = Context {
            conversation_id: None,
            messages: vec![ContextMessage::Text(
                TextMessage::new(Role::User, "u")
                    .model(ModelId::new("claude-3-5-sonnet-20241022")),
            )
            .into()],
            tools: vec![tool_a, tool_b],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = Request::try_from(fixture).expect("Failed to convert context to request");
        let mut transformer = SetCache;
        let request = transformer.transform(request);

        let cached = request
            .tools
            .iter()
            .map(|tool| tool.is_cached())
            .collect::<Vec<_>>();
        assert_eq!(cached, vec![false, true]);
    }

    /// With multiple system messages, only the LAST gets a cache breakpoint.
    /// Anthropic prefix-caches everything up to and including that marker,
    /// so earlier system messages are covered without their own breakpoints.
    /// Marking each one would burn breakpoint slots and could push the
    /// request over the 4-block limit.
    #[test]
    fn test_only_last_system_message_cached() {
        let fixture = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::Text(TextMessage::new(Role::System, "first")).into(),
                ContextMessage::Text(TextMessage::new(Role::System, "second")).into(),
                ContextMessage::Text(TextMessage::new(Role::System, "third")).into(),
                ContextMessage::Text(
                    TextMessage::new(Role::User, "user")
                        .model(ModelId::new("claude-3-5-sonnet-20241022")),
                )
                .into(),
            ],
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = Request::try_from(fixture).expect("Failed to convert context to request");
        let mut transformer = SetCache;
        let request = transformer.transform(request);

        let actual = request
            .system
            .as_ref()
            .unwrap()
            .iter()
            .map(|message| message.is_cached())
            .collect::<Vec<_>>();
        assert_eq!(actual, vec![false, false, true]);
        assert!(request.get_messages()[0].is_cached());
    }

    /// Regression: Anthropic returns 400 when a request carries more than 4
    /// cache_control blocks. This test fabricates the worst realistic case
    /// (multiple system prompts + many tools + multi-message conversation)
    /// and counts every breakpoint emitted by SetCache. Must stay ≤ 4.
    #[test]
    fn test_breakpoint_count_within_anthropic_limit() {
        use forge_domain::{ToolDefinition, ToolName};

        let fixture = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::Text(TextMessage::new(Role::System, "s1")).into(),
                ContextMessage::Text(TextMessage::new(Role::System, "s2")).into(),
                ContextMessage::Text(TextMessage::new(Role::System, "s3")).into(),
                ContextMessage::Text(TextMessage::new(Role::System, "s4")).into(),
                ContextMessage::Text(
                    TextMessage::new(Role::User, "u1")
                        .model(ModelId::new("claude-3-5-sonnet-20241022")),
                )
                .into(),
                ContextMessage::Text(TextMessage::new(Role::Assistant, "a1")).into(),
                ContextMessage::Text(
                    TextMessage::new(Role::User, "u2")
                        .model(ModelId::new("claude-3-5-sonnet-20241022")),
                )
                .into(),
            ],
            tools: vec![
                ToolDefinition::new(ToolName::new("a")).description("A"),
                ToolDefinition::new(ToolName::new("b")).description("B"),
                ToolDefinition::new(ToolName::new("c")).description("C"),
                ToolDefinition::new(ToolName::new("d")).description("D"),
                ToolDefinition::new(ToolName::new("e")).description("E"),
            ],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = Request::try_from(fixture).expect("convert");
        let mut transformer = SetCache;
        let request = transformer.transform(request);

        let tool_breakpoints = request.tools.iter().filter(|t| t.is_cached()).count();
        let system_breakpoints = request
            .system
            .as_ref()
            .map(|s| s.iter().filter(|m| m.is_cached()).count())
            .unwrap_or(0);
        let message_breakpoints = request
            .get_messages()
            .iter()
            .filter(|m| m.is_cached())
            .count();

        let total = tool_breakpoints + system_breakpoints + message_breakpoints;
        assert!(
            total <= 4,
            "Anthropic rejects >4 cache_control blocks (got {}): tools={}, system={}, messages={}",
            total,
            tool_breakpoints,
            system_breakpoints,
            message_breakpoints
        );

        // And specifically: should be exactly 3 in this layout (last tool +
        // last system + last message). The first-message fallback is only
        // engaged when no system prompt is present.
        assert_eq!(total, 3);
    }
}
