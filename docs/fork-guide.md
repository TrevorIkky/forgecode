# ForgeCode Fork Guide

A reference for forking ForgeCode into a rebranded coding-agent CLI. Every claim below has a `file:line` citation so changes can be made directly.

## Architecture at a glance

```
crates/forge_main      ← binary "forge" (REPL + clap CLI + reedline editor + banner)
   ↓ forge_api          ← stable public surface (ForgeAPI::init)
   ↓ forge_app          ← orchestrator: chat → LLM → tool calls (orch.rs)
   ↓ forge_services     ← provider clients, agent registry, conversation svc
   ↓ forge_infra        ← HTTP, FS, git
   ↓ forge_domain       ← types: Agent, Conversation, ToolCall, Context…
   ↓ forge_repo         ← persistence + EMBEDDED built-in agents (forge/sage/muse .md)
   ↓ forge_tracker      ← PostHog telemetry (gated by FORGE_TRACKER env var)
```

The agent loop lives in `crates/forge_app/src/orch.rs` (`Orchestrator::run`, ~lines 31–185), invoked from `ForgeApp::chat` in `crates/forge_app/src/app.rs:60`. Tools come from `tool_registry.rs` (built-ins generated via `forge_tool_macros` from `.md` description files) plus MCP servers plus agent-as-tool subagents.

The input editor (`crates/forge_main/src/editor.rs`) is reedline-based — keybindings, completer, highlighter, bracketed paste — not a code editor. The visual surface (banner, streamed markdown, spinners, pickers) lives across `ui.rs` (5k lines), `stream_renderer.rs`, `prompt.rs`, `banner.rs`.

## Hardcoded brand identity — the fork change-list

### Binary & CLI

- `crates/forge_main/Cargo.toml:8` — `[[bin]] name = "forge"`
- `crates/forge_main/src/banner` — ASCII-art logo (the "Forge" block)
- `crates/forge_main/src/banner.rs:58` — `FORGE_BANNER` env var override
- `crates/forge_main/src/banner.rs:128,134` — "forge zsh setup" tip + `forgecode.dev/docs/zsh-support` link

### Config dirs (user-facing)

- `crates/forge_config/src/reader.rs:67–84` — resolves `FORGE_CONFIG` → `~/forge` (legacy) → `~/.forge`
- `crates/forge_domain/src/env.rs` — history `.forge_history`, MCP `.mcp.json`, project `.forge/agents/`
- `crates/forge_config/src/reader.rs:103` — reads `FORGE_*` env vars as config sources

### Network endpoints

These will silently break or phone to tailcallhq if left alone.

- `crates/forge_main/src/update.rs:90` — update-informer polls GitHub `tailcallhq/forgecode`
- `crates/forge_main/src/update.rs:16` — auto-update runs `curl -fsSL https://forgecode.dev/cli | sh`
- `crates/forge_config/src/config.rs:191` — workspace/semantic-search API default `https://api.forgecode.dev/api` (overridable via `FORGE_WORKSPACE_SERVER_URL`)
- `crates/forge_tracker/src/dispatch.rs:20` — `POSTHOG_API_SECRET` build-time env (defaults to `"dev"` → no-op)
- `crates/forge_tracker/src/dispatch.rs:30` — `FORGE_TRACKER` runtime gate
- `crates/forge_tracker/src/collect/posthog.rs:77` — `https://us.i.posthog.com/capture/`
- `crates/forge_infra/src/http.rs:200` — User-Agent advertises `https://forgecode.dev`
- `crates/forge_main/src/oauth_callback.rs:158,162` — OAuth success/failure HTML says "ForgeCode"
- `crates/forge_main/src/info.rs:647` — billing link `https://app.forgecode.dev/app/billing`

### Git identity

- `crates/forge_app/src/git_app.rs:401` — every AI commit uses `GIT_COMMITTER_NAME='ForgeCode' GIT_COMMITTER_EMAIL='noreply@forgecode.dev'`

### Built-in agents (embedded at compile time)

- `crates/forge_repo/src/agents/{forge,muse,sage}.md` — YAML front-matter + Handlebars body
- `crates/forge_repo/src/agent.rs:77–79` — `include_str!()` embeds them
- Each agent's persona name is in the markdown body too (e.g., `forge.md:35` "You are Forge, an expert software engineering assistant…")
- Override path precedence: `./.forge/agents/*.md` > `~/.forge/agents/*.md` > embedded

### ZSH plugin (the `:` prefix)

