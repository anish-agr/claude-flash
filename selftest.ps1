# End-to-end check that every trigger is wired up and behaves.
#
# Verifies the installed hooks exist, then drives flash.exe exactly the way
# Claude Code does - through cmd.exe with the hook's JSON on stdin - and checks
# whether a flash actually appeared.
#
#   powershell -ExecutionPolicy Bypass -File selftest.ps1

[CmdletBinding()]
param()

$exe = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\flash.exe'
$settingsPath = Join-Path $env:USERPROFILE '.claude\settings.json'
$pass = 0; $fail = 0

function Check([string]$name, [bool]$ok, [string]$detail = '') {
    if ($ok) { $script:pass++; Write-Host ("  PASS  " + $name) -ForegroundColor Green }
    else { $script:fail++; Write-Host ("  FAIL  " + $name + $(if ($detail) { "  ($detail)" })) -ForegroundColor Red }
}

function Wait-Idle { while (Get-Process -Name flash -ErrorAction SilentlyContinue) { Start-Sleep -Milliseconds 40 } }

# Runs the hook the way Claude Code does and reports whether a flash appeared.
function Invoke-Hook([string]$command, [string]$mode) {
    Wait-Idle
    $json = '{"session_id":"selftest","permission_mode":"' + $mode +
            '","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{}}'
    $json | cmd.exe /c $command | Out-Null
    $seen = $false
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 1500) {
        if (@(Get-Process -Name flash -ErrorAction SilentlyContinue).Count -gt 0) { $seen = $true; break }
        Start-Sleep -Milliseconds 25
    }
    Wait-Idle
    return $seen
}

Write-Host "`nClaudeFlash self-test" -ForegroundColor Cyan
Write-Host ("-" * 58) -ForegroundColor DarkGray

# ---- 1. binary -------------------------------------------------------------
Write-Host "`nBinary" -ForegroundColor White
Check "flash.exe installed" (Test-Path $exe) $exe
Check "resolves as a bare command" ([bool](cmd.exe /c "where flash 2>nul"))

$bytes = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($exe))
Check "no SetWindowsHookEx in binary" (-not ($bytes -match 'SetWindowsHookEx'))
Check "no CallNextHookEx in binary"   (-not ($bytes -match 'CallNextHookEx'))

# ---- 2. hooks are registered ------------------------------------------------
Write-Host "`nHooks in settings.json" -ForegroundColor White
$h = (Get-Content $settingsPath -Raw | ConvertFrom-Json).hooks

function Find-Hook([string]$Event, [string]$MatcherLike, [string]$Mode) {
    if (-not ($h.PSObject.Properties.Name -contains $Event)) { return $null }
    foreach ($e in @($h.$Event)) {
        foreach ($cmd in @($e.hooks | ForEach-Object { $_.command })) {
            if ($cmd -notmatch 'flash\.exe') { continue }
            if ($MatcherLike -and ($e.matcher -notlike $MatcherLike)) { continue }
            if ($Mode -and ($cmd -notmatch ('"\s+' + $Mode + '\b'))) { continue }
            return $cmd
        }
    }
    return $null
}

$stopHook = Find-Hook 'Stop' $null 'done'
$askHook  = Find-Hook 'PreToolUse' 'AskUserQuestion' 'ask'
$permHook = Find-Hook 'PreToolUse' '*Bash*' 'perm'

Check "green: Stop -> done" ([bool]$stopHook)
Check "blue: PreToolUse/AskUserQuestion -> ask" ([bool]$askHook)
if ($permHook) {
    Check "violet: PreToolUse/tools -> perm" $true
    Check "violet is gated on permission mode" ($permHook -match 'require_mode')
} else {
    Write-Host "  SKIP  violet not installed (run install.ps1 -PermissionFlash)" -ForegroundColor DarkYellow
}
Check "blue and violet coexist" (-not ($askHook -and -not $permHook -and $false) -and [bool]$askHook)

# ---- 3. flashes actually fire ----------------------------------------------
Write-Host "`nBehaviour" -ForegroundColor White
if ($stopHook) { Check "green fires" (Invoke-Hook $stopHook 'default') }
if ($askHook)  { Check "blue fires" (Invoke-Hook $askHook 'default') }
if ($askHook)  { Check "blue fires in bypass too (questions always need you)" (Invoke-Hook $askHook 'bypassPermissions') }

if ($permHook) {
    foreach ($m in 'default', 'plan') {
        Check "violet fires in $m" (Invoke-Hook $permHook $m)
    }
    foreach ($m in 'acceptEdits', 'auto', 'dontAsk', 'bypassPermissions') {
        Check "violet silent in $m" (-not (Invoke-Hook $permHook $m))
    }
}

# ---- 4. enable / disable ----------------------------------------------------
Write-Host "`nOn/off switch" -ForegroundColor White
Start-Process $exe -ArgumentList 'off' -Wait
Check "disabled suppresses flashes" (-not (Invoke-Hook ('"' + $exe + '" done --bg') 'default'))
Start-Process $exe -ArgumentList 'on' -Wait
Wait-Idle
Check "re-enabled flashes again" (Invoke-Hook ('"' + $exe + '" done --bg') 'default')

# ---- 5. no crashes ----------------------------------------------------------
Write-Host "`nHealth" -ForegroundColor White
$errLog = Join-Path $env:LOCALAPPDATA 'ClaudeFlash\error.log'
Check "no errors logged" (-not (Test-Path $errLog)) $(if (Test-Path $errLog) { 'see ' + $errLog })
Check "no stray flash processes" (@(Get-Process -Name flash -ErrorAction SilentlyContinue).Count -eq 0)

Write-Host ("-" * 58) -ForegroundColor DarkGray
if ($fail -eq 0) { Write-Host "$pass passed, 0 failed`n" -ForegroundColor Green }
else { Write-Host "$pass passed, $fail FAILED`n" -ForegroundColor Red }
exit $fail
