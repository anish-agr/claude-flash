# Security

## Reporting a vulnerability

Report vulnerabilities privately through
[GitHub's private vulnerability reporting](https://github.com/anish-agr/claude-flash/security/advisories/new),
not in a public issue. Include the version (`flash --version`), the operating
system, and the steps to reproduce.

## What the agent exposes

`flash-agent` listens on `127.0.0.1`, on port 47823 by default, and makes no
outbound connections except the push notifications you configure. It binds a wider
address only when you set `agent.remote`, described at the end of this file.

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
| `POST /v1/hooks/claude-code` | `X-Claude-Flash` header, and the token when `agent.remote` is on | Records a hook event, which may flash |
| `GET /v1/status`, `POST /v1/control`, `POST /v1/signal`, `GET /v1/events` | bearer token | Read status, change switches, raise signals, follow the journal |

The token is 32 random bytes from the operating system, stored in the data
directory. On macOS and Linux the file is created with mode `0600`; on Windows it
sits in the user's profile folder (`~\.claude-flash`), which only that user can
read. Token comparison runs in constant time.

While the agent is on loopback the hook endpoint needs no token, because Claude
Code's HTTP hooks have nowhere secure to read one from. Any process running on the
machine can therefore make the agent flash or add entries to the journal. It cannot read the status, change
settings, pause or stop the agent, or read the journal through the API.

## Taking events from other machines

`agent.remote = true` makes the agent listen on every interface, so that Claude
Code in WSL, over SSH or on another computer can reach it. Two things change:

- The token is required on every endpoint, the hook endpoint included. Nothing on
  the network can raise a signal without it.
- The `Host` allowlist is dropped, because the agent is then addressed by whatever
  name the other machine used and the header settles nothing. The `Origin` and
  `Sec-Fetch-*` rule stays, so web pages are still refused.

`flash hooks remote HOST` prints hooks that carry the token. It then sits in
`settings.json` on the other machine as plain text, which is the cost of this
mode: anyone who can read that file can raise signals on your desktop, read the
agent's status and change its settings. Leave `agent.remote` off unless you want
it.

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

## Code signing

The released `flash.exe` and `flash-agent.exe` are not yet Authenticode-signed.
Windows SmartScreen acts only on the "downloaded from the internet" mark, which the
install steps clear, but **Smart App Control** judges the programs themselves: it
runs code it recognises or that carries a valid signature, and blocks unsigned code
it does not recognise, with no per-program exception. A signature on an installer
would not help, because Smart App Control checks every executable, so both programs
have to be signed.

The plan is to sign both binaries in the release workflow with a certificate from
the [SignPath Foundation](https://signpath.org/), which provides free OV code
signing to open-source projects through a GitHub Actions integration and keeps the
private key in its own HSM. Until that is in place, users whom Smart App Control
blocks can turn it off or build from source, as
[docs/getting-started.md](docs/getting-started.md#if-something-is-wrong) describes.
