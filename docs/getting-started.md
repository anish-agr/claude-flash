# Getting started

[README](../README.md) · [How it works](how-it-works.md) ·
[Configuration](configuration.md) · [Local API](api.md) · [Security](../SECURITY.md)

This guide installs Claude Flash, checks each part of it on your machine, and
covers reporting a problem and removing it again. It takes about fifteen minutes
on Windows or macOS. Where the two differ, each gets its own command.

## Before you start

You need Windows 10 (version 1803 or later) or Windows 11, or macOS 11 or later,
with Claude Code installed and signed in. The terminal, the desktop app and editor
extensions all work, because they read the same settings file.

Installing changes these things, and [removing Claude Flash](#removing-claude-flash)
undoes them:

| | Windows | macOS |
|---|---|---|
| Programs | `%LOCALAPPDATA%\Microsoft\WindowsApps` | `~/.local/bin` |
| Settings and journal | `%LOCALAPPDATA%\ClaudeFlash` | `~/Library/Application Support/claude-flash` |
| Start at login | A `Run` registry value | A LaunchAgent |
| Claude Code hooks | `~/.claude/settings.json`, with a backup beside it | The same |

The agent listens only on this machine. Its journal records event names, signal
kinds, project folder names and timings, and never prompts, code or Claude's
replies.

## 1. Install

### Windows

In Windows PowerShell, run these one at a time:

```powershell
cd $env:TEMP
```

```powershell
curl.exe -fLO https://github.com/anish-agr/claude-flash/releases/latest/download/claude-flash-windows-x64.zip
```

```powershell
Expand-Archive claude-flash-windows-x64.zip -DestinationPath claude-flash -Force
```

```powershell
.\claude-flash\flash.exe install
```

Type `curl.exe` in full: in Windows PowerShell, `curl` on its own runs a different
command.

The installer prints a line for each step and ends with `Claude Flash 2.1.0 is
installed`. A green sphere appears in the notification area; Windows 11 may put it
under the arrow that shows hidden icons. The programs go in a folder that is
already on the `PATH`, so `flash` works straight away:

```powershell
flash --version
```

### macOS

In Terminal, run these one at a time:

```bash
cd "$TMPDIR"
```

```bash
curl -fLO https://github.com/anish-agr/claude-flash/releases/latest/download/claude-flash-macos-universal.tar.gz
```

```bash
mkdir -p claude-flash && tar -xzf claude-flash-macos-universal.tar.gz -C claude-flash
```

```bash
./claude-flash/flash install
```

The installer prints a line for each step and ends with `Claude Flash 2.1.0 is
installed`, and a green sphere appears in the menu bar. On macOS 13 or later a
Background Items Added notification may name `flash-agent`; that is the login item.

The programs go in `~/.local/bin`. Check that your shell finds them:

```bash
flash --version
```

If that says `command not found`, the folder is not on your `PATH`. Add it for
zsh, the default shell, and try again:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc
```

## 2. Restart Claude Code

Claude Code reads hooks when a session starts, so a session that was open during
the install cannot reach Claude Flash. Quit every session: `/exit` in a terminal,
quit the desktop app completely, and close the Claude Code panel in your editor.
Sessions you start from now on send their events to Claude Flash.

## 3. Check each part

A command is the same on both platforms unless the check shows one for each.

### Test flashes

```bash
flash test
```

Green, blue, purple and red flashes, one after another, each about a second long,
on every display.

### Out of your way

```bash
flash test done
```

Click something while the flash is up, and the click goes through. Run it again and
press a key: the flash fades out early. On macOS the flash also covers the menu bar
and the Dock.

### Full screen and other desktops

Run the command for your platform, then within five seconds switch to a
full-screen window or to another desktop (a virtual desktop on Windows, a Space on
macOS). The purple flash appears there too.

On Windows:

```powershell
Start-Sleep 5; flash test approval
```

On macOS:

```bash
sleep 5; flash test approval
```

On Windows this covers borderless full screen, such as a browser or a video player.
A game in exclusive full screen draws over it.

### Tray icon or menu bar item

Open the sphere's menu and turn flashes off: the icon turns grey. Turn them back on
and it turns green.

### A finished turn

```bash
claude -p "Reply with just: ok"
```

Claude answers `ok`, and the screen flashes green.

### An approval

Start a session with `claude` and ask it to create a file called `hello.txt`. When
the permission prompt appears, the screen flashes purple. Before you answer, run
`flash status` in another window: it lists the approval Claude is waiting on.

If Claude writes the file without asking, your permission settings already allow
it, and no flash is the right result. Ask it to run `touch hello2.txt` instead.

### A question

In the same session, ask: `Ask me a multiple-choice question about what to name a
cat.` While the question is on screen, the screen flashes blue.

### The journal

```bash
flash log --since 30m
```

Each signal from the checks above, with what became of it. A signal that did not
flash says why, such as `held back: background session`.

### Doctor

```bash
flash doctor
```

Every line starts with ✓, and the last one reads `Everything checks out.` Until a
Claude Code session has sent an event, the activity line shows `!` instead.

### If you have time

**Notifications while you are away.** Give Claude a task that takes a few minutes,
then leave the computer alone for five minutes. Signals arrive as notifications
instead of flashes, and when you come back, one flash shows the most urgent thing
you missed. On Windows, Focus Assist or Do Not Disturb hides the notifications. On
macOS they are sent through AppleScript, so System Settings lists them under Script
Editor.

**Sound.** Turn on the sound for a finished turn, and show a test flash.

On Windows:

```powershell
flash config set signals.done.sound true; Start-Sleep 2; flash test done
```

On macOS:

```bash
flash config set signals.done.sound true && sleep 2 && flash test done
```

Windows plays its Asterisk sound and macOS its Glass sound. Turn it off again with
`flash config set signals.done.sound false`.

**A second display.** With another display connected, `flash test` covers both.

## If something is wrong

**`command not found: flash` on macOS.** `~/.local/bin` is not on your `PATH`. Add
it with the line in [Install](#macos).

**`flash` is not recognized on Windows.** Your `PATH` no longer includes
`%LOCALAPPDATA%\Microsoft\WindowsApps`. Add that folder back to your user `PATH`,
or run the program by its full path:
`& "$env:LOCALAPPDATA\Microsoft\WindowsApps\flash.exe" --version`.

**macOS will not open `flash`.** The archive was downloaded in a browser, which
quarantines it. In the folder you extracted it in, run
`xattr -dr com.apple.quarantine claude-flash`, then `./claude-flash/flash install`
again.

**Windows will not run `flash.exe`.** A zip downloaded in a browser is marked as
coming from the internet, and SmartScreen acts on that mark. Run `Unblock-File` on
the zip and extract it again. Smart App Control, when it is on, blocks unsigned
programs whatever their origin, and allows no exception for a single program.

**Claude Code shows "hook error occurred".** The agent is not running, so Claude
Code cannot deliver events to it. Start it with `flash agent start`.

**A Claude Code session never flashes.** It started before the install; restart it.
`flash log --since 15m --all` shows whether its events arrive.

**A flash did not appear.** `flash log` says what became of each signal, such as
`held back: quiet hours`.

## Reporting a problem

Open an [issue](https://github.com/anish-agr/claude-flash/issues) that says what you
did, what you expected and what happened, and paste in the report this command puts
on the clipboard: the system version, the Claude Code and Claude Flash versions,
`flash doctor`, the last hour of the journal and the end of the agent's log.

On Windows:

```powershell
& { Get-CimInstance Win32_OperatingSystem | Format-List Caption, Version; claude --version; flash --version; flash doctor; flash log --since 1h --all; flash agent logs } 2>&1 | Out-String | Set-Clipboard
```

On macOS:

```bash
( sw_vers; uname -m; claude --version; flash --version; flash doctor; flash log --since 1h --all; flash agent logs ) 2>&1 | pbcopy
```

The journal names project folders and tools, and holds no prompts or code. Report a
security problem privately instead, as [SECURITY.md](../SECURITY.md) describes.

## Removing Claude Flash

```bash
flash uninstall --purge
```

This stops the agent and removes the hooks, the login item, the settings and the
journal. The programs stay, so delete them next.

On Windows:

```powershell
Remove-Item "$env:LOCALAPPDATA\Microsoft\WindowsApps\flash.exe", "$env:LOCALAPPDATA\Microsoft\WindowsApps\flash-agent.exe"
```

On macOS:

```bash
rm ~/.local/bin/flash ~/.local/bin/flash-agent
```

Restart Claude Code once more. A session that is still open keeps the hooks it
loaded, and shows "hook error occurred" until it restarts. One file stays behind on
purpose: `~/.claude/settings.json.claude-flash-backup`, your settings as they were
before Claude Flash last changed them. Delete it when you no longer need it.
