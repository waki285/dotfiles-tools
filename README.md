# dotfiles-tools

Development tools for [waki285/dotfiles](https://github.com/waki285/dotfiles).

## Contents

| Directory | Description | Language |
|-----------|-------------|----------|
| [agent_hooks](agent_hooks/) | Hook system for AI coding agents (Claude Code, OpenCode) | Rust |
| [claude_statusline](claude_statusline/) | Claude Status hook renderer with powerline-style ANSI output | Rust |
| [permissions-gen](permissions-gen/) | Tool permission generator from centralized YAML | Go |

## Rust workspace

This repository is organized as a Cargo workspace rooted at `tools/Cargo.toml`.

Workspace members:

- `agent_hooks/core`
- `agent_hooks/claude`
- `agent_hooks/opencode`
- `claude_statusline`

Build all Rust members:

```bash
cargo build --workspace
```

## agent_hooks

A Rust-based hook system providing safety checks for AI coding agents:

- Block `rm` commands (suggest `trash` instead)
- Confirm destructive `find` commands
- Protect dangerous paths from rm/trash/mv
- Deny `#[allow(...)]` attributes in Rust files
- Detect package manager mismatches

See [agent_hooks/README.md](agent_hooks/README.md) for details.

## claude_statusline

`claude_statusline` reads Claude Status hook JSON from `stdin` and prints a powerline-style status line.

Displayed fields:

- Model (`model.display_name` or `model.id`)
- CWD (`workspace.current_dir` or `cwd`)
- Project directory when different from CWD (`workspace.project_dir`)
- Context usage percentage (`context_window.current_usage` / `context_window_size`)
- Claude version (`version`) aligned to the right

## permissions-gen

Generates tool-specific permission configs from a centralized YAML file (`.chezmoidata/permissions.yaml` in dotfiles).

Outputs:
- `dot_claude/settings.json.tmpl` (permissions block)
- `dot_codex/rules/default.rules`
- `dot_config/opencode/opencode.json`

## License

Apache License 2.0 - See [LICENSE](LICENSE) for details.
