# agent_hooks

A Rust-based hook system for AI coding agents that provides safety checks and restrictions across Claude Code, Codex, GitHub Copilot CLI, and OpenCode.

## Architecture

```text
agent_hooks/
├── core/           # Core library - pure check functions
├── cli/            # Unified CLI (`agent_hooks`) for Claude/Codex/Copilot
└── opencode/       # OpenCode NAPI bindings (agent_hooks_opencode)
```

## Features

### Bash command checks

- `block-rm`: Blocks `rm` commands and suggests `trash` instead
- `deny-destructive-find`: Denies destructive `find` commands such as `find -delete`
- `dangerous-paths`: Detects `rm`/`trash`/`mv` commands targeting configured paths
- `check-package-manager`: Detects package manager mismatches such as `npm` in a `pnpm-lock.yaml` repo
- `deny-nul-redirect`: Windows only. Denies redirects to `nul` and enforces `/dev/null`

### Rust edit checks

- `deny-rust-allow`: Denies adding `#[allow(...)]` or `#[expect(...)]` attributes to Rust files
- `expect`: With `deny-rust-allow`, allows `#[expect(...)]` while still denying `#[allow(...)]`
- `additional-context`: Appends a custom denial message

## Installation

Pre-built binaries are published on GitHub Releases. The dotfiles install scripts download the unified CLI plus the OpenCode `.node` file automatically.

### Manual installation

#### Unified CLI

```bash
# Download the binary for your platform
curl -fsSL -o ~/.local/bin/agent_hooks \
  https://github.com/waki285/dotfiles-tools/releases/download/agent_hooks-vX.Y.Z/agent_hooks-<platform>

chmod +x ~/.local/bin/agent_hooks
```

#### OpenCode plugin

```bash
curl -fsSL -o ~/.config/opencode/plugin/agent_hooks.node \
  https://github.com/waki285/dotfiles-tools/releases/download/agent_hooks-vX.Y.Z/agent_hooks_opencode-<platform>.node
```

## Usage

### Claude Code

Add to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "agent_hooks claude pre-tool-use --deny-rust-allow --expect"
          }
        ]
      },
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "agent_hooks claude pre-tool-use --check-package-manager --deny-destructive-find"
          }
        ]
      }
    ],
    "PermissionRequest": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "agent_hooks claude permission-request --block-rm"
          }
        ]
      }
    ]
  }
}
```

Examples:

```bash
echo '{"tool_name":"Bash","tool_input":{"command":"find . -name \"*.tmp\" -delete"}}' | \
  agent_hooks claude pre-tool-use --deny-destructive-find

echo '{"tool_name":"Edit","tool_input":{"file_path":"src/main.rs","new_string":"#[allow(dead_code)]"}}' | \
  agent_hooks claude pre-tool-use --deny-rust-allow --expect
```

### Codex

Enable hooks in `~/.codex/config.toml`:

```toml
[features]
codex_hooks = true
```

Then create `~/.codex/hooks.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "agent_hooks codex pre-tool-use --block-rm --check-package-manager --deny-destructive-find"
          }
        ]
      }
    ]
  }
}
```

Current Codex support is Bash-only because Codex `PreToolUse` currently covers Bash tool calls. The CLI does not expose Rust edit checks for Codex.

Windows note: this repository only enables Codex hooks automatically on non-Windows hosts.

Example:

```bash
echo '{"session_id":"session","transcript_path":null,"cwd":"/repo","hook_event_name":"PreToolUse","model":"gpt-5.4","permission_mode":"default","turn_id":"turn","tool_name":"Bash","tool_use_id":"tool","tool_input":{"command":"rm -rf /tmp/test"}}' | \
  agent_hooks codex pre-tool-use --block-rm
```

### GitHub Copilot CLI

Create `.github/hooks/agent-hooks.json` in your repository:

```json
{
  "version": 1,
  "hooks": {
    "preToolUse": [
      {
        "type": "command",
        "bash": "agent_hooks copilot pre-tool-use --block-rm --deny-destructive-find --check-package-manager --deny-rust-allow --expect"
      }
    ]
  }
}
```

Examples:

```bash
echo '{"toolName":"bash","toolArgs":"{\"command\":\"rm -rf /tmp/test\"}","cwd":"/repo"}' | \
  agent_hooks copilot pre-tool-use --block-rm

echo '{"toolName":"edit","toolArgs":"{\"filePath\":\"src/main.rs\",\"content\":\"#[allow(dead_code)]\"}","cwd":"/repo"}' | \
  agent_hooks copilot pre-tool-use --deny-rust-allow --expect
```

### OpenCode

Create `~/.config/opencode/plugin/agent_hooks.json`:

```json
{
  "allowExpect": true,
  "additionalContext": "See project guidelines",
  "dangerousPaths": ["~/"]
}
```

Plugin setup:

1. Place `agent_hooks.node` in `~/.config/opencode/plugin/`
2. Place `agent_hooks.ts` in `~/.config/opencode/plugin/`
3. Create `agent_hooks.json` in `~/.config/opencode/plugin/`

The plugin automatically:

- blocks `rm` commands
- blocks `rm`/`trash`/`mv` commands targeting dangerous paths
- warns on destructive `find` commands
- denies `#[allow(...)]` / `#[expect(...)]` in Rust files based on configuration

