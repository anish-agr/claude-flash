# Builds flash.exe and wires it up:
#   * Win+R  ->  "flash"           (registry App Paths, no PATH edits, works immediately)
#   * Desktop shortcuts            (test flash + on/off toggle)
#   * Claude Code hooks            (Stop -> green, Notification -> amber)
#
# Re-running this is safe. Undo everything with uninstall.ps1.

[CmdletBinding()]
param(
    [switch]$NoHooks,
    [switch]$NoShortcuts,
    [switch]$NoRun,
    [switch]$Diagnose,
    [switch]$PermissionFlash
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$built = Join-Path $root 'bin\flash.exe'

# Canonical install location. %LOCALAPPDATA%\Microsoft\WindowsApps is on the user
# PATH out of the box on Windows 10/11, so putting the binary here makes `flash`
# resolve from Win+R, cmd, and PowerShell - including shells that are already
# open, which a PATH edit could never do. Everything below points at this one
# copy, so there is never a stale second binary to get confused about.
$installDir = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps'
$exe = Join-Path $installDir 'flash.exe'

function Step($text) { Write-Host "  $text" -ForegroundColor Gray }
function Ok($text) { Write-Host "  $text" -ForegroundColor Green }

Write-Host "`nClaudeFlash setup" -ForegroundColor Cyan
Write-Host ("-" * 50) -ForegroundColor DarkGray

# ---- 1. build ---------------------------------------------------------------
& (Join-Path $root 'build.ps1') | Out-Null
if (-not (Test-Path $built)) { throw "Build did not produce $built" }
Ok "Built bin\flash.exe"

# ---- 2. install so the name resolves everywhere -----------------------------
Get-Process -Name flash -ErrorAction SilentlyContinue | ForEach-Object {
    try { $_.Kill(); $_.WaitForExit(2000) } catch { }   # a running flash would lock the file
}
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Copy-Item $built $exe -Force
Ok "Installed to $exe"

# Belt and braces: App Paths makes Win+R work even if PATH is ever mangled.
$appPaths = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\flash.exe'
New-Item -Path $appPaths -Force | Out-Null
Set-ItemProperty -Path $appPaths -Name '(Default)' -Value $exe
Set-ItemProperty -Path $appPaths -Name 'Path' -Value $installDir
Ok "'flash' available in Win+R, cmd and PowerShell"

# ---- 3. desktop shortcuts ---------------------------------------------------
if (-not $NoShortcuts) {
    $desktop = [Environment]::GetFolderPath('Desktop')
    $shell = New-Object -ComObject WScript.Shell
    $shortcuts = @(
        @{ Name = 'Claude Flash (test).lnk'; Args = 'done';   Desc = 'Fire a test flash' },
        @{ Name = 'Claude Flash on-off.lnk'; Args = 'toggle'; Desc = 'Turn ClaudeFlash on or off (green = on, red = off)' }
    )
    foreach ($s in $shortcuts) {
        $lnk = $shell.CreateShortcut((Join-Path $desktop $s.Name))
        $lnk.TargetPath = $exe
        $lnk.Arguments = $s.Args
        $lnk.WorkingDirectory = $installDir
        $lnk.IconLocation = "$exe,0"
        $lnk.Description = $s.Desc
        $lnk.Save()
    }
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell)
    Ok "Desktop shortcuts created"
}

