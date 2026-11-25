# Makes the Android app's release key, once: the key every release of the app is signed with (for Google Play it
# is the upload key). Whoever has it can publish updates of the app, so it is kept like a password:
#
#   %LOCALAPPDATA%\Android\release\facerecognizer.keystore      the key (PKCS #12)
#   %LOCALAPPDATA%\Android\release\facerecognizer.password      its password, protected for this Windows user
#
# and, with -Backup <folder>, a copy of both in a folder only the owner can open (there the password is readable:
# a backup has to work on another computer). Neither the key nor the password is printed; what is printed is the
# certificate's SHA-256, which is public (it is in every package) and is what Google asks for.
#
#   pwsh apps\android\make_key.ps1 [-Backup <folder>]        (run again: says what is there, changes nothing)
param([string]$Backup = '')
$ErrorActionPreference = 'Stop'
$a = if ($env:FACE_ANDROID_HOME) { $env:FACE_ANDROID_HOME } else { "$env:LOCALAPPDATA\Android" }
$dir = "$a\release"; $ks = "$dir\facerecognizer.keystore"; $pwFile = "$dir\facerecognizer.password"
$keytool = Join-Path (Get-ChildItem "$a\jdk" -Directory | Select-Object -First 1).FullName 'bin\keytool.exe'
Add-Type -AssemblyName System.Security
if (-not (Test-Path $ks)) {
    New-Item -ItemType Directory -Force $dir | Out-Null
    $raw = [byte[]]::new(24); [Security.Cryptography.RandomNumberGenerator]::Fill($raw)
    $env:FACE_KEY_PASSWORD = [Convert]::ToBase64String($raw) -replace '[+/=]', 'k'
    # 4096-bit RSA, valid for 30 years (Google Play wants a key that outlives 2033)
    & $keytool -genkeypair -keystore $ks -storetype PKCS12 -storepass:env FACE_KEY_PASSWORD -alias facerecognizer `
        -keyalg RSA -keysize 4096 -validity 10950 -dname 'CN=Face Recognizer' 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $ks)) { throw 'the key was not made' }
    $protected = [Security.Cryptography.ProtectedData]::Protect([Text.Encoding]::UTF8.GetBytes($env:FACE_KEY_PASSWORD), $null, 'CurrentUser')
    [IO.File]::WriteAllBytes($pwFile, $protected)
    "made: $ks"
} else {
    $env:FACE_KEY_PASSWORD = [Text.Encoding]::UTF8.GetString([Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($pwFile), $null, 'CurrentUser'))
    "already there: $ks"
}
$said = & $keytool -list -v -keystore $ks -storepass:env FACE_KEY_PASSWORD -alias facerecognizer 2>&1
"certificate SHA-256: " + (($said | Select-String -Pattern 'SHA256:\s*(\S+)').Matches[0].Groups[1].Value)
"valid until: " + (($said | Select-String -Pattern 'until:\s*(.+)$').Matches[0].Groups[1].Value)
if ($Backup) {
    New-Item -ItemType Directory -Force $Backup | Out-Null
    Copy-Item $ks (Join-Path $Backup 'facerecognizer.keystore') -Force
    Set-Content (Join-Path $Backup 'facerecognizer.password.txt') $env:FACE_KEY_PASSWORD -NoNewline
    "copied to the backup: " + ((Get-FileHash $ks).Hash -eq (Get-FileHash (Join-Path $Backup 'facerecognizer.keystore')).Hash)
}
Remove-Item Env:FACE_KEY_PASSWORD
