# Undoes install.ps1: removes the hooks, the Win+R registration and the desktop
# shortcuts. Leaves the repo and your config.ini alone.

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

function Ok($text) { Write-Host "  $text" -ForegroundColor Green }

Write-Host "`nRemoving ClaudeFlash" -ForegroundColor Cyan
Write-Host ("-" * 50) -ForegroundColor DarkGray

# ---- hooks ------------------------------------------------------------------
$settingsPath = Join-Path $env:USERPROFILE '.claude\settings.json'
if (Test-Path $settingsPath) {
    Copy-Item $settingsPath "$settingsPath.bak-$(Get-Date -Format yyyyMMdd-HHmmss)"
    $settings = Get-Content $settingsPath -Raw | ConvertFrom-Json

    if ($settings.hooks) {
        foreach ($event in @($settings.hooks.PSObject.Properties.Name)) {
            $kept = @($settings.hooks.$event) | Where-Object {
                $_ -and (($_ | ConvertTo-Json -Depth 10 -Compress) -notmatch 'flash\.exe')
            }
            if ($kept.Count -eq 0) { $settings.hooks.PSObject.Properties.Remove($event) }
            else { $settings.hooks.$event = $kept }
        }
    }

    $json = $settings | ConvertTo-Json -Depth 32
    [System.IO.File]::WriteAllText($settingsPath, $json, (New-Object System.Text.UTF8Encoding($false)))
    Ok "Hooks removed from settings.json"
}

# ---- the installed binary ---------------------------------------------------
Get-Process -Name flash -ErrorAction SilentlyContinue | ForEach-Object {
    try { $_.Kill(); $_.WaitForExit(2000) } catch { }
}
$installed = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\flash.exe'
if (Test-Path $installed) { Remove-Item $installed -Force; Ok "Removed $installed" }

# ---- Win+R ------------------------------------------------------------------
$appPaths = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\flash.exe'
if (Test-Path $appPaths) {
    Remove-Item $appPaths -Recurse -Force
    Ok "Win+R registration removed"
}

# ---- shortcuts --------------------------------------------------------------
$desktop = [Environment]::GetFolderPath('Desktop')
foreach ($name in 'Claude Flash (test).lnk', 'Claude Flash on-off.lnk') {
    $lnk = Join-Path $desktop $name
    if (Test-Path $lnk) { Remove-Item $lnk -Force; Ok "Removed $name" }
}

Write-Host ("-" * 50) -ForegroundColor DarkGray
Write-Host "Done. Restart Claude Code to drop the hooks.`n" -ForegroundColor Cyan
Write-Host "Settings still on disk (delete by hand if you want them gone):" -ForegroundColor DarkGray
Write-Host "  $env:LOCALAPPDATA\ClaudeFlash`n" -ForegroundColor DarkGray
