use async_trait::async_trait;
use forge_domain::{
    Agent, Conversation, EndPayload, Environment, EventData, EventHandle, RequestPayload,
};
use tracing::{debug, info};

use crate::compact::Compactor;

/// Hook handler that performs context compaction when needed.
///
/// Registered on **both** `on_request` (so a mid-turn context blowup is
/// caught before the next API call) and `on_end` (turn-boundary cleanup).
/// The combination of (a) hysteresis in `Compactor::derive_target_retention`
/// landing post-compaction at ~60% of threshold and (b) idempotent
/// behavior when context is already below threshold means firing on every
/// request is safe — the handler is a no-op when no work is needed and
/// the runaway loop from issue #3076 can't recur.
#[derive(Clone)]
pub struct CompactionHandler {
    agent: Agent,
    environment: Environment,
}

impl CompactionHandler {
    /// Creates a new compaction handler
    ///
    /// # Arguments
    /// * `agent` - The agent configuration containing compaction settings
    /// * `environment` - The environment configuration
    pub fn new(agent: Agent, environment: Environment) -> Self {
        Self { agent, environment }
    }

    /// Shared compaction logic invoked from both lifecycle hooks.
    async fn maybe_compact(&self, conversation: &mut Conversation) -> anyhow::Result<()> {
        if let Some(context) = &conversation.context {
            let token_count = context.token_count();
            if self.agent.compact.should_compact(context, *token_count) {
                info!(
                    agent_id = %self.agent.id,
                    tokens = *token_count,
                    "Compaction triggered by hook"
                );
                let compacted =
                    Compactor::new(self.agent.compact.clone(), self.environment.clone())
                        .compact(context.clone(), false)?;
                conversation.context = Some(compacted);
            } else {
                debug!(agent_id = %self.agent.id, "Compaction not needed");
            }
        }
        Ok(())
    }
}

#[async_trait]
impl EventHandle<EventData<EndPayload>> for CompactionHandler {
    async fn handle(
        &self,
        _event: &EventData<EndPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        self.maybe_compact(conversation).await
    }
}

#[async_trait]
impl EventHandle<EventData<RequestPayload>> for CompactionHandler {
    async fn handle(
        &self,
        _event: &EventData<RequestPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        self.maybe_compact(conversation).await
    }
}
