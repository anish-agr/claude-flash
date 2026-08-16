# Adds or removes the ClaudeFlash hooks in ~/.claude/settings.json.
#
#   powershell -ExecutionPolicy Bypass -File hooks.ps1 -Off
#   powershell -ExecutionPolicy Bypass -File hooks.ps1 -On
#
# `flash off` stops the overlay drawing, but the hooks still fire and still launch
# a process per event. When a script is spawning sessions back to back that is a
# lot of churn, and anything odd it causes is hard to tell apart from the flash
# itself. Removing the hooks means nothing runs at all.
#
# Hooks are read once when a session starts, so this affects sessions started
# afterwards - which is exactly the case when a script keeps making new ones.

[CmdletBinding(DefaultParameterSetName = 'Status')]
param(
    [Parameter(ParameterSetName = 'Off')][switch]$Off,
    [Parameter(ParameterSetName = 'On')][switch]$On
)

$ErrorActionPreference = 'Stop'
$settingsPath = Join-Path $env:USERPROFILE '.claude\settings.json'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path

function Count-FlashHooks {
    if (-not (Test-Path $settingsPath)) { return 0 }
    $s = Get-Content $settingsPath -Raw | ConvertFrom-Json
    if (-not $s.hooks) { return 0 }
    $n = 0
    foreach ($ev in $s.hooks.PSObject.Properties.Name) {
        foreach ($e in @($s.hooks.$ev)) {
            if (($e | ConvertTo-Json -Depth 10 -Compress) -match 'flash\.exe') { $n++ }
        }
    }
    return $n
}

if ($Off) {
    if (-not (Test-Path $settingsPath)) { Write-Host "no settings.json"; exit 0 }
    Copy-Item $settingsPath "$settingsPath.bak-$(Get-Date -Format yyyyMMdd-HHmmss)"
    $s = Get-Content $settingsPath -Raw | ConvertFrom-Json
    if ($s.hooks) {
        foreach ($ev in @($s.hooks.PSObject.Properties.Name)) {
            $keep = @($s.hooks.$ev) | Where-Object {
                $_ -and (($_ | ConvertTo-Json -Depth 10 -Compress) -notmatch 'flash\.exe')
            }
            if ($keep.Count -eq 0) { $s.hooks.PSObject.Properties.Remove($ev) }
            else { $s.hooks.$ev = $keep }
        }
    }
    [System.IO.File]::WriteAllText($settingsPath, ($s | ConvertTo-Json -Depth 32),
        (New-Object System.Text.UTF8Encoding($false)))
    Write-Host "ClaudeFlash hooks removed. Sessions started from now on will not run it." -ForegroundColor Green
    Write-Host "Put them back with: hooks.ps1 -On" -ForegroundColor DarkGray
}
elseif ($On) {
    & (Join-Path $root 'install.ps1') -NoRun -NoShortcuts -PermissionFlash | Out-Null
    Write-Host "ClaudeFlash hooks restored ($(Count-FlashHooks) entries)." -ForegroundColor Green
    Write-Host "Restart Claude Code, or just start a new session." -ForegroundColor DarkGray
}
else {
    $n = Count-FlashHooks
    if ($n -gt 0) { Write-Host "hooks: INSTALLED ($n entries)" -ForegroundColor Green }
    else { Write-Host "hooks: not installed" -ForegroundColor Yellow }
    Write-Host "  turn off : hooks.ps1 -Off"
    Write-Host "  turn on  : hooks.ps1 -On"
}
