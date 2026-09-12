# Configuration

[README](../README.md) · [How it works](how-it-works.md) ·
[Local API](api.md) · [Security](../SECURITY.md)

Claude Flash reads `config.toml` from:

| Platform | Location |
|---|---|
| Windows | `%LOCALAPPDATA%\ClaudeFlash\config.toml` |
| macOS | `~/Library/Application Support/claude-flash/config.toml` |
| Linux | `$XDG_CONFIG_HOME/claude-flash/config.toml`, by default `~/.config/claude-flash/config.toml` |

With `CLAUDE_FLASH_HOME` set, this file and everything else Claude Flash keeps live
in that directory instead.

The agent writes the file, with every setting and a comment on each, the first time
it starts. It checks for changes every second. A valid file applies straight away;
an invalid one is reported by `flash status`, `flash doctor` and `flash config
check`, and the settings already in effect stay in effect.

## Reading and changing settings

```bash
flash config edit
```

opens the file in `$VISUAL` or `$EDITOR` when either is set, and otherwise in the
system's text editor.

```bash
flash config set flash.hold_ms 600
```

changes one value in place, leaving the comments and layout alone. Booleans accept
`true`, `false`, `on`, `off`, `yes` and `no`, and lists accept comma-separated
values:

```bash
flash config set flash.skip_when_focused "WindowsTerminal, Code"
```

The result is validated before it is written, so a bad value never reaches the
file. `flash config get KEY` prints one value, `flash config check` validates the
file, and `flash config reset` restores the commented defaults after saving the
current file as `config.toml.bak`.

## Value formats

**Durations** carry a unit: `250ms`, `90s`, `15m`, `1h30m`, `2d`. `"0"` turns off
whatever the duration controls. A bare number is refused, because `15` could mean
seconds or minutes.

**Colours** are `#RRGGBB`, `#RGB`, or a name:

| Name | Colour | Name | Colour |
|---|---|---|---|
| `green` | `#00FF5A` | `violet` | `#A855F7` |
| `blue` | `#08A9FF` | `lavender` | `#B9A7FF` |
| `purple` | `#8B2FCE` | `indigo` | `#7C6BFF` |
| `red` | `#FF3B30` | `teal` | `#00C9A7` |
| `amber` | `#FFAA00` | `pink` | `#FF7AB8` |
| `white` | `#FFFFFF` | `cyan` | `#00E5FF` |

**Times of day** are 24-hour `HH:MM`, in local time.

## `[flash]`

| Key | Default | Meaning |
|---|---|---|
| `style` | `"wash"` | `"wash"` tints the whole screen, more at the edges; `"edge"` glows around the border and leaves the centre clear |
| `vignette` | `0.32` | For `wash`, how much clearer the centre is than the edges, from 0 (a flat tint) to 0.9 |
| `fade_in_ms` | `70` | Fade-in time, up to 5000 |
| `hold_ms` | `420` | Time at full strength, up to 10000 |
| `fade_out_ms` | `560` | Fade-out time, up to 10000 |
| `dismiss_fade_ms` | `110` | Fade-out after a key press or click, up to 2000 |
| `min_visible_ms` | `120` | Input sooner than this after a flash starts does not dismiss it, up to 2000 |
| `min_interval_ms` | `1000` | Flashes never start closer together than this, from 334 to 60000 |
| `respect_reduce_motion` | `true` | Use short cross-fades when the system asks for reduced motion |
| `skip_when_focused` | `[]` | Applications that suppress flashes while focused, such as `"WindowsTerminal"`, `"Code"` or `"iTerm2"`. Case and a trailing `.exe` are ignored |
| `remind_after` | `"0"` | Flash again after this long while Claude is still blocked on you; `"0"` is off |

## `[signals.done]`, `[signals.question]`, `[signals.approval]`, `[signals.error]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Whether the signal is shown at all |
| `color` | `#00FF5A`, `#08A9FF`, `#8B2FCE`, `#FF3B30` | The flash colour |
| `opacity` | `0.28`, `0.2`, `0.2`, `0.24` | Peak opacity, from 0.02 to 1 |