- `shell-plugin/` directory (pure shell, no Rust)
- `shell-plugin/lib/bindings.zsh:21` — intercepts lines starting with `:`
- `crates/forge_main/src/zsh/` — Rust side that installs/refreshes the plugin

## Minimum viable rebrand to `myforge`

Renames that MUST happen (user-facing):

1. `crates/forge_main/Cargo.toml:8` → binary name
2. Replace `crates/forge_main/src/banner` ASCII art
3. Search/replace `FORGE_` env var prefixes across the workspace (config, banner, tracker, env.rs) — pick a new prefix or keep `FORGE_` if you don't mind the leakage
4. `crates/forge_config/src/reader.rs:73,83` → config dir name (`~/forge` / `~/.forge`)
5. `crates/forge_app/src/git_app.rs:401` → committer name/email
6. `crates/forge_main/src/update.rs:90,16` → either your repo + install URL, or set `UpdateFrequency::Never` default and delete the auto-update flow
7. Rewrite `crates/forge_repo/src/agents/{forge,muse,sage}.md` — both the YAML `id` and the persona text in the body, or replace with your own agents
8. `crates/forge_main/src/oauth_callback.rs:158,162` — callback page HTML
9. `shell-plugin/` — references to the `forge` binary name

Safe to leave as `forge_*` internally:

- All `crates/forge_*` workspace crate names — never shown to users
- Internal type names (`ForgeAPI`, `ForgeApp`, `ForgeHighlighter`, etc.)
- Cargo workspace dep names

### Tripwires

- `POSTHOG_API_SECRET` is build-time — the fork won't emit telemetry unless a key is injected, but the code path is still compiled in. To strip it entirely, disable the `forge_tracker` dispatch in `crates/forge_main/src/lib.rs:37`.
- `FORGE_BANNER` env var lets users override the banner at runtime — useful for distribution.
- Workspace API (`api.forgecode.dev`) only triggers if the user runs `:sync` — a fork can ignore it until semantic search is needed.
- The README's `curl -fsSL forgecode.dev/cli | sh` install pipeline doesn't exist in code; it's a separate hosted shell script — a fork needs its own install endpoint.

## Where to put your own stamp (quick wins, low risk)

1. **Banner** — replace `crates/forge_main/src/banner` with your ASCII logo
2. **Agent personas** — edit `crates/forge_repo/src/agents/forge.md` body + YAML `id`; that's the main "voice" of the tool
3. **Default model/provider** — `crates/forge_config/.forge.toml` (embedded defaults via `include_str!` at `reader.rs:98`)
4. **Slash/colon commands** — `crates/forge_main/src/model.rs` (1673 lines — the `ForgeCommandManager`) registers commands; new commands can also be added as YAML in `.forge/commands/`
5. **Tools** — each tool has a `.md` description file under `crates/forge_app/src/tools/` (via `forge_tool_macros`); add new ones without touching plumbing

## Key file reference

| Concern | File(s) |
|---|---|
| Binary name | `crates/forge_main/Cargo.toml:8` |
| Banner art | `crates/forge_main/src/banner` |
| Banner rendering & override | `crates/forge_main/src/banner.rs` |
| Git committer identity | `crates/forge_app/src/git_app.rs:401` |
| Update check (GitHub repo) | `crates/forge_main/src/update.rs:90` |
| Install script URL | `crates/forge_main/src/update.rs:16` |
| Workspace API default | `crates/forge_config/src/config.rs:191` |
| OAuth callback HTML | `crates/forge_main/src/oauth_callback.rs:158,162` |
| Config base path resolution | `crates/forge_config/src/reader.rs:67–84` |
| Embedded config defaults | `crates/forge_config/.forge.toml` (via `reader.rs:98`) |
| Built-in agents | `crates/forge_repo/src/agents/{forge,muse,sage}.md` |
| Agent loading + precedence | `crates/forge_repo/src/agent.rs:44–111` |
| Orchestrator loop | `crates/forge_app/src/orch.rs:31–185` |
| Tool registry | `crates/forge_app/src/tool_registry.rs` |
| Telemetry gating | `crates/forge_tracker/src/dispatch.rs:20,30` |
| PostHog endpoint | `crates/forge_tracker/src/collect/posthog.rs:77` |
| Environment struct | `crates/forge_domain/src/env.rs` |
| Input editor (reedline) | `crates/forge_main/src/editor.rs` |
| UI / TUI | `crates/forge_main/src/ui.rs` (5k lines) |
