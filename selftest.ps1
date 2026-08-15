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
$script:exePath = $exe
$settingsPath = Join-Path $env:USERPROFILE '.claude\settings.json'
$pass = 0; $fail = 0

function Check([string]$name, [bool]$ok, [string]$detail = '') {
    if ($ok) { $script:pass++; Write-Host ("  PASS  " + $name) -ForegroundColor Green }
    else { $script:fail++; Write-Host ("  FAIL  " + $name + $(if ($detail) { "  ($detail)" })) -ForegroundColor Red }
}

function Wait-Idle { while (Get-Process -Name flash -ErrorAction SilentlyContinue) { Start-Sleep -Milliseconds 40 } }

# A flash is only proven by an overlay actually rendering. The --bg parent is also
# called flash.exe and lives ~300ms, so process presence is not evidence.
$script:lastFlashFile = Join-Path $env:LOCALAPPDATA 'ClaudeFlash\lastflash'
function Get-FlashStamp { if (Test-Path $script:lastFlashFile) { [IO.File]::ReadAllText($script:lastFlashFile) } else { '' } }
function Wait-Flash([int]$ms) {
    $before = $script:stampBefore
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $ms) {
        if ((Get-FlashStamp) -ne $before) { Wait-Idle; return $true }
        Start-Sleep -Milliseconds 25
    }
    Wait-Idle
    return $false
}

# Hooks gated with --require_session only fire for sessions you actually typed into,
# so the harness has to register its fake session the way UserPromptSubmit would.
function Register-Session([string]$id) {
    ('{"session_id":"' + $id + '","hook_event_name":"UserPromptSubmit"}') |
        cmd.exe /c ('"' + $script:exePath + '" seen') | Out-Null
}

# Runs the hook the way Claude Code does and reports whether a flash appeared.
$script:hookSeq = 0
function Invoke-Hook([string]$command, [string]$mode) {
    Wait-Idle
    Register-Session 'selftest'
    # A unique tool_use_id per call, and no matching PostToolUse, so a hook using
    # --wait_prompt sees an unfinished call and flashes. The window has to exceed
    # that wait, hence 3s rather than 1.5s.
    $script:hookSeq++
    $id = "selftesthook$script:hookSeq"
    $json = '{"session_id":"selftest","permission_mode":"' + $mode +
            '","hook_event_name":"PreToolUse","tool_name":"Bash","tool_use_id":"' + $id +
            '","tool_input":{}}'
    $script:stampBefore = Get-FlashStamp
    $json | cmd.exe /c $command | Out-Null
    return (Wait-Flash 3500)
}

Write-Host "`nClaudeFlash self-test" -ForegroundColor Cyan
Write-Host ("-" * 58) -ForegroundColor DarkGray

# ---- 1. binary -------------------------------------------------------------
Write-Host "`nBinary" -ForegroundColor White
Check "flash.exe installed" (Test-Path $exe) $exe
# A shell can be missing WindowsApps from its own inherited PATH while the user PATH
# still has it, so fall back to checking the real registrations.
$wa = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps'
$onUserPath = (([Environment]::GetEnvironmentVariable('Path', 'User')) -split ';') -contains $wa
$appPaths = Test-Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\flash.exe'
Check "resolves as a bare command" ([bool](cmd.exe /c "where flash 2>nul") -or ($onUserPath -and $appPaths))

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
    Check "purple: PreToolUse/tools -> perm" $true
    Check "purple is gated on permission mode" ($permHook -match 'require_mode')
} else {
    Write-Host "  SKIP  purple not installed (run install.ps1 -PermissionFlash)" -ForegroundColor DarkYellow
}
Check "blue and purple coexist" (-not ($askHook -and -not $permHook -and $false) -and [bool]$askHook)

# ---- 3. flashes actually fire ----------------------------------------------
Write-Host "`nBehaviour" -ForegroundColor White
if ($stopHook) { Check "green fires" (Invoke-Hook $stopHook 'default') }
if ($askHook)  { Check "blue fires" (Invoke-Hook $askHook 'default') }
if ($askHook)  { Check "blue fires in bypass too (questions always need you)" (Invoke-Hook $askHook 'bypassPermissions') }

if ($permHook) {
    foreach ($m in 'default', 'plan', 'acceptEdits') {
        Check "purple fires in $m" (Invoke-Hook $permHook $m)
    }
    foreach ($m in 'auto', 'dontAsk', 'bypassPermissions') {
        Check "purple silent in $m" (-not (Invoke-Hook $permHook $m))
    }
}