A translucent tint keeps its hue, while lightness turns into white haze, so a pale
colour reads as fog rather than as a colour. To make a flash gentler, lower its
opacity and keep the colour saturated. The default purple is deep for the same
reason: lighter purples drift towards the blue of a question at low opacity.

## `[sessions]`

| Key | Default | Meaning |
|---|---|---|
| `ignore_background` | `true` | Hold back signals from sessions nobody typed a prompt into, such as subagents and scripted runs. Until any session is known, nothing is held back |

## `[presence]`

| Key | Default | Meaning |
|---|---|---|
| `away_after` | `"5m"` | With no keyboard or mouse input for this long, you count as away; `"0"` turns presence off |
| `notify_when_away` | `true` | While you are away, show a desktop notification instead of a flash |
| `digest_on_return` | `true` | When you return, flash once in the colour of the most urgent signal among the waits still open and the signals that arrived while you were away |

## `[push]`

| Key | Default | Meaning |
|---|---|---|
| `url` | `""` | Where to send signals raised while you are away. Empty turns push off |
| `format` | `"ntfy"` | `"ntfy"` posts the text with `Title`, `Priority` and `Tags` headers; `"json"` posts `{"kind", "title", "body", "ts"}` |
| `token` | `""` | Sent as `Authorization: Bearer TOKEN` when set |
| `after` | `"10m"` | Push only once you have been away at least this long |
| `kinds` | `["question", "approval", "error"]` | Which signals to push |

With [ntfy](https://ntfy.sh), pick a topic name nobody could guess, subscribe to it
in the ntfy app, and point Claude Flash at it:

```toml
[push]
url = "https://ntfy.sh/a-long-random-topic-name"
```

## `[quiet_hours]`

| Key | Default | Meaning |
|---|---|---|
| `start` | `""` | When quiet hours begin, such as `"22:00"` |
| `end` | `""` | When they end, such as `"08:00"`. Set both or neither; the window may cross midnight |
| `notify` | `false` | Still show desktop notifications during quiet hours |

## `[[project]]`

Rules for particular projects. The first rule that matches a signal applies.

| Key | Meaning |
|---|---|
| `match` | A pattern. One containing `/` is matched against the session's working directory, and one without against the project folder's name. `*` matches any run of characters and `?` any one character, ignoring case. Windows paths are compared with forward slashes |
| `mute` | `true` holds back every signal from the project |
| `colors` | Colours for the project's signals, by kind |

```toml
[[project]]
match = "*/experiments/*"
mute = true

[[project]]
match = "pricetime"
colors = { done = "#FFD400", approval = "#FF7AB8" }
```

Signals raised through the API are matched by their `project` field, or by
`source` when they have no project. Project rules are edited in the file;
`flash config set` does not change them.

## `[agent]`

| Key | Default | Meaning |
|---|---|---|
| `port` | `47823` | The agent's loopback port. The installed hooks include it, so after changing it run `flash hooks install` and `flash agent restart` |
| `remote` | `false` | Take events from other machines, such as Claude Code in WSL or over SSH. The agent listens on every interface instead of loopback, and the token becomes necessary on every request, hook events included. `flash hooks remote HOST` prints the hooks to install on the other machine. Restart the agent after changing this |

## `[journal]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Keep a journal of signals |
| `retain_days` | `30` | Delete day files older than this, from 1 to 3650 |
| `store_paths` | `false` | Record full working-directory paths, not only project folder names |

## Environment variables

| Variable | Effect |
|---|---|
| `CLAUDE_FLASH` | In a Claude Code session's environment, `off`, `0`, `false` or `no` leaves that session out |
| `CLAUDE_FLASH_HOME` | Keep the settings, state, token, journal and log in this directory |
| `CLAUDE_CONFIG_DIR` | Claude Code's configuration directory, where `settings.json` is looked for |
| `NO_COLOR` | Print CLI output without colour |
| `VISUAL`, `EDITOR` | The editor `flash config edit` opens |
