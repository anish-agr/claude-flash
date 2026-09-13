# Local HTTP API

[README](../README.md) · [Getting started](getting-started.md) ·
[How it works](how-it-works.md) · [Configuration](configuration.md) · [Security](../SECURITY.md)

The agent serves a small JSON API on loopback. Claude Code's hooks use one endpoint;
the `flash` CLI uses the rest, and so can any other program.

The base URL is `http://127.0.0.1:47823`, or whichever port `agent.port` sets.

## Admission

Every request must:

- carry a `Host` header of `127.0.0.1:PORT`, `localhost:PORT` or `[::1]:PORT`, and
- carry no `Origin` header and no `Sec-Fetch-*` header.

Anything else gets `403`. Browsers always send those headers on the requests a web
page makes, so no page can use the API, even one on a domain that resolves to
127.0.0.1.

With `agent.remote = true` the agent listens on every interface, and is addressed
by whatever name the other machine used, so the `Host` check is dropped: the token
is then required on every endpoint, including the hook endpoint below. The
`Origin` and `Sec-Fetch-*` rule still applies.

Requests and responses are HTTP/1.1 with one request per connection. Bodies are
JSON, up to 512 KB; request heads are limited to 16 KB.

## Authentication

While the agent is on loopback, `/v1/health` and the hook endpoint need no token;
with `agent.remote = true` the hook endpoint needs one too. Everything else always
needs the bearer token the agent creates in its data directory on first start:

| Platform | Token file |
|---|---|
| Windows | `%LOCALAPPDATA%\ClaudeFlash\token` |
| macOS | `~/Library/Application Support/claude-flash/token` |
| Linux | `$XDG_STATE_HOME/claude-flash/token`, by default `~/.local/state/claude-flash/token` |

With `CLAUDE_FLASH_HOME` set, the token is in that directory instead.

```bash
TOKEN=$(cat "$HOME/Library/Application Support/claude-flash/token")
```

```bash
curl -s http://127.0.0.1:47823/v1/status -H "Authorization: Bearer $TOKEN"
```

In PowerShell:

```powershell
$token = Get-Content "$env:LOCALAPPDATA\ClaudeFlash\token"
```

```powershell
Invoke-RestMethod http://127.0.0.1:47823/v1/status -Headers @{ Authorization = "Bearer $token" }
```

## Errors

Errors are JSON with a message:

```json
{ "error": "missing or incorrect API token" }
```

| Status | Meaning |
|---|---|
| `400` | The body is not valid for the endpoint |
| `401` | The token is missing or wrong |
| `403` | Refused by admission |
| `404`, `405` | No such endpoint, or wrong method |
| `408` | The request did not arrive within five seconds |
| `413`, `431` | The body or the head is too large |
| `503` | The agent is shutting down or busy |

## `GET /v1/health`

Whether an agent is listening, and which.

```json
{ "name": "claude-flash", "version": "2.1.0", "api": 1, "pid": 20412 }
```

## `POST /v1/hooks/claude-code`

Where Claude Code's HTTP hooks deliver events. The body is the hook's JSON input,
unchanged. Only these fields are read: `hook_event_name`, `session_id`, `cwd`,
`permission_mode`, `tool_name`, `tool_use_id`, `notification_type` and `agent_id`.

| Header | |
|---|---|
| `X-Claude-Flash` | Required. Marks the request as coming from a Claude Flash hook |
| `X-Claude-Flash-Session` | Optional. `off`, `0`, `false` or `no` drops the event without processing it |

The response is always `200` with an empty body, which Claude Code treats as a
successful hook with no decision.

## `GET /v1/status`

```json
{
  "version": "2.1.0",
  "pid": 20412,
  "port": 47823,
  "started": "2026-09-11T08:02:11.304Z",
  "display": "windows",
  "enabled": true,
  "paused_for_ms": null,
  "away": false,
  "waiting": [
    { "kind": "approval", "project": "claude-flash", "session": "3aee9676", "tool": "Bash", "for_ms": 80213 }
  ],
  "today": { "done": 14, "question": 3, "approval": 5, "error": 0, "flashes": 21, "suppressed": 4 },
  "config_path": "C:\\Users\\you\\AppData\\Local\\ClaudeFlash\\config.toml",
  "config_error": null,
  "journal_path": "C:\\Users\\you\\AppData\\Local\\ClaudeFlash\\journal",
  "last_event": { "event": "PermissionRequest", "project": "claude-flash", "ts": "2026-09-11T11:14:03.120Z" },
  "opted_out": 0
}
```

