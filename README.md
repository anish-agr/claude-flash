# ClaudeFlash

Washes your whole screen in colour for about a second when Claude Code wants
you. Look away, get on with something else, and let peripheral vision tell you
when to come back.

| Colour | Means | Fires on |
|---|---|---|
| **Green** `#00FF5A` | Claude finished responding | `Stop` |
| **Blue** `#08A9FF` | Claude has a question for you | `PreToolUse` / `AskUserQuestion` |
| **Violet** `#A855F7` | Claude wants to run something | `PreToolUse` / tool names — opt-in |

The overlay is click-through and never takes focus, so it can't eat a keystroke
or a click. Any mouse button or key makes it vanish immediately.

## Install

Double-click **`setup.cmd`**, or:

```bash
powershell -ExecutionPolicy Bypass -File install.ps1
```

Add `-PermissionFlash` for the violet signal (see [Permission
flashes](#permission-flashes) — it's off by default for a reason):

```bash
powershell -ExecutionPolicy Bypass -File install.ps1 -PermissionFlash
```

That builds `bin\flash.exe`, copies it to
`%LOCALAPPDATA%\Microsoft\WindowsApps\flash.exe`, drops two desktop shortcuts,
and adds the Claude Code hooks. **Restart Claude Code afterwards** — hooks are
read once when a session starts, so edits mid-session change nothing.

Nothing to install first: it compiles with the C# compiler that already ships
with Windows.

`WindowsApps` is on the user PATH by default on Windows 10/11, so `flash` works
as a bare command from `Win+R`, `cmd` and PowerShell straight away — including
terminals that are already open, which editing PATH could not do. Hooks,
shortcuts and the `Win+R` registration all point at that one copy, so there's
never a stale second binary.

If you edit the source, re-run `install.ps1` — that's what refreshes the
installed copy.

## Commands

Run these from `Win+R`, `cmd`, PowerShell, or the desktop shortcuts.

### Flashing

```
flash                       green flash
flash done                  green flash
flash ask                   blue flash
flash perm                  violet flash
flash <colour>              any colour, see below
```

Colour names: `green` `amber` `red` `blue` `violet` `lavender` `indigo` `teal`
`pink` `purple` `cyan` `white`, or any `#RRGGBB`.

```bash
flash "#00C2FF"
```

### Changing settings

```
flash set <key> <value>     change a setting permanently
flash config                open config.ini in your editor
flash reset                 restore all defaults
flash status                current state, config path, whether hooks are live
```

`flash set` confirms by flashing in the colour you just set, validates what you
give it, and leaves the comments in `config.ini` intact:

```bash
flash set color_ask "#08A9FF"
```

```bash
flash set alpha 0.35
```

```bash
flash set hold_ms 700
```

Bad values are rejected rather than silently ignored — `flash set color_ask
notacolour` tells you it isn't a colour and changes nothing.

### Turning it off

```
flash off                   disable (hooks stay installed, they just do nothing)
flash on                    enable
flash toggle                flip — green confirm = on, red confirm = off
```

Turning it back on is instant. To remove it entirely, run `uninstall.ps1`.

## Settings

Every key below works with `flash set <key> <value>`, as a one-off flag
(`flash done --alpha=0.5 --hold_ms=900`), or by editing
`%LOCALAPPDATA%\ClaudeFlash\config.ini` directly.

| Key | Default | What it does |
|---|---|---|
| `alpha` | `0.28` | Peak opacity, 0–1. Higher = harder to miss |
| `alpha_ask` | `0.20` | Opacity for the blue flash |
| `alpha_perm` | `0.17` | Opacity for the violet flash |
| `color_done` | `#00FF5A` | "Finished" colour |
| `color_ask` | `#08A9FF` | "Has a question" colour |
| `color_perm` | `#A855F7` | "Wants permission" colour |
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

If the flash is too much during heavy back-and-forth:

```bash
flash set skip_if_focused WindowsTerminal
```

That suppresses it whenever you're already looking at the terminal.

## Where it works

| | |
|---|---|
| Claude Code, desktop app | yes — green and blue verified |
| Claude Code, terminal (CLI) | yes — same `~/.claude/settings.json`, same hooks |
| Claude Code, VS Code / JetBrains | yes — those run Claude Code underneath |
| Claude Code on the web | no — runs in the cloud, can't reach your machine |
| Normal Claude chats (app or browser) | no — no hook system exists for them |
| Claude Code inside WSL / over SSH | no — hooks run in Linux, where `flash.exe` isn't |
| Exclusive-fullscreen games | probably not — topmost overlays get suppressed. Borderless fullscreen is fine |
| `flash` as a plain command | yes — anywhere on this Windows machine |

Anything that can run a command can trigger it, Claude or not:

```bash
npm run build; flash green
```

## How it works

`install.ps1` writes hooks into `~/.claude/settings.json`, preserving any that
are already there:

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

`--bg` makes the process relaunch itself detached and return in a few
milliseconds, so the hook never delays Claude Code.

### Permission flashes

There is no reliable "permission requested" event. `Notification` is supposed to
provide one, but on the Windows desktop app it never fired in testing — not
`permission_prompt`, not `idle_prompt`, not `elicitation_dialog`, and not with a
`"*"` matcher or no matcher at all. `Notification` entries are still installed
because they cost nothing and may work elsewhere, but don't count on them.

So `-PermissionFlash` approximates it with `PreToolUse` on common tool names,
which fires *before a tool runs*. In manual approval mode that's exactly when
you get prompted. With tools pre-approved it fires anyway, without a prompt — so
it's noisy in auto-approve mode. That's the tradeoff, and why it's opt-in.
`AskUserQuestion` is excluded from the match so it keeps its own blue.

To turn it off, re-run `install.ps1` without the flag.

### Two gotchas worth knowing

- On events that support a matcher, an entry **without** one never fires. `Stop`
  takes no matcher and works; `PreToolUse` and `Notification` need one.
- Hooks load when the session starts. Editing `settings.json` mid-session does
  nothing until you restart Claude Code.

### The overlay

One layered window per monitor, drawn with `UpdateLayeredWindow` so alpha varies
per pixel — the tint is stronger at the edges than in the middle, which makes it
noticeable without hiding your work. It's `WS_EX_TRANSPARENT` (clicks pass
straight through), `WS_EX_NOACTIVATE` (never takes focus) and `WS_EX_TOOLWINDOW`
(stays out of alt-tab).

Because clicks pass through, the overlay never sees input. Two low-level hooks
answer the single question "did anything happen" and dismiss it — no key code is
ever read or recorded, and every event is passed straight on to whatever was
going to receive it. The hooks live only for the ~1 second the flash is up.

### Adding more triggers

Any Claude Code hook event works. To flash when a subagent finishes:

```json
{ "hooks": { "SubagentStop": [
  { "hooks": [{ "type": "command", "command": "\"...\\flash.exe\" teal --bg" }] }
] } }
```

## Notes

- Covers every monitor, and is DPI-aware, so it fills scaled displays exactly.
- A new flash cancels one still fading, so the colour always reflects the latest event.
- Message boxes only appear for commands you type (`set`, `status`, `help`). Hook-driven flashes are always silent.
- If it ever fails, it writes `%LOCALAPPDATA%\ClaudeFlash\error.log` rather than dying silently.

## License

MIT
