# keep_session_on_console.ps1 - for a Windows computer that runs Face Recognizer and is used through Remote Desktop
# (a server in a cupboard, a NAS).
#
# A Remote Desktop session, connected or disconnected, has none of the computer's own sound devices: the app there
# finds no microphone and no speakers (Settings > Music: "No audio interface"), and its page is drawn only now and
# then (the camera's stream to your other devices paused about 0.4 s every 11 to 17 s, measured 2026-11-15). On the
# computer's own console both are as on any PC. This puts your session back on the console whenever you
# disconnect from Remote Desktop, and locks it there, so the computer's own screen shows the lock screen and the
# app keeps its camera, microphone and speakers behind it.
#
#   powershell -ExecutionPolicy Bypass -File keep_session_on_console.ps1 -Install     (as an administrator, once)
#   powershell -ExecutionPolicy Bypass -File keep_session_on_console.ps1 -Uninstall
#   powershell -ExecutionPolicy Bypass -File keep_session_on_console.ps1 -Now         (once, now; as SYSTEM)
#
# -Install copies this script to %ProgramData%\FaceRecognizer and makes two scheduled tasks:
#   "Face Recognizer: session to console"  as SYSTEM, each time a Remote Desktop session is disconnected (and at
#                                          start): runs this script with -Now
#   "Face Recognizer: lock the console"    as the user who installed it: locks the screen (started by the first)
# Nothing else is changed. With no user signed in, nothing happens.
param([switch]$Install, [switch]$Uninstall, [switch]$Now, [string]$User)
$ErrorActionPreference = 'Stop'
$move, $lock = 'Face Recognizer: session to console', 'Face Recognizer: lock the console'
$home_ = Join-Path $env:ProgramData 'FaceRecognizer'
$here = Join-Path $home_ 'keep_session_on_console.ps1'

function Sessions {                       # [{user, id, state, console}] from quser
  foreach ($line in (quser 2>$null | Select-Object -Skip 1)) {
    $t = ($line -replace '^[ >]', '') -replace '\s{2,}', '|'
    $f = $t.Split('|')
    if ($f.Count -ge 4) {
      $hasName = $f[1] -notmatch '^\d+$'            # a disconnected session has no session name
      [pscustomobject]@{ user = $f[0].Trim(); id = [int]$(if ($hasName) { $f[2] } else { $f[1] })
                         state = $(if ($hasName) { $f[3] } else { $f[2] }); console = ($hasName -and $f[1] -eq 'console') }
    }
  }
}

if ($Uninstall) {
  foreach ($t in $move, $lock) { Unregister-ScheduledTask -TaskName $t -Confirm:$false -ErrorAction SilentlyContinue }
  Remove-Item $here -ErrorAction SilentlyContinue
  "Removed. Your session stays where it is until you next disconnect."
  exit 0
}

if ($Install) {
  $me = if ($User) { $User } else { [Security.Principal.WindowsIdentity]::GetCurrent().Name }
  New-Item -ItemType Directory $home_ -Force | Out-Null
  Copy-Item $PSCommandPath $here -Force
  $ps = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
  # 1. lock the console, in the user's own session
  Register-ScheduledTask -TaskName $lock -Force `
    -Action (New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\rundll32.exe" -Argument 'user32.dll,LockWorkStation') `
    -Principal (New-ScheduledTaskPrincipal -UserId $me -LogonType Interactive) `
    -Settings (New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 1) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries) | Out-Null
  # 2. on every Remote Desktop disconnect (event 24 of the local session manager), and once at start
  $class = Get-CimClass -Namespace Root/Microsoft/Windows/TaskScheduler -ClassName MSFT_TaskEventTrigger
  $onDisconnect = New-CimInstance -CimClass $class -ClientOnly
  $onDisconnect.Subscription = '<QueryList><Query Id="0" Path="Microsoft-Windows-TerminalServices-LocalSessionManager/Operational">' +
    '<Select Path="Microsoft-Windows-TerminalServices-LocalSessionManager/Operational">*[System[(EventID=24)]]</Select></Query></QueryList>'
  $onDisconnect.Delay = 'PT5S'
  $onDisconnect.Enabled = $true
  Register-ScheduledTask -TaskName $move -Force `
    -Action (New-ScheduledTaskAction -Execute $ps -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$here`" -Now") `
    -Trigger $onDisconnect, (New-ScheduledTaskTrigger -AtStartup) `
    -Principal (New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest) `
    -Settings (New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 2) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew) | Out-Null
  "Installed for $me. From now on a disconnected Remote Desktop session goes back to this computer's console, locked."
  "Sessions now:"; Sessions | ForEach-Object { "   $($_.user): session $($_.id), $($_.state)$(if ($_.console) { ', on the console' })" }
  exit 0
}

if ($Now) {
  $log = Join-Path $home_ 'session-to-console.log'
  $all = @(Sessions)
  $gone = @($all | Where-Object { $_.state -eq 'Disc' })
  if ($all | Where-Object { $_.console -and $_.state -eq 'Active' }) { exit 0 }      # someone is at the console already
  if (-not $gone) { exit 0 }                                                         # nobody signed in, or still connected
  $s = $gone[0]
  & "$env:SystemRoot\System32\tscon.exe" $s.id /dest:console
  $ok = $LASTEXITCODE -eq 0
  Start-Sleep 3
  if ($ok) { Start-ScheduledTask -TaskName $lock -ErrorAction SilentlyContinue }
  $line = "{0} session {1} of {2}: {3}" -f (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'), $s.id, $s.user,
    $(if ($ok) { 'moved to the console and locked' } else { "tscon failed ($LASTEXITCODE)" })
  try { Add-Content $log $line; if ((Get-Item $log).Length -gt 200kb) { Set-Content $log (Get-Content $log -Tail 200) } } catch { }
  exit $(if ($ok) { 0 } else { 1 })
}

"Say -Install, -Uninstall or -Now (see the top of this file)."