`display` is `windows`, `macos` or `headless`. `waiting` is ordered most urgent
first. `config_error` explains why the settings file was not applied, when it was
not. `journal_path` is `null` when the journal is off.

## `POST /v1/control`

Changes a switch. The response is the status after the change.

| Body | Effect |
|---|---|
| `{"action": "on"}` | Turn flashes on |
| `{"action": "off"}` | Turn flashes off |
| `{"action": "toggle", "confirm": true}` | Flip the switch; with `confirm`, show a flash when turning on and a notification when turning off |
| `{"action": "pause", "for": "45m"}` | Hold signals for a duration; `"0"` resumes |
| `{"action": "resume"}` | End a pause |
| `{"action": "test", "kind": "approval"}` | Show a test flash, even while paused |
| `{"action": "reload"}` | Apply the settings file now; `400` with the problem if it is invalid |
| `{"action": "quit"}` | Stop the agent |

## `POST /v1/signal`

Raises a signal as if Claude Code had. It goes through the same rules: the switch,
a pause, quiet hours, project rules and presence.

```json
{
  "kind": "error",
  "title": "Nightly build failed",
  "body": "3 tests failed in pricetime",
  "source": "ci",
  "project": "pricetime"
}
```

Only `kind` is required: `done`, `question`, `approval` or `error`. `title` and
`body` are used for notifications and pushes; `project` is matched against
`[[project]]` rules, and falls back to `source`. Text is cut to 120 characters for
`title`, 500 for `body`, 40 for `source` and 80 for `project`. Unknown fields are
rejected.

The response is `202`:

```json
{ "accepted": true }
```

## `GET /v1/events`

Follows the journal as it is written. Without parameters it returns the latest 20
records; with `since`, it waits up to `wait` seconds (25 by default, 30 at most)
for records numbered `since` or later.

```bash
curl -s "http://127.0.0.1:47823/v1/events?since=41&wait=25" -H "Authorization: Bearer $TOKEN"
```

```json
{
  "next": 43,
  "records": [
    {
      "ts": "2026-09-11T11:14:03.120Z",
      "tz": -420,
      "event": "PermissionRequest",
      "kind": "approval",
      "session": "3aee9676",
      "project": "claude-flash",
      "tool": "Bash",
      "delivered": ["flash"]
    },
    {
      "ts": "2026-09-11T11:15:23.333Z",
      "tz": -420,
      "event": "PostToolUse",
      "kind": "approval",
      "session": "3aee9676",
      "project": "claude-flash",
      "tool": "Bash",
      "waited_ms": 80213
    }
  ]
}
```

Pass `next` as `since` to continue. The agent keeps the latest 512 records in
memory and numbers them from zero each time it starts, so a `next` lower than the
`since` you sent means the agent restarted.

Records have these fields, and omit the ones that do not apply:

| Field | |
|---|---|
| `ts`, `tz` | UTC time, and the local offset in minutes when it was written |
| `event` | A hook event name, or `signal`, `test`, `digest`, `reminder`, `pause`, `resume`, `enable`, `disable` |
| `kind` | `done`, `question`, `approval` or `error` |
| `session` | The first eight characters of the session id |
| `project`, `path` | The project folder name, and the full path when `journal.store_paths` is on |
| `tool`, `source` | The tool a wait was for; what raised an API signal |
| `delivered` | How it reached you: `flash`, `notify`, `push`, `digest` |
| `suppressed` | Why it did not: `disabled`, `paused`, `muted`, `signal_off`, `background`, `duplicate`, `quiet_hours`, `focused` |
| `waited_ms` | On the record that ends a wait, how long Claude was blocked |
| `detail` | Extra context, such as a pause's length or what a return digest summarised |
