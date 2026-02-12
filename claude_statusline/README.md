# claude_statusline

Powerline-style statusline renderer for Claude Status hook input.

## Input

Reads a JSON payload from `stdin` (hook event `Status`).

## Output

Prints one ANSI-colored powerline containing:

- Model name (prettified from `model.display_name` or `model.id`)
- CWD folder name
- Project directory folder name (when different from CWD)
- Git branch or short commit hash
- Session cost in USD (when > $0.00)
- Context window usage bar with percentage

## Usage

Place the binary at `~/.claude/hooks/claude_statusline` and add the following to `~/.claude/settings.json`:

```json
{
  "statusLine": {
    "type": "command",
    "command": "$HOME/.claude/hooks/claude_statusline",
    "padding": 0
  }
}
```

On Windows, use `%USERPROFILE%\\.claude\\hooks\\claude_statusline.exe` instead.
