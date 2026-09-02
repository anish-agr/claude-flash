# ClaudeFlash

Tints the whole screen for about a second when Claude Code needs attention, so a
completed response or a pending prompt is visible without watching the terminal.

Windows only. The overlay uses Win32 (`UpdateLayeredWindow`, `WS_EX_TRANSPARENT`,
`GetAsyncKeyState`) and the installer uses PowerShell and the registry. See
[Other platforms](#other-platforms).

| Colour | Meaning | Trigger |
|---|---|---|
| Green `#00FF5A` | Response finished | `Stop` |
| Blue `#08A9FF` | Claude asked a question | `PreToolUse` / `AskUserQuestion` |
| Purple `#8B2FCE` | Waiting for approval | `PreToolUse` + `PostToolUse` |

The overlay is click-through and never takes focus, so it cannot consume a
keystroke or a click. Any mouse button or key dismisses it.

## Install

Run `setup.cmd`, or:

```bash
powershell -ExecutionPolicy Bypass -File install.ps1 -PermissionFlash
```

Omit `-PermissionFlash` for green and blue only. Everything else is configurable
afterwards without reinstalling.

The installer compiles `bin\flash.exe`, copies it to
`%LOCALAPPDATA%\Microsoft\WindowsApps\flash.exe`, creates a desktop toggle, and
adds hooks to `~/.claude/settings.json`. Restart Claude Code afterwards; hooks
are read once per session at startup.

No prerequisites: it compiles with the C# compiler included with Windows.

`WindowsApps` is on the user PATH by default on Windows 10/11, so `flash` resolves
from `Win+R`, `cmd` and PowerShell, including already-open terminals. Hooks,
shortcut and the `Win+R` registration all reference that copy.

After editing the source, re-run `install.ps1` to refresh the installed binary.

## Commands

Available from `Win+R`, `cmd`, PowerShell, or the desktop shortcut.

```
flash                       green flash
flash done                  green flash
flash ask                   blue flash
flash perm                  purple flash
flash <colour>              named colour or #RRGGBB

flash set <key> <value>     change a setting
flash config                open config.ini
flash reset                 restore defaults
flash status                current state, config path, hook status

flash on | off | toggle     enable or disable all flashes
flash help                  full usage
```

Colour names: `green` `amber` `red` `blue` `violet` `lavender` `indigo` `teal`
`pink` `purple` `cyan` `white`, or any `#RRGGBB`.

### Colours

`flash set` validates the value, rewrites one line of `config.ini` leaving
comments intact, and confirms with a flash in the colour set. Changes apply to
the next flash; no reinstall or restart.

```bash
flash set color_perm "#C13FFF"
```

```bash
flash set color_ask teal
```

Invalid values are rejected without modifying the config.

### Timing and opacity

```bash
flash set hold_ms 700
```

```bash
flash set alpha 0.35
```

```bash
flash set prompt_wait_ms 5000
```

Every key also works as a one-off command-line flag:

```bash
flash perm --alpha=0.4 --hold_ms=900
```

### Disabling the purple flash

Green and blue correspond to specific events. Purple is inferred (see
[Purple flash](#purple-flash)) and has a separate switch:

```bash
flash set perm_flash off
```

```bash
flash set perm_flash on
```

This takes effect immediately and does not affect green or blue. The hooks remain
installed either way.

### Disabling for scripted runs

`flash off` stops the overlay from drawing, but the hooks still fire and still
launch a process per event. When a script creates sessions in a loop, remove the
hooks instead:

```bash
powershell -ExecutionPolicy Bypass -File hooks.ps1 -Off
```

```bash
powershell -ExecutionPolicy Bypass -File hooks.ps1 -On
```

Hooks are read at session start, so this applies to sessions created afterwards.
Running `hooks.ps1` with no arguments reports the current state.

## Settings

`%LOCALAPPDATA%\ClaudeFlash\config.ini`, created on first run. Edit directly or
use `flash set <key> <value>`.

| Key | Default | Description |
|---|---|---|
| `alpha` | `0.28` | Peak opacity, 0–1 |
| `alpha_ask` | `0.20` | Opacity of the blue flash |
| `alpha_perm` | `0.20` | Opacity of the purple flash |
| `color_done` | `#00FF5A` | Response-finished colour |
| `color_ask` | `#08A9FF` | Question colour |
| `color_perm` | `#8B2FCE` | Approval-pending colour |
| `perm_flash` | `on` | Enable the purple flash |
| `perm_modes` | `default,plan,acceptEdits` | Permission modes in which purple may fire |
| `only_your_sessions` | `on` | Restrict flashes to sessions you submitted a prompt in |
| `prompt_wait_ms` | `3000` | How long a call may run before an unfinished one counts as blocked |
| `fade_in_ms` | `70` | Fade-in duration |
| `hold_ms` | `420` | Duration at full opacity |
| `fade_out_ms` | `560` | Fade-out duration |
| `dismiss_fade_ms` | `110` | Fade-out duration after input |
| `min_visible_ms` | `120` | Input before this is ignored, so a keystroke already in flight does not dismiss the flash early |
| `vignette` | `0.32` | 0 is a flat tint; higher keeps the centre clearer than the edges |
| `skip_if_focused` | *(empty)* | Comma-separated process names; skip the flash if one owns the focused window |

### Choosing a colour

Lightening a colour does not soften it. A translucent tint retains its hue while
lightness becomes white haze, so a pale value such as `#7FD8FF` reads as white fog
at 20% opacity rather than blue. Keep the colour saturated and reduce opacity
instead, which is why `alpha_ask` and `alpha_perm` are separate from `alpha`.

The same applies to purple: `#A855F7` has a high blue channel and desaturates
toward the blue used for questions at low opacity. `#8B2FCE` is deeper and stays
distinguishable.

To suppress flashes while the terminal is already focused:

```bash
flash set skip_if_focused WindowsTerminal
```

## Compatibility

| Environment | Supported |
|---|---|
| Claude Code, desktop app | Yes |
| Claude Code, terminal (CLI) | Yes — same `~/.claude/settings.json` |
| Claude Code, VS Code / JetBrains | Yes — these run Claude Code underneath |
| Claude Code on the web | No — runs remotely |
| Claude chat (app or browser) | No — no hook system |
| Claude Code under WSL or SSH | No — hooks run in Linux, where `flash.exe` is unavailable |
| Exclusive-fullscreen applications | Generally no; topmost overlays are suppressed. Borderless fullscreen works |
| `flash` as a standalone command | Yes, anywhere on the machine |
| macOS / Linux | No — see [Other platforms](#other-platforms) |

Any command can trigger it:

```bash
npm run build; flash green
```

## How it works

`install.ps1` writes hooks into `~/.claude/settings.json`, preserving existing
entries. `--bg` makes the process relaunch itself detached and return within a few
milliseconds, so hooks do not delay Claude Code.

```json
{
  "hooks": {
    "Stop": [
      { "hooks": [{ "type": "command", "command": "\"...\\flash.exe\" done --bg" }] }
    ],
    "PreToolUse": [
      { "matcher": "AskUserQuestion",
        "hooks": [{ "type": "command", "command": "\"...\\flash.exe\" ask --bg" }] }
    ]
  }
}
```

### Purple flash

Green and blue map directly onto events. Purple has no corresponding event and is
inferred from two checks.

`Notification` provides a `permission_prompt` type, but it did not fire on the
Windows desktop app during testing, with an exact matcher, with `"*"`, or with no
matcher. Those entries are still installed because they only fire on a real
prompt, but they are not relied upon.

**Permission mode.** Every hook receives `permission_mode` on stdin:

| `permission_mode` | Purple fires |
|---|---|
| `default`, `plan` | Yes — prompts occur |
| `acceptEdits` | Yes — edits are auto-accepted, commands still prompt |
| `auto`, `dontAsk`, `bypassPermissions` | No — nothing is prompted |

This is evaluated per call, so switching modes needs no configuration change.

**Whether the call was blocked.** In `default` mode most calls are pre-approved,
and the payload does not distinguish them. An approved call completes on its own;
a prompted one cannot complete until answered. `PreToolUse` writes a marker keyed
by `tool_use_id`, `PostToolUse` removes it, and the flash waits `prompt_wait_ms`
before checking:

| After the wait | Interpretation | Result |
|---|---|---|
| Marker removed | Call completed | Silent |
| Marker present | Call is blocked | Purple |

Known limitation: an approved call that runs longer than `prompt_wait_ms` is
indistinguishable from a pending prompt and will flash. Claude Code emits no
"tool started" event, only before-approval and after-completion, so the two cases
cannot be separated.

`PostToolUse` adds roughly 800 ms of overhead even for an immediate command, which
is why the default is 3000 ms. Increase it if long-running commands trigger
purple:

```bash
flash set prompt_wait_ms 8000
```

### Session filtering

Hooks in `~/.claude/settings.json` apply to every Claude Code session. A single
prompt can spawn background agents, each a separate session firing its own `Stop`
on completion, which produces flashes while the originating prompt is still
running.

Sessions that received a typed prompt emit `UserPromptSubmit`; spawned agents do
not. A `UserPromptSubmit` hook records those session ids and flashes are checked
against them. If no session has been recorded, flashes still fire.

```bash
flash set only_your_sessions off
```

### Hook behaviour

- On events that support matchers, an entry without one never fires. `Stop` takes
  no matcher; `PreToolUse` and `Notification` require one. Matchers are registered
  one per exact tool name, as a single `Bash|Write|...` alternation did not match.
- Hooks are read at session start. Editing `settings.json` has no effect until
  Claude Code restarts. `config.ini` is read per flash, and rebuilding `flash.exe`
  applies immediately because the hook command string is unchanged.

### Overlay

One layered window per monitor, drawn with `UpdateLayeredWindow` so opacity varies
per pixel; the tint is stronger at the edges than the centre, which keeps
underlying content readable. The window is `WS_EX_TRANSPARENT` (clicks pass
through), `WS_EX_NOACTIVATE` (never takes focus) and `WS_EX_TOOLWINDOW` (excluded
from alt-tab).

Because clicks pass through, the overlay receives no input, so dismissal is
detected separately. It polls `GetAsyncKeyState` on the animation timer rather
than installing `SetWindowsHookEx(WH_KEYBOARD_LL)`: a global keyboard hook matches
the behaviour antivirus associates with keyloggers, and those API names are absent
from the compiled binary. Polling installs nothing, intercepts nothing, and only
runs during the ~1 second the flash is visible. The loop stops at the first key
that changed state and does not record which one.

### Antivirus

`flash.exe` is unsigned, and each rebuild produces a new hash with no reputation,
so Defender may perform a cloud reputation check or quarantine it. If flashes stop
working, verify the binary is still present:

```powershell
Test-Path "$env:LOCALAPPDATA\Microsoft\WindowsApps\flash.exe"
```

```powershell
Get-MpThreatDetection | Select-Object InitialDetectionTime, ThreatID, Resources
```

Adding an exclusion for the install directory prevents recurrence. Code signing
would remove the warning entirely.

## Testing

```bash
powershell -ExecutionPolicy Bypass -File selftest.ps1
```

Invokes each hook the way Claude Code does — through `cmd.exe` with the real JSON
payload on stdin — and verifies a flash rendered. Coverage includes purple firing
across permission modes, staying silent for approved calls and firing for blocked
ones, every command path staying silent while disabled, and 20 concurrent hooks
under a disabled switch.

Assertions use a timestamp the overlay writes when it draws, rather than the
presence of a `flash.exe` process, because the short-lived `--bg` parent shares
that process name.

## Other platforms

The hook configuration is portable since `~/.claude/settings.json` is identical
across platforms; only the overlay requires reimplementation.

- **macOS** — a borderless `NSWindow` at `.screenSaver` level with
  `ignoresMouseEvents = true`, one per `NSScreen`.
- **Linux** — compositor-dependent. On X11, an override-redirect window with an
  empty input region via XShape. Wayland has no portable equivalent;
  `wlr-layer-shell` covers wlroots compositors only.

## Notes

- Covers all monitors and is DPI-aware, filling scaled displays exactly.
- A new flash cancels one still fading, so the colour reflects the latest event.
- Message boxes appear only for interactive commands (`set`, `status`, `help`).
  Hook-driven flashes are silent.
- Failures are written to `%LOCALAPPDATA%\ClaudeFlash\error.log`.
- Creating `%LOCALAPPDATA%\ClaudeFlash\trace` logs every invocation to
  `timing.log`, including the enabled state, before any gate is applied. Delete
  the file to stop.

## License

MIT
