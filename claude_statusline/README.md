# claude_statusline

Powerline-style statusline renderer for Claude Status hook input.

## Input

Reads a JSON payload from `stdin` (hook event `Status`).

## Output

Prints one ANSI-colored line containing:

- model
- cwd
- project directory segment when `workspace.project_dir` differs from cwd
- context usage percent
- right-aligned version segment

## Run

```bash
cargo run -q -p claude_statusline < status.json
```
