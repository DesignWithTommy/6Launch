# Records 15 seconds of what Windows' scheduler does while the game streams,
# to find what the game's memory-mapping calls wait for. Needs an
# administrator PowerShell. It waits for Claude's signal (the file trace\go),
# records, and writes trace\kernel.etl (about 1-2 GB; safe to delete after).
$ErrorActionPreference = "Stop"
$xperf = "C:\Program Files (x86)\Windows Kits\10\Windows Performance Toolkit\xperf.exe"
$dir = Join-Path $PSScriptRoot "trace"
New-Item -ItemType Directory -Force $dir | Out-Null
Remove-Item "$dir\go", "$dir\done", "$dir\kernel.etl" -ErrorAction SilentlyContinue

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole("Administrators")
if (-not $admin) { Write-Host "This window is not an administrator window. Open PowerShell with 'Run as administrator' and try again." -ForegroundColor Red; exit 1 }

Set-Content "$dir\ready" "ready"
Write-Host "Ready. Waiting for the game to reach the prison (Claude starts it; about 3 minutes)..."
while (-not (Test-Path "$dir\go")) { Start-Sleep -Milliseconds 500 }

Write-Host "Recording for 15 seconds..."
$ErrorActionPreference = "Continue"
$log = "$dir\xperf.log"
"start $(Get-Date -Format HH:mm:ss)" | Set-Content $log
# Windows has one "NT Kernel Logger"; another program on this PC holds it
# (the first try failed with "already exists"). Note whose it is, stop it,
# and use it for these 15 seconds. That program gets it back at its next start.
cmd /c "logman query `"NT Kernel Logger`" -ets 2>&1" | Add-Content $log
cmd /c "`"$xperf`" -stop 2>&1" | Add-Content $log
cmd /c "`"$xperf`" -on PROC_THREAD+LOADER+CSWITCH+DISPATCHER -stackwalk CSwitch+ReadyThread -f `"$dir\raw.etl`" -BufferSize 1024 -MinBuffers 256 -MaxBuffers 2048 2>&1" | Tee-Object -FilePath $log -Append
"on exit code $LASTEXITCODE" | Add-Content $log
Start-Sleep -Seconds 15
cmd /c "`"$xperf`" -d `"$dir\kernel.etl`" 2>&1" | Tee-Object -FilePath $log -Append
"stop exit code $LASTEXITCODE" | Add-Content $log
Remove-Item "$dir\raw.etl" -ErrorAction SilentlyContinue
Set-Content "$dir\done" "done"
Remove-Item "$dir\ready", "$dir\go" -ErrorAction SilentlyContinue
Write-Host "Done. You can close this window." -ForegroundColor Green
