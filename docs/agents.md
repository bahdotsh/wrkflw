# Using wrkflw from coding agents

```bash
# Machine-readable output: stdout is JSON only, logs go to stderr
wrkflw --format json validate
wrkflw --format json run .github/workflows/ci.yml --intent "check the new cache step"

# See what would run, and why a trigger fires, without running anything
wrkflw plan .github/workflows/ci.yml --event push --diff

# Every run is recorded
wrkflw runs list
wrkflw runs failure latest
wrkflw runs logs latest --step test --tail 50
wrkflw runs replay latest

# Resume a job at a step
wrkflw run .github/workflows/ci.yml --job test --from-step "Run tests"

# Run a workflow as a finished GitHub run did, and list where the results differ.
# Check out the commit that run tested first; a detached HEAD is fine, the
# run's branch is used.
wrkflw compare https://github.com/OWNER/REPO/actions/runs/123456789

# Check a push to another branch than the one checked out, or on a detached HEAD
wrkflw run --event push --branch release/2.0 --diff .github/workflows/ci.yml

# Check a tag push
wrkflw run --event push --tag v1.2.0 .github/workflows/release.yml

# Serve all of this over the Model Context Protocol
wrkflw mcp serve
```

Register the MCP server with every coding agent installed on this machine, or name the ones you want. Running it again changes nothing, and an entry that launches a different command is left alone unless you pass `--force`.

```bash
wrkflw mcp install            # every agent found
wrkflw mcp install codex      # just one
wrkflw mcp install --dry-run  # show what would change
```

| Agent | Name | What `install` changes |
|-------|------|------------------------|
| Claude Code | `claude-code` | Runs `claude mcp add --scope user` |
| Codex | `codex` | Runs `codex mcp add` |
| Cursor | `cursor` | `~/.cursor/mcp.json` |
| Windsurf | `windsurf` | `~/.codeium/windsurf/mcp_config.json` |
| VS Code (Copilot) | `vscode` | `mcp.json` in the VS Code user directory |
| Gemini CLI | `gemini` | `~/.gemini/settings.json` |
| opencode | `opencode` | `~/.config/opencode/opencode.json` |
| Claude Desktop | `claude-desktop` | `claude_desktop_config.json`, with the absolute path to `wrkflw`. Restart Claude Desktop afterwards |
| GitHub Copilot CLI | `copilot` | `~/.copilot/mcp-config.json`, or under `$COPILOT_HOME` |
| Grok Build | `grok` | Runs `grok mcp add` |
| Kiro CLI | `kiro` | `~/.kiro/settings/mcp.json`, or under `$KIRO_HOME`, with the absolute path to `wrkflw` |
| pi | `pi` | `~/.pi/agent/mcp.json`, or under `$PI_CODING_AGENT_DIR` |
| Zed | `zed` | `context_servers` in Zed's `settings.json` |

Without names, `install` sets up every agent whose CLI is on `PATH` or whose config directory exists. A named agent gets its config file even if it doesn't exist yet, so you can set up wrkflw before the agent's first run. Config files are edited in place, so comments, trailing commas and formatting are kept. For any other client, add a stdio server named `wrkflw` that runs `wrkflw mcp serve`.

Runs record which agent started them and from which session. The agent comes from `--agent` or `WRKFLW_AGENT`, or is detected from `AI_AGENT` and what Claude Code, Codex, Gemini CLI, opencode, Cursor and Augment set. Other agents, such as Kiro, Grok Build, pi and Copilot CLI, set nothing wrkflw can rely on, so set `WRKFLW_AGENT` in their environment. The session comes from `--session` or `WRKFLW_SESSION`, or from `CLAUDE_CODE_SESSION_ID` or `CODEX_THREAD_ID`. Filter on either with `wrkflw runs list --agent claude-code --session <id>`.

The TUI's History tab shows the same history, including which agent started each run and why. See [AGENTS.md](../AGENTS.md) for the output contract, and `skills/wrkflw` for a Claude Code skill.
