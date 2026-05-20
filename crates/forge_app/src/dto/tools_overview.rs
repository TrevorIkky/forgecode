use derive_setters::Setters;
use forge_domain::{McpServers, ToolDefinition};
use serde::{Deserialize, Serialize};

/// A comprehensive view of all tools available in the environment,
/// categorized by their source type for easier navigation and understanding.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Setters)]
#[setters(into, strip_option)]
pub struct ToolsOverview {
    /// System tools provided by the Forge environment
    pub system: Vec<ToolDefinition>,
    /// Tools provided by registered agents
    pub agents: Vec<ToolDefinition>,
    /// Tools provided by MCP servers, grouped by server name
    pub mcp: McpServers,
}

impl ToolsOverview {
    /// Create a new empty ToolsOverview
    pub fn new() -> Self {
        ToolsOverview::default()
    }

    // Creates a flat list of all tool definitions in a stable order.
    //
    // MCP iteration is doubly non-deterministic in the source (a HashMap of
    // servers, each rebuilt from a HashMap of tools). We sort both layers:
    // by server name across the outer map, and by tool name within each
    // server. Without this the prefix cache on Anthropic invalidates on
    // every request because the byte representation of the tools array
    // shifts. See issue #3076.
    pub fn as_vec(&self) -> Vec<&ToolDefinition> {
        let mut tools = Vec::new();
        tools.extend(&self.system);
        tools.extend(&self.agents);
        let mut mcp_servers: Vec<_> = self.mcp.get_servers().iter().collect();
        mcp_servers.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (_, server_tools) in mcp_servers {
            let mut sorted: Vec<&ToolDefinition> = server_tools.iter().collect();
            sorted.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
            tools.extend(sorted);
        }
        tools
    }
}

impl From<ToolsOverview> for Vec<ToolDefinition> {
    fn from(value: ToolsOverview) -> Self {
        value.as_vec().into_iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use forge_domain::{McpServers, ServerName, ToolDefinition, ToolName};
    use pretty_assertions::assert_eq;

    use super::*;

    fn td(name: &str) -> ToolDefinition {
        ToolDefinition::new(ToolName::new(name))
    }

    /// MCP servers come back from the registry in a HashMap, which has no
    /// stable iteration order. `as_vec` must sort by server name so two
    /// successive calls produce identical Vecs — otherwise Anthropic's
    /// prefix cache invalidates on every request. See issue #3076.
    #[test]
    fn test_as_vec_mcp_order_is_stable() {
        let mut servers: HashMap<ServerName, Vec<ToolDefinition>> = HashMap::new();
        // Multi-tool servers in arbitrary intra-server order to exercise
        // the inner sort.
        servers.insert(
            ServerName::from("zebra".to_string()),
            vec![td("zebra.y"), td("zebra.a"), td("zebra.m")],
        );
        servers.insert(
            ServerName::from("alpha".to_string()),
            vec![td("alpha.z"), td("alpha.a")],
        );
        servers.insert(
            ServerName::from("mango".to_string()),
            vec![td("mango.one")],
        );

        let overview = ToolsOverview::new()
            .system(vec![td("read")])
            .mcp(McpServers::new(servers.clone(), HashMap::new()));

        // Call as_vec many times — order must be stable across calls.
        let mut prev: Option<Vec<String>> = None;
        for _ in 0..32 {
            let names: Vec<String> = overview
                .as_vec()
                .into_iter()
                .map(|t| t.name.to_string())
                .collect();

            if let Some(prev) = &prev {
                assert_eq!(&names, prev, "MCP iteration order must be deterministic");
            }
            prev = Some(names);
        }

        // Verify the sort: servers alpha, then tools alpha within each
        // server.
        let names: Vec<String> = overview
            .as_vec()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "read".to_string(),
                "alpha.a".to_string(),
                "alpha.z".to_string(),
                "mango.one".to_string(),
                "zebra.a".to_string(),
                "zebra.m".to_string(),
                "zebra.y".to_string(),
            ]
        );
    }
}
