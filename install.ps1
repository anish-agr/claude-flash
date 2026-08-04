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
    [switch]$NoRun
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $root 'bin\flash.exe'
$binDir = Join-Path $root 'bin'

function Step($text) { Write-Host "  $text" -ForegroundColor Gray }
function Ok($text) { Write-Host "  $text" -ForegroundColor Green }

Write-Host "`nClaudeFlash setup" -ForegroundColor Cyan
Write-Host ("-" * 50) -ForegroundColor DarkGray

# ---- 1. build ---------------------------------------------------------------
& (Join-Path $root 'build.ps1') | Out-Null
if (-not (Test-Path $exe)) { throw "Build did not produce $exe" }
Ok "Built bin\flash.exe"

# ---- 2. Win+R ---------------------------------------------------------------
# App Paths beats editing PATH: it takes effect instantly and Explorer needs no restart.
$appPaths = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\flash.exe'
New-Item -Path $appPaths -Force | Out-Null
Set-ItemProperty -Path $appPaths -Name '(Default)' -Value $exe
Set-ItemProperty -Path $appPaths -Name 'Path' -Value $binDir
Ok "Win+R -> 'flash' registered"

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
        $lnk.WorkingDirectory = $binDir
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

    function Set-FlashHook($hooks, [string]$Event, [string]$Command) {
        # Keep any hooks the user already had; replace only our own.
        $kept = @()
        if ($hooks.PSObject.Properties.Name -contains $Event) {
            $kept = @($hooks.$Event) | Where-Object {
                $_ -and (($_ | ConvertTo-Json -Depth 10 -Compress) -notmatch 'flash\.exe')
            }
        }
        $entry = [pscustomobject]@{
            hooks = @([pscustomobject]@{ type = 'command'; command = $Command })
        }
        $value = @($entry) + $kept
        if ($hooks.PSObject.Properties.Name -contains $Event) { $hooks.$Event = $value }
        else { $hooks | Add-Member -NotePropertyName $Event -NotePropertyValue $value }
    }

    Set-FlashHook $settings.hooks 'Stop'         ('"{0}" done --bg' -f $exe)
    Set-FlashHook $settings.hooks 'Notification' ('"{0}" ask --bg'  -f $exe)

    $json = $settings | ConvertTo-Json -Depth 32
    [System.IO.File]::WriteAllText($settingsPath, $json, (New-Object System.Text.UTF8Encoding($false)))
    Ok "Hooks installed in $settingsPath"
    Step "Stop -> green,  Notification -> amber"
}

# ---- done -------------------------------------------------------------------
Write-Host ("-" * 50) -ForegroundColor DarkGray
Write-Host "Ready.`n" -ForegroundColor Cyan
Write-Host "  Win+R  ->  flash          test it"          -ForegroundColor White
Write-Host "  Win+R  ->  flash toggle   on / off"         -ForegroundColor White
Write-Host "  Win+R  ->  flash status   what's it doing"  -ForegroundColor White
Write-Host ""
Write-Host "  Restart Claude Code for the hooks to load.`n" -ForegroundColor Yellow

if (-not $NoRun) { Start-Process -FilePath $exe -ArgumentList 'done' }
