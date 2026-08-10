# ClaudeFlash

Washes your whole screen in colour for about a second when Claude Code wants
you. Look away, get on with something else, and let peripheral vision tell you
when to come back.

**Windows only.** The overlay is built on Win32 — `UpdateLayeredWindow`,
`WS_EX_TRANSPARENT`, `GetAsyncKeyState` — and the installer uses PowerShell and
the registry. See [Other platforms](#other-platforms).

| Colour | Means | Fires on |
|---|---|---|
| **Green** `#00FF5A` | Claude finished responding | `Stop` |
| **Blue** `#08A9FF` | Claude has a question for you | `PreToolUse` / `AskUserQuestion` |
| **Purple** `#8B2FCE` | Claude is waiting for you to approve something | `PreToolUse` + `PostToolUse` |

The overlay is click-through and never takes focus, so it can't eat a keystroke
or a click. Any mouse button or key makes it vanish immediately.

## Install

Double-click **`setup.cmd`**, or:

```bash
powershell -ExecutionPolicy Bypass -File install.ps1 -PermissionFlash
```

Drop `-PermissionFlash` if you only want green and blue. Everything else is
tunable afterwards without reinstalling.

That builds `bin\flash.exe`, copies it to
`%LOCALAPPDATA%\Microsoft\WindowsApps\flash.exe`, drops two desktop shortcuts,
and adds the Claude Code hooks. **Restart Claude Code afterwards** — hooks are
read once when a session starts.

Nothing to install first: it compiles with the C# compiler that already ships
with Windows.

`WindowsApps` is on the user PATH by default on Windows 10/11, so `flash` works
as a bare command from `Win+R`, `cmd` and PowerShell straight away — including
terminals that are already open, which editing PATH could not do.

If you edit the source, re-run `install.ps1` — that's what refreshes the
installed copy.

## Commands

Run these from `Win+R`, `cmd`, PowerShell, or the desktop shortcuts.

```
flash                       green flash
flash done                  green flash
flash ask                   blue flash
flash perm                  purple flash
flash <colour>              any colour, see below

flash set <key> <value>     change a setting permanently
flash config                open config.ini in your editor
flash reset                 restore all defaults
flash status                current state, config path, whether hooks are live

flash off                   disable everything (hooks stay, they just do nothing)
flash on                    enable
flash toggle                flip — green confirm = on, red confirm = off
flash help                  full usage
```

Colour names: `green` `amber` `red` `blue` `violet` `lavender` `indigo` `teal`
`pink` `purple` `cyan` `white`, or any `#RRGGBB`.

### Changing colours

`flash set` validates the value, writes one line of `config.ini` leaving the
comments intact, and confirms by flashing the colour you just set. **Applies
immediately — no reinstall, no restart.**

```bash
flash set color_perm "#C13FFF"
```

```bash
flash set color_ask teal
```

Bad values are rejected rather than silently ignored — `flash set color_ask
notacolour` tells you it isn't a colour and changes nothing.

### Turning the purple flash on and off

Green and blue are exact. Purple is a heuristic (see
[below](#the-purple-flash-and-its-one-false-positive)), so it has its own switch:

```bash
flash set perm_flash off
```

```bash
flash set perm_flash on
```

Immediate, and it leaves green and blue alone. The hooks stay installed either
way, so flipping it back costs nothing.

### Changing timings

```bash
flash set hold_ms 700
```

```bash
flash set prompt_wait_ms 5000
```

```bash
flash set alpha 0.35
```

All of these take effect on the next flash. Every key also works as a one-off
flag for experimenting before you commit to it:

```bash
flash perm --alpha=0.4 --hold_ms=900
```

## Settings

`%LOCALAPPDATA%\ClaudeFlash\config.ini`, created on first run. Edit directly, or
use `flash set <key> <value>`.

| Key | Default | What it does |
|---|---|---|
| `alpha` | `0.28` | Peak opacity, 0–1. Higher = harder to miss |
| `alpha_ask` | `0.20` | Opacity for the blue flash |
| `alpha_perm` | `0.20` | Opacity for the purple flash |
| `color_done` | `#00FF5A` | "Finished" colour |
| `color_ask` | `#08A9FF` | "Has a question" colour |
| `color_perm` | `#8B2FCE` | "Waiting for approval" colour |
| `perm_flash` | `on` | Turn the purple flash on/off without reinstalling |
| `perm_modes` | `default,plan,acceptEdits` | Permission modes purple may fire in |
| `prompt_wait_ms` | `3000` | How long a call may run before an unfinished one counts as waiting on you |
| `fade_in_ms` | `70` | Fade-in time |
| `hold_ms` | `420` | Time at full opacity |
| `fade_out_ms` | `560` | Fade-out time |
| `dismiss_fade_ms` | `110` | How fast it goes once you click or type |
| `min_visible_ms` | `120` | Ignore input this early, so a keystroke already in flight can't kill the flash before you see it |
| `vignette` | `0.32` | 0 = flat tint. Higher = clearer in the middle, so you can still read what's underneath |
| `skip_if_focused` | *(empty)* | Comma-separated process names. Skip the flash when one of them is already the focused window |

### Picking a colour that reads properly

Softening a colour by *lightening* it doesn't work. A tint keeps its hue but
lightness turns into white haze, so pale shades like `#7FD8FF` look like white
fog at 20% opacity rather than blue. Keep the colour saturated and soften it
with opacity instead — that's why `alpha_ask` and `alpha_perm` exist separately
from `alpha`.

The same trap catches purple: `#A855F7` has such a high blue channel that at low
opacity it desaturates and reads as the ask blue. `#8B2FCE` is deeper and stays
purple.

If the flash is too much during heavy back-and-forth:

```bash
flash set skip_if_focused WindowsTerminal
```

## Where it works

| | |
|---|---|
| Claude Code, desktop app | yes — all three colours verified |
| Claude Code, terminal (CLI) | yes — same `~/.claude/settings.json`, same hooks |
| Claude Code, VS Code / JetBrains | yes — those run Claude Code underneath |
| Claude Code on the web | no — runs in the cloud, can't reach your machine |
| Normal Claude chats (app or browser) | no — no hook system exists for them |
| Claude Code inside WSL / over SSH | no — hooks run in Linux, where `flash.exe` isn't |
| Exclusive-fullscreen games | probably not — topmost overlays get suppressed. Borderless fullscreen is fine |
| `flash` as a plain command | yes — anywhere on this Windows machine |
| macOS / Linux | no — see [Other platforms](#other-platforms) |

Anything that can run a command can trigger it, Claude or not:

```bash
npm run build; flash green
```

## How it works

`install.ps1` writes hooks into `~/.claude/settings.json`, preserving any that
are already there. `--bg` makes the process relaunch itself detached and return
in a few milliseconds, so hooks never delay Claude.

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

### The purple flash, and its one false positive

Green and blue map onto real events. Purple doesn't have one, and it's worth
knowing why before you trust it.

`Notification` is supposed to provide `permission_prompt`, but on the Windows
desktop app it never fired in testing — not with an exact matcher, not with
`"*"`, not with none. Those entries are still installed since they cost nothing
and only ever fire on a real prompt, but don't count on them.

So purple is inferred from two checks:

**1. Is this a mode that asks at all?** Every hook receives `permission_mode` on
stdin:

| `permission_mode` | Flashes? |
|---|---|
| `default`, `plan` | yes — you get prompted |
| `acceptEdits` | yes — it auto-accepts *edits* but still asks before commands |
| `auto`, `dontAsk`, `bypassPermissions` | no — nothing is ever asked |

So it follows your mode automatically, per call. In bypass it is silent without
being told.

**2. Was this call actually blocked on you?** Even in `default` most calls are
pre-approved. Nothing in the payload distinguishes them, but behaviour does: an
approved call completes on its own, a prompted one can't finish until you click.
`PreToolUse` drops a marker keyed by `tool_use_id`, `PostToolUse` deletes it, and
the flash waits `prompt_wait_ms` before looking:

| After the wait | Meaning | Result |
|---|---|---|
| marker gone | the tool ran and finished | silent |
| marker still there | something is waiting on you | purple |

**The false positive:** an approved call that legitimately runs longer than
`prompt_wait_ms` looks identical to a prompt and will flash. There is no fix,
because Claude Code emits no "tool started" event — only before-approval and
after-completion — so a long-running command and an unanswered prompt are
genuinely indistinguishable.

Measured on the default install: `PostToolUse` costs **~800ms of overhead** even
for an instant `echo`, which is why the default is 3000ms rather than something
tighter. Raise it if long commands set purple off:

```bash
flash set prompt_wait_ms 8000
```

Or switch purple off and keep the two exact signals:

```bash
flash set perm_flash off
```

### Two gotchas worth knowing

- On events that support a matcher, an entry **without** one never fires. `Stop`
  takes no matcher and works; `PreToolUse` and `Notification` need one. Matchers
  are registered one per exact tool name — a single `Bash|Write|...` alternation
  did not match.
- Hooks load when the session starts. Editing `settings.json` mid-session does
  nothing until you restart Claude Code. Changing `config.ini`, however, applies
  to the very next flash — as does rebuilding `flash.exe`, since the hook command
  string is unchanged.

### The overlay

One layered window per monitor, drawn with `UpdateLayeredWindow` so alpha varies
per pixel — the tint is stronger at the edges than in the middle, which makes it
noticeable without hiding your work. It's `WS_EX_TRANSPARENT` (clicks pass
straight through), `WS_EX_NOACTIVATE` (never takes focus) and `WS_EX_TOOLWINDOW`
(stays out of alt-tab).

Because clicks pass through, the overlay never sees input, so dismissing it means
detecting input another way.

**It does not install a keyboard hook.** `SetWindowsHookEx(WH_KEYBOARD_LL)` is
the obvious approach and the wrong one: a global keyboard hook is the defining
behaviour of a keylogger, antivirus treats it as such, and an unsigned binary
that installs one is asking to be quarantined. Those API names do not appear in
the compiled binary at all. It polls `GetAsyncKeyState` on the animation timer
instead — no hook, nothing intercepted, and it can only observe during the ~1
second the flash is up. The loop stops at the first key that changed and never
records which one.

### If Windows warns about it

`flash.exe` is unsigned and every rebuild produces a new hash with no reputation,
so Defender may run a cloud check the first time a freshly built copy runs. That
is a reputation warning, not a detection. Confirm nothing was flagged:

```powershell
Get-MpThreatDetection | Select-Object InitialDetectionTime, ThreatID, Resources
```

Signing would remove the warning, and costs money. Building from source — the
only way this ships — means you can read what it does first.

## Testing it

```bash
powershell -ExecutionPolicy Bypass -File selftest.ps1
```

Drives every hook the way Claude Code does — through `cmd.exe` with the real
JSON payload on stdin — and checks a flash actually rendered, including that
purple stays silent for an approved call and fires for a blocked one.

It asserts on a timestamp the overlay writes when it genuinely draws, **not** on
the presence of a `flash.exe` process. The short-lived `--bg` parent shares that
name, and an earlier version of this suite passed on the parent while the purple
flash was completely broken.

### Other platforms

The hook wiring would carry over, since `~/.claude/settings.json` is the same
everywhere; only the overlay needs rewriting.

- **macOS** — a borderless `NSWindow` at `.screenSaver` level with
  `ignoresMouseEvents = true`, one per `NSScreen`. Swift, roughly a hundred lines.
- **Linux** — compositor-dependent. On X11, an override-redirect window with an
  empty input region via XShape. Wayland has no portable equivalent;
  `wlr-layer-shell` covers wlroots compositors only.

## Notes

- Covers every monitor, and is DPI-aware, so it fills scaled displays exactly.
- A new flash cancels one still fading, so the colour always reflects the latest event.
- Message boxes only appear for commands you type (`set`, `status`, `help`). Hook-driven flashes are always silent.
- If it ever fails, it writes `%LOCALAPPDATA%\ClaudeFlash\error.log` rather than dying silently.
- Set `%LOCALAPPDATA%\ClaudeFlash\trace` to any content to log hook timings to `timing.log`; delete it to stop.

## License

MIT
