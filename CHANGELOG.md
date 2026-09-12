# Changelog

Notable changes to Claude Flash. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `flash run -- COMMAND` runs a command and raises a signal for how it went, then
  exits with the command's own code. `--only-errors` reports only failures. It
  replaces the `&&`/`||` pair, which PowerShell 5.1 cannot run.

## [2.0.0] - 2026-09-11

A rewrite in Rust with macOS support, built around a background agent.

### Added

- macOS support: overlay windows on every display, above full-screen apps and the
  menu bar, and a menu bar item.
- `flash-agent`, a background process that receives Claude Code hook events over
  loopback HTTP. On Windows it has a tray icon; both platforms show the state in
  the icon's colour and switch flashes, pauses and tests from its menu.
- A red signal for turns that end in an API error (`StopFailure`).
- Approval detection from `PermissionRequest`, matched to its tool call by
  `tool_use_id`. Waits stay open until answered, and `flash status` lists them.
- Presence: after five minutes without input, signals become desktop
  notifications, and returning brings one flash for the most important thing that
  happened.
- Push notifications through ntfy or any JSON webhook.
- Quiet hours, project rules that mute or recolour by path or folder name, and
  reminders while Claude stays blocked.
- A journal of signals, with `flash log`, `flash watch` and `flash stats`.
- A local API for raising signals from other tools, and `flash signal`.
- `CLAUDE_FLASH=off` in a session's environment keeps its events out.
- `flash doctor`, `flash config`, `flash hooks`, `flash install`, `flash uninstall`
  and shell completions.
- Reduced-motion support, and a limit of three flashes per second however many
  events arrive.

### Changed

- Settings moved from `config.ini` to `config.toml`, grouped by signal.
  `flash install` converts the old file.
- Hooks deliver events over HTTP instead of starting a process for each one.

### Removed

- The C# implementation and the PowerShell scripts.
- `prompt_wait_ms` and `perm_modes`. Approvals are now detected directly, so
  neither the delay nor the permission-mode filter is needed.

## [1.0.0] - 2026-08-03

First release, for Windows: green, blue and purple flashes from Claude Code hooks,
with a desktop toggle and `flash set` for colours, opacity and timing.
