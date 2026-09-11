# Security

## Reporting a vulnerability

Report vulnerabilities privately through
[GitHub's private vulnerability reporting](https://github.com/anish-agr/claude-flash/security/advisories/new),
not in a public issue. Include the version (`flash --version`), the operating
system, and the steps to reproduce.

## What the agent exposes

`flash-agent` listens on `127.0.0.1` only, on port 47823 by default. It never
binds to other interfaces and makes no outbound connections except the push
notifications you configure.

Every request passes two checks before it is routed:

- **Host allowlist.** The `Host` header must name loopback on the agent's port
  (`127.0.0.1`, `localhost` or `[::1]`). This defeats DNS rebinding, where a web
  page resolves its own domain to 127.0.0.1.
- **Browser rejection.** Requests carrying `Origin` or any `Sec-Fetch-*` header
  are refused. Browsers attach these to every cross-origin request and a page
  cannot remove them; command-line clients and Claude Code's hook runner send
  neither.

Endpoints are then split by what they can do:

| Endpoint | Authentication | Effect |
|---|---|---|
| `GET /v1/health` | none | Reports the agent's name, version and process id |
| `POST /v1/hooks/claude-code` | `X-Claude-Flash` header | Records a hook event, which may flash |
| `GET /v1/status`, `POST /v1/control`, `POST /v1/signal`, `GET /v1/events` | bearer token | Read status, change switches, raise signals, follow the journal |

The token is 32 random bytes from the operating system, stored in the data
directory. On macOS and Linux the file is created with mode `0600`; on Windows it
sits in the user's local application data folder, which only that user can read.
Token comparison runs in constant time.

The hook endpoint needs no token, because Claude Code's HTTP hooks have no
secure way to read one. Any process running on the machine can therefore make the
agent flash or add entries to the journal. It cannot read the status, change
settings, pause or stop the agent, or read the journal through the API.

## Claude Code's behaviour is never changed

Every hook response is an empty `200`. Claude Code treats that as a successful
hook that makes no decision, so Claude Flash cannot approve, deny or block a tool
call, and cannot add anything to Claude's context. The `SessionStart` command hook
prints nothing to standard output for the same reason.

## What is stored

The journal records event names, signal kinds, project folder names, tool names,
session id prefixes, durations and what was shown. It never records prompts,
tool input or output, file contents or assistant messages; those fields are not
read from hook payloads at all. Full working-directory paths are recorded only
with `journal.store_paths = true`.

Journal files are kept for 30 days by default (`journal.retain_days`) and can be
turned off with `journal.enabled = false`.

## Push notifications

When `push.url` is set, signals raised while you are away are sent there with the
system `curl`. The request, including any token, is passed on curl's standard
input rather than its command line, so other users cannot see it in the process
list. Header values are stripped of control characters before they are sent.
