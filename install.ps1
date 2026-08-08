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
    [switch]$Diagnose
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

    function Set-FlashHook($hooks, [string]$Event, [string]$Command, [string]$Matcher) {
        # Keep any hooks the user already had; replace only our own.
        $kept = @()
        if ($hooks.PSObject.Properties.Name -contains $Event) {
            $kept = @($hooks.$Event) | Where-Object {
                $_ -and (($_ | ConvertTo-Json -Depth 10 -Compress) -notmatch 'flash\.exe|ClaudeFlash')
            }
        }
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
    $needsYou = 'permission_prompt', 'idle_prompt', 'elicitation_dialog', 'agent_needs_input'
    $entries = @()
    foreach ($type in $needsYou) {
        $commands = @([pscustomobject]@{ type = 'command'; command = ('"{0}" ask --bg' -f $exe) })
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
        $keptNotif = @($settings.hooks.Notification) | Where-Object {
            $_ -and (($_ | ConvertTo-Json -Depth 10 -Compress) -notmatch 'flash\.exe|ClaudeFlash')
        }
    }
    $value = $entries + $keptNotif
    if ($settings.hooks.PSObject.Properties.Name -contains 'Notification') { $settings.hooks.Notification = $value }
    else { $settings.hooks | Add-Member -NotePropertyName Notification -NotePropertyValue $value }

    $json = $settings | ConvertTo-Json -Depth 32
    [System.IO.File]::WriteAllText($settingsPath, $json, (New-Object System.Text.UTF8Encoding($false)))
    Ok "Hooks installed in $settingsPath"
    Step "Stop -> green,  Notification -> amber"
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