# ---- 3b. approved vs blocked -------------------------------------------------
# The point of the permission flash: an approved call finishes on its own and must
# stay silent; a call still waiting on you must flash.
if ($permHook) {
    Write-Host "`nApproved vs blocked" -ForegroundColor White
    $pend = Join-Path $env:LOCALAPPDATA 'ClaudeFlash\pending'
    $markHook = $null
    foreach ($e in @($h.PostToolUse)) {
        foreach ($cmd in @($e.hooks | ForEach-Object { $_.command })) {
            if ($cmd -match 'flash\.exe.*\bmark\b') { $markHook = $cmd; break }
        }
        if ($markHook) { break }
    }
    Check "PostToolUse mark hook registered" ([bool]$markHook)

    function Clear-Pending {
        if ([IO.Directory]::Exists($pend)) {
            foreach ($f in [IO.Directory]::GetFiles($pend)) { try { [IO.File]::Delete($f) } catch {} }
        }
    }
    function Invoke-Pair([string]$id, [bool]$complete) {
        Clear-Pending; Wait-Idle
        Register-Session 'selftest'
        $pre = '{"session_id":"selftest","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_use_id":"' + $id + '"}'
        $script:stampBefore = Get-FlashStamp
        $pre | cmd.exe /c $permHook | Out-Null
        if ($complete -and $markHook) {
            Start-Sleep -Milliseconds 300
            ('{"hook_event_name":"PostToolUse","tool_name":"Bash","tool_use_id":"' + $id + '"}') | cmd.exe /c $markHook | Out-Null
        }
        $seen = Wait-Flash 3500
        Clear-Pending
        return $seen
    }

    Check "silent when the call completes (already approved)" (-not (Invoke-Pair 'selftestA' $true))
    Check "flashes when the call is still waiting (prompt up)" (Invoke-Pair 'selftestB' $false)
    Check "no marker files left behind" (@([IO.Directory]::GetFiles($pend)).Count -eq 0)
}

# ---- 4. enable / disable ----------------------------------------------------
Write-Host "`nOn/off switch" -ForegroundColor White
Start-Process $exe -ArgumentList 'off' -Wait
Check "disabled suppresses flashes" (-not (Invoke-Hook ('"' + $exe + '" done --bg') 'default'))
Start-Process $exe -ArgumentList 'on' -Wait
Wait-Idle
Check "re-enabled flashes again" (Invoke-Hook ('"' + $exe + '" done --bg') 'default')

# ---- 4b. the kill switch under load -----------------------------------------
# A script spawning sessions back to back fires many hooks at once. File.Exists
# reports every failure as "missing", so a contended read was enough to read "off"
# as "on" and flash anyway - which made the switch look like it did nothing.
Write-Host "`nKill switch under concurrency" -ForegroundColor White
Start-Process $exe -ArgumentList 'off' -Wait
Wait-Idle
$script:stampBefore = Get-FlashStamp
$stressJobs = 1..20 | ForEach-Object {
    Start-Job -ScriptBlock {
        param($e)
        '{"session_id":"selftest","hook_event_name":"Stop"}' | cmd.exe /c ('"' + $e + '" done --bg') | Out-Null
    } -ArgumentList $exe
}
$stressJobs | Wait-Job -Timeout 90 | Out-Null
$stressJobs | Remove-Job -Force
Start-Sleep -Milliseconds 1500
Check "20 concurrent hooks stay silent while off" ((Get-FlashStamp) -eq $script:stampBefore)
Start-Process $exe -ArgumentList 'on' -Wait
Wait-Idle

# ---- 5. no crashes ----------------------------------------------------------
Write-Host "`nHealth" -ForegroundColor White
$errLog = Join-Path $env:LOCALAPPDATA 'ClaudeFlash\error.log'
Check "no errors logged" (-not (Test-Path $errLog)) $(if (Test-Path $errLog) { 'see ' + $errLog })
Check "no stray flash processes" (@(Get-Process -Name flash -ErrorAction SilentlyContinue).Count -eq 0)

Write-Host ("-" * 58) -ForegroundColor DarkGray
if ($fail -eq 0) { Write-Host "$pass passed, 0 failed`n" -ForegroundColor Green }
else { Write-Host "$pass passed, $fail FAILED`n" -ForegroundColor Red }
exit $fail
