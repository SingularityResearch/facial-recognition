# keep_session_on_console.ps1 - for a Windows computer that runs Face Recognizer and is used through Remote Desktop
# (a server in a cupboard, a NAS).
#
# A Remote Desktop session, connected or disconnected, has none of the computer's own sound devices: the app there
# finds no microphone and no speakers (Settings > Music: "No audio interface"), and its page is drawn only now and
# then (the camera's stream to your other devices paused about 0.4 s every 11 to 17 s, measured 2026-11-15). On the
# computer's own console both are as on any PC. This puts your session back on the console whenever you
# disconnect from Remote Desktop.
#
# READ THIS FIRST: the session arrives on the console UNLOCKED, and has to stay so. A session locked at the
# computer's own screen gets no camera from Windows: the app's picture froze 20 s after the console was locked,
# and a camera started behind the lock screen never started (both seen 2026-11-15; an earlier version of this
# script locked the console and so stopped the camera). So anyone who can reach this computer's own screen and
# keyboard is in your session. Use it only where that is acceptable (a locked room, a computer with no screen and
# keyboard attached).
# Without it the camera still works in a disconnected Remote Desktop session, with the two limits above: a
# disconnect locks the session and Windows stops the camera 20 s later; the app (1.23.0 on) then starts itself
# again, once, and has its camera back.
#
#   powershell -ExecutionPolicy Bypass -File keep_session_on_console.ps1 -Install     (as an administrator, once)
#   powershell -ExecutionPolicy Bypass -File keep_session_on_console.ps1 -Uninstall
#   powershell -ExecutionPolicy Bypass -File keep_session_on_console.ps1 -Now         (once, now; as SYSTEM)
#
# -Install copies this script to %ProgramData%\FaceRecognizer and makes one scheduled task:
#   "Face Recognizer session to console"   as SYSTEM, each time a Remote Desktop session is disconnected (and at
#                                          start): runs this script with -Now
# Nothing else is changed. With no user signed in, nothing happens. A session that is locked stays locked when it
# is moved: sign in once (at the computer, or connect with Remote Desktop and disconnect) to have the camera again.
param([switch]$Install, [switch]$Uninstall, [switch]$Now)
$ErrorActionPreference = 'Stop'
$move = 'Face Recognizer session to console'                # no colon: not allowed in a task's name
$old = 'Face Recognizer lock the console'                   # made by the first version of this script: removed
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
  foreach ($t in $move, $old) { Unregister-ScheduledTask -TaskName $t -Confirm:$false -ErrorAction SilentlyContinue }
  Remove-Item $here -ErrorAction SilentlyContinue
  "Removed. Your session stays where it is until you next disconnect."
  exit 0
}

if ($Install) {
  New-Item -ItemType Directory $home_ -Force | Out-Null
  Copy-Item $PSCommandPath $here -Force
  Unregister-ScheduledTask -TaskName $old -Confirm:$false -ErrorAction SilentlyContinue
  $ps = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
  # on every Remote Desktop disconnect (event 24 of the local session manager), and once at start
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
  "Installed. From now on a disconnected Remote Desktop session goes back to this computer's console, unlocked:"
  "whoever can reach its screen and keyboard is in that session. Locked there, it gets no camera from Windows."
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
  $line = "{0} session {1} of {2}: {3}" -f (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'), $s.id, $s.user,
    $(if ($ok) { 'moved to the console' } else { "tscon failed ($LASTEXITCODE)" })
  try { Add-Content $log $line; if ((Get-Item $log).Length -gt 200kb) { Set-Content $log (Get-Content $log -Tail 200) } } catch { }
  exit $(if ($ok) { 0 } else { 1 })
}

"Say -Install, -Uninstall or -Now (see the top of this file)."
