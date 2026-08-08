# ClaudeFlash

Washes your whole screen green for about a second when Claude Code finishes a
response, and blue when it has a question for you. Look away, get on with
something else, and let peripheral vision tell you when to come back.

- **Green** — Claude finished responding.
- **Blue** — Claude has a question for you.
- **Violet** — Claude wants permission to run something.

The overlay is click-through and never takes focus, so it can't eat a keystroke
or a click. Any mouse button or key makes it vanish immediately.

## Install

Double-click **`setup.cmd`**, or:

```bash
powershell -ExecutionPolicy Bypass -File install.ps1
```

That builds `bin\flash.exe`, copies it to
`%LOCALAPPDATA%\Microsoft\WindowsApps\flash.exe`, drops two desktop shortcuts,
and adds the Claude Code hooks. **Restart Claude Code afterwards** so it picks up
the hooks.

Nothing to install first — it compiles with the C# compiler that already ships
with Windows.

`WindowsApps` is on the user PATH by default on Windows 10/11, so `flash` works
as a bare command from `Win+R`, `cmd` and PowerShell straight away — including
terminals that are already open, which editing PATH could not do. Hooks,
shortcuts and the `Win+R` registration all point at that one copy, so there's
never a stale second binary.

If you edit the source, re-run `install.ps1` — that's what refreshes the
installed copy.

## Using it

| | |
|---|---|
| `Win+R` → `flash` | test flash |
| `Win+R` → `flash toggle` | turn it on/off — green confirm = on, red = off |
| `Win+R` → `flash status` | current state, config path, whether hooks are live |
| Desktop → **Claude Flash (test)** | same as `flash` |
| Desktop → **Claude Flash on-off** | same as `flash toggle` |

Full command list:

```
flash                 green flash
flash done            green flash
flash ask             blue flash
flash perm            violet flash
flash <color>         green | amber | red | blue | violet | lavender | indigo
                      | teal | pink | purple | cyan | white | #RRGGBB
flash on | off | toggle | status | help
```

## Turning it off

`flash off` (or the desktop toggle). The hooks stay installed and simply do
nothing, so turning it back on is instant. To rip it out completely, run
`uninstall.ps1`.

## Tuning it

`%LOCALAPPDATA%\ClaudeFlash\config.ini`, created on first run. Edit and save —
the next flash picks it up.

```ini
alpha=0.28           # peak opacity, 0..1. higher = harder to miss
alpha_ask=0.20       # the ask colour gets its own, gentler opacity. blue only reads
                     # as blue while it stays saturated, and lightening it to soften
                     # it just turns the wash white — so soften with opacity instead
fade_in_ms=70
hold_ms=420
fade_out_ms=560
dismiss_fade_ms=110  # how fast it goes once you click or type
min_visible_ms=120   # ignore input this early, so a keystroke already in flight
                     # can't kill the flash before you see it
vignette=0.32        # 0 = flat tint. higher = clearer in the middle, so you can
                     # still read what's underneath
color_done=#00FF5A
color_ask=#08A9FF
color_perm=#A855F7   # shown when Claude asks permission to run something
alpha_perm=0.22
skip_if_focused=     # comma-separated process names; skip the flash when one of
                     # them is already the focused window
```

Any of these also works as a one-off flag: `flash done --alpha=0.5 --hold_ms=900`.

If the flash is too loud during heavy back-and-forth, `skip_if_focused=WindowsTerminal`
suppresses it whenever you're already looking at the terminal.

## How it works

`install.ps1` writes these hooks into `~/.claude/settings.json`:

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

Green comes from `Stop`. Blue comes from `PreToolUse` matching
`AskUserQuestion` — the tool call Claude makes when it asks you something.

Violet comes from `Notification` matching `permission_prompt`. The other
`Notification` types (`idle_prompt`, `elicitation_dialog`, `agent_needs_input`)
map to blue.

Caveat on `Notification`: on the Windows desktop app, `idle_prompt` and
`elicitation_dialog` did not fire in testing, so blue is driven off
`PreToolUse`/`AskUserQuestion` instead, which does. `permission_prompt` is
untested — no permission prompt came up during testing — so violet may or may
not fire for you. It costs nothing if it doesn't.

## Where it works

| | |
|---|---|
| Claude Code, desktop app | yes — green and blue verified here |
| Claude Code, terminal (CLI) | yes — same `~/.claude/settings.json`, same hooks |
| Claude Code, VS Code / JetBrains | yes — those run Claude Code underneath |
| Claude Code on the web | no — runs in the cloud, can't reach your machine |
| Normal Claude chats (app or browser) | no — no hook system exists for them |
| Claude Code inside WSL / over SSH | no — hooks run in Linux, where `flash.exe` isn't |
| `flash` as a plain command | yes — anywhere on this Windows machine |

Anything that can run a command can trigger it, Claude or not:

```bash
npm run build; flash green
```

Two things worth knowing if you extend this:

- On events that support a matcher, an entry **without** one never fires. `Stop`
  takes no matcher and works fine; `PreToolUse` and `Notification` need one.
- Hooks are read when the session starts. Editing `settings.json` mid-session
  changes nothing until you restart Claude Code.

`--bg` makes the process relaunch itself detached and return in a few
milliseconds, so the hook never delays Claude Code.

The overlay itself is one layered window per monitor, drawn with
`UpdateLayeredWindow` so alpha varies per pixel — the tint is stronger at the
edges than in the middle, which is what makes it noticeable without hiding your
work. It's `WS_EX_TRANSPARENT` (clicks pass straight through),
`WS_EX_NOACTIVATE` (never takes focus) and `WS_EX_TOOLWINDOW` (stays out of
alt-tab).

Because clicks pass through, the overlay never sees input. Two low-level hooks
answer the single question "did anything happen" and dismiss it — no key code is
ever read or recorded, and every event is passed straight on to whatever was
going to receive it. The hooks live only for the ~1 second the flash is up.

### Adding more triggers

Any Claude Code hook event works. To flash when a subagent finishes, add a
`SubagentStop` entry pointing at `"...\flash.exe" violet --bg`.

## Notes

- Covers every monitor, and is DPI-aware, so it fills scaled displays exactly.
- A new flash cancels one still fading, so the color always reflects the latest event.
- If it ever fails, it writes `%LOCALAPPDATA%\ClaudeFlash\error.log` rather than dying silently.

## License

MIT