# ---- 4. Claude Code hooks ---------------------------------------------------
if (-not $NoHooks) {
    $settingsPath = Join-Path $env:USERPROFILE '.claude\settings.json'
    New-Item -ItemType Directory -Force -Path (Split-Path $settingsPath) | Out-Null

    if (Test-Path $settingsPath) {
        $backup = "$settingsPath.bak-$(Get-Date -Format yyyyMMdd-HHmmss)"
        Copy-Item $settingsPath $backup
        Step "Backed up settings.json -> $(Split-Path -Leaf $backup)"
        $settings = Get-Content $settingsPath -Raw | ConvertFrom-Json
    } else {
        $settings = [pscustomobject]@{}
    }

    if (-not $settings.PSObject.Properties.Name.Contains('hooks') -or $null -eq $settings.hooks) {
        $settings | Add-Member -NotePropertyName hooks -NotePropertyValue ([pscustomobject]@{}) -Force
    }

    # Clear out every entry this installer owns, across all events, before adding any
    # back. Doing it once up front means each Set-FlashHook call can simply append.
    #
    # The previous version filtered inside Set-FlashHook instead, which silently
    # deleted our OWN other entries on the same event: adding the violet PreToolUse
    # hook wiped the blue AskUserQuestion one, because both matched "flash.exe".
    foreach ($ev in @($settings.hooks.PSObject.Properties.Name)) {
        $survivors = @($settings.hooks.$ev) | Where-Object {
            $_ -and (($_ | ConvertTo-Json -Depth 10 -Compress) -notmatch 'flash\.exe|ClaudeFlash\\\\hook\.log')
        }
        if ($survivors.Count -eq 0) { $settings.hooks.PSObject.Properties.Remove($ev) }
        else { $settings.hooks.$ev = $survivors }
    }

    function Set-FlashHook($hooks, [string]$Event, [string]$Command, [string]$Matcher) {
        # Everything of ours is already gone, so this only ever appends.
        $kept = @()
        if ($hooks.PSObject.Properties.Name -contains $Event) { $kept = @($hooks.$Event) | Where-Object { $_ } }
        $entry = [pscustomobject]@{
            hooks = @([pscustomobject]@{ type = 'command'; command = $Command })
        }
        # Notification matches on notification type (permission_prompt, idle_prompt,
        # agent_needs_input, ...). Without an explicit matcher it never fires.
        if ($Matcher) { $entry | Add-Member -NotePropertyName matcher -NotePropertyValue $Matcher }

        $value = @($entry) + $kept
        if ($hooks.PSObject.Properties.Name -contains $Event) { $hooks.$Event = $value }
        else { $hooks | Add-Member -NotePropertyName $Event -NotePropertyValue $value }
    }

    Set-FlashHook $settings.hooks 'Stop' ('"{0}" done --bg' -f $exe)

    # One entry per notification type rather than a single wildcard. A bare "*" and an
    # absent matcher both failed to fire; an exact type matches whether the matcher is
    # compared literally or as a regex.
    # permission_prompt gets its own colour: "may I run this" is a different ask from
    # "answer my question".
    $needsYou = [ordered]@{
        permission_prompt  = 'perm'
        idle_prompt        = 'ask'
        elicitation_dialog = 'ask'
        agent_needs_input  = 'ask'
    }
    $entries = @()
    foreach ($type in $needsYou.Keys) {
        $commands = @([pscustomobject]@{ type = 'command'; command = ('"{0}" {1} --bg' -f $exe, $needsYou[$type]) })
        if ($Diagnose) {
            $commands += [pscustomobject]@{
                type    = 'command'
                command = ('cmd /c echo [%TIME%] {0} >> "%LOCALAPPDATA%\ClaudeFlash\hook.log"' -f $type)
            }
        }
        $entries += [pscustomobject]@{ matcher = $type; hooks = $commands }
    }
    if ($Diagnose) {
        # Catch-all logger: tells us if ANY notification fires under a type we did not list.
        $entries += [pscustomobject]@{
            hooks = @([pscustomobject]@{
                type    = 'command'
                command = 'cmd /c echo [%TIME%] (no matcher) >> "%LOCALAPPDATA%\ClaudeFlash\hook.log"'
            })
        }
        Step "Diagnostic logging on -> %LOCALAPPDATA%\ClaudeFlash\hook.log"
    }

    $keptNotif = @()
    if ($settings.hooks.PSObject.Properties.Name -contains 'Notification') {
        $keptNotif = @($settings.hooks.Notification) | Where-Object { $_ }
    }
    $value = $entries + $keptNotif
    if ($settings.hooks.PSObject.Properties.Name -contains 'Notification') { $settings.hooks.Notification = $value }
    else { $settings.hooks | Add-Member -NotePropertyName Notification -NotePropertyValue $value }

    # Notification turned out not to fire at all in the desktop app, so amber also
    # rides on the tool call Claude makes when it asks you something. This is the
    # trigger that actually works.
    Set-FlashHook $settings.hooks 'PreToolUse' ('"{0}" ask --bg' -f $exe) 'AskUserQuestion'

    if ($PermissionFlash) {
        # Notification/permission_prompt never fires here, so the closest available
        # signal is "a tool is about to run". In manual approval mode that is exactly
        # when you get prompted; with tools pre-approved it fires without a prompt too.
        # AskUserQuestion is deliberately excluded so it keeps its own colour.
        # One entry per tool with an exact matcher, rather than a single
        # "Bash|Write|..." alternation. AskUserQuestion matches as an exact name and
        # works; an alternation never fired, which is what a literal string compare
        # would do. Exact names match either way, so this stops depending on it.
        $tools = 'Bash', 'PowerShell', 'Write', 'Edit', 'NotebookEdit', 'WebFetch',
                 'WebSearch', 'Task', 'Agent', 'Read', 'Glob', 'Grep', 'Artifact', 'Workflow'
        $dbg = if ($Diagnose) { ' --debug_payload' } else { '' }
        foreach ($tool in $tools) {
            # PreToolUse arms it; PostToolUse disarms it if the tool finishes on its own.
            # Only a call that is still unfinished after the wait was actually blocked on
            # you, so an already-approved call never flashes.
            Set-FlashHook $settings.hooks 'PreToolUse' `
                ('"{0}" perm --bg --require_mode --wait_prompt{1}' -f $exe, $dbg) $tool
            Set-FlashHook $settings.hooks 'PostToolUse' ('"{0}" mark{1}' -f $exe, $dbg) $tool
        }
        Step "Permission flash on -> purple. Tune with: flash set prompt_wait_ms <ms>"
    }

    if ($Diagnose) {
        # Log every event we can name, so one restart reveals which ones fire.
        foreach ($ev in 'PreToolUse', 'PostToolUse', 'UserPromptSubmit', 'Stop', 'SubagentStop', 'SessionStart') {
            $logger = [pscustomobject]@{
                hooks = @([pscustomobject]@{
                    type    = 'command'
                    command = ('cmd /c echo [%TIME%] {0} >> "%LOCALAPPDATA%\ClaudeFlash\hook.log"' -f $ev)
                })
            }
            if ($settings.hooks.PSObject.Properties.Name -contains $ev) {
                $settings.hooks.$ev = @($settings.hooks.$ev) + $logger
            } else {
                $settings.hooks | Add-Member -NotePropertyName $ev -NotePropertyValue @($logger)
            }
        }
    }

    $json = $settings | ConvertTo-Json -Depth 32
    [System.IO.File]::WriteAllText($settingsPath, $json, (New-Object System.Text.UTF8Encoding($false)))
    Ok "Hooks installed in $settingsPath"
    Step "Stop -> green,  AskUserQuestion -> blue"
}

# ---- verify -----------------------------------------------------------------
# Resolve the bare name the way a fresh shell would, rather than assuming it works.
$resolved = cmd.exe /c "where flash 2>nul"
if ($LASTEXITCODE -eq 0 -and $resolved) { Ok "Verified: 'flash' resolves to $($resolved | Select-Object -First 1)" }
else { Write-Host "  WARNING: 'flash' did not resolve on PATH" -ForegroundColor Yellow }

# ---- done -------------------------------------------------------------------
Write-Host ("-" * 50) -ForegroundColor DarkGray
Write-Host "Ready.`n" -ForegroundColor Cyan
Write-Host "  Win+R  ->  flash          test it"          -ForegroundColor White
Write-Host "  Win+R  ->  flash toggle   on / off"         -ForegroundColor White
Write-Host "  Win+R  ->  flash status   what's it doing"  -ForegroundColor White
Write-Host ""
Write-Host "  Restart Claude Code for the hooks to load.`n" -ForegroundColor Yellow

if (-not $NoRun) { Start-Process -FilePath $exe -ArgumentList 'done' }