## CLI flags

### `claude permission-request`

| Flag | Description |
|------|-------------|
| `--block-rm` | Block `rm` commands and suggest using `trash` instead |
| `--dangerous-paths <paths>` | Protect dangerous paths from `rm`/`trash`/`mv` and ask for confirmation |

### `claude pre-tool-use`

| Flag | Description |
|------|-------------|
| `--deny-rust-allow` | Deny `#[allow(...)]` in Rust edits |
| `--expect` | Allow `#[expect(...)]` while denying `#[allow(...)]` |
| `--additional-context <msg>` | Append extra denial context |
| `--check-package-manager` | Deny mismatched package manager commands |
| `--deny-destructive-find` | Deny destructive `find` commands |
| `--deny-nul-redirect` | Windows only. Deny `> nul`, `2> nul`, and `&> nul` |

### `codex pre-tool-use`

| Flag | Description |
|------|-------------|
| `--block-rm` | Block `rm` commands |
| `--dangerous-paths <paths>` | Deny dangerous path operations |
| `--check-package-manager` | Deny mismatched package manager commands |
| `--deny-destructive-find` | Deny destructive `find` commands |
| `--deny-nul-redirect` | Windows only. Deny `nul` redirects |

### `copilot pre-tool-use`

| Flag | Description |
|------|-------------|
| `--block-rm` | Block `rm` commands |
| `--dangerous-paths <paths>` | Deny dangerous path operations |
| `--deny-rust-allow` | Deny `#[allow(...)]` in Rust edits |
| `--expect` | Allow `#[expect(...)]` while denying `#[allow(...)]` |
| `--additional-context <msg>` | Append extra denial context |
| `--check-package-manager` | Deny mismatched package manager commands |
| `--deny-destructive-find` | Deny destructive `find` commands |
| `--deny-nul-redirect` | Windows only. Deny `nul` redirects |

## Supported platforms

### Unified CLI

| Platform | Architecture | Binary name |
|----------|--------------|-------------|
| macOS | x86_64 | `agent_hooks-macos-x86_64` |
| macOS | arm64 | `agent_hooks-macos-arm64` |
| Linux | x86_64 | `agent_hooks-linux-x86_64` |
| Linux | arm64 | `agent_hooks-linux-arm64` |
| Windows | x86_64 | `agent_hooks-windows-x86_64.exe` |
| Windows | arm64 | `agent_hooks-windows-arm64.exe` |

Linux binaries are statically linked with musl, and Windows binaries are statically linked with CRT for maximum compatibility.

### OpenCode NAPI

| Platform | Architecture | Binary name |
|----------|--------------|-------------|
| macOS | x86_64 | `agent_hooks_opencode-macos-x86_64.node` |
| macOS | arm64 | `agent_hooks_opencode-macos-arm64.node` |
| Linux | x86_64 | `agent_hooks_opencode-linux-x86_64.node` |
| Linux | arm64 | `agent_hooks_opencode-linux-arm64.node` |
| Windows | x86_64 | `agent_hooks_opencode-windows-x86_64.node` |
| Windows | arm64 | `agent_hooks_opencode-windows-arm64.node` |

## Core API

The core library exports simple check functions that can be reused by other clients:

```rust
pub fn is_rm_command(cmd: &str) -> bool
pub fn check_destructive_find(cmd: &str) -> Option<&'static str>
pub fn has_nul_redirect(cmd: &str) -> bool
pub fn is_rust_file(file_path: &str) -> bool
pub fn check_rust_allow_attributes(content: &str) -> RustAllowCheckResult
pub fn check_dangerous_path_command(cmd: &str, dangerous_paths: &[&str]) -> Option<DangerousPathCheck>
pub fn detect_package_manager_command(cmd: &str) -> Option<PackageManager>
pub fn find_lock_files(start_dir: &Path) -> Vec<PackageManager>
pub fn check_package_manager(cmd: &str, start_dir: &Path) -> PackageManagerCheckResult
```

## Building from source

```bash
cd agent_hooks

# Build all packages
cargo build --release

# Build unified CLI only
cargo build -p agent_hooks --release

# Build OpenCode NAPI only
cargo build -p agent_hooks_opencode --release

# Run tests
cargo test
```

### OpenCode `.node` installation from source

```bash
cd agent_hooks
cargo build -p agent_hooks_opencode --release

# macOS
cp target/release/libagent_hooks_opencode.dylib ~/.config/opencode/plugin/agent_hooks.node

# Linux
cp target/release/libagent_hooks_opencode.so ~/.config/opencode/plugin/agent_hooks.node
```

```powershell
# Windows
Copy-Item target\release\agent_hooks_opencode.dll "$env:USERPROFILE\.config\opencode\plugin\agent_hooks.node"
```

## License

Apache License 2.0 - See [LICENSE](LICENSE) for details.
