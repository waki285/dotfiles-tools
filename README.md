# dotfiles-tools

Development tools for [waki285/dotfiles](https://github.com/waki285/dotfiles).

## Contents

| Directory | Description | Language |
|-----------|-------------|----------|
| [agent_hooks](agent_hooks/) | Hook system for AI coding agents (Claude Code, OpenCode) | Rust |
| [permissions-gen](permissions-gen/) | Tool permission generator from centralized YAML | Go |

## agent_hooks

A Rust-based hook system providing safety checks for AI coding agents:

- Block `rm` commands (suggest `trash` instead)
- Confirm destructive `find` commands
- Protect dangerous paths from rm/trash/mv
- Deny `#[allow(...)]` attributes in Rust files
- Detect package manager mismatches

See [agent_hooks/README.md](agent_hooks/README.md) for details.

## permissions-gen

Generates tool-specific permission configs from a centralized YAML file (`.chezmoidata/permissions.yaml` in dotfiles).

Outputs:
- `dot_claude/settings.json.tmpl` (permissions block)
- `dot_codex/rules/default.rules`
- `dot_config/opencode/opencode.json`

## License

Apache License 2.0 - See [LICENSE](LICENSE) for details.
