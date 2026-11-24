# Builds the app's Windows installer (FaceRecognizer-Setup-x64.exe, in target\dist):
#
#     powershell -File packaging\windows\build.ps1 [-Trial] [-NoBuild] [-Sign] [-Output <folder>] [-WebView2 <file>]
#
# The program is built (cargo build -p face-desktop --release), what it runs with is fetched where it isn't there
# yet (face-tools get runtime ffmpeg heif: each download pinned by size and SHA-256, the HEIC libraries built from
# their pinned sources), Microsoft's WebView2 bootstrapper is fetched (below), and Inno Setup 6 packs all of it
# (FaceRecognizer.iss). -Trial makes the installer that installs beside the app, under another name (see the
# .iss). -NoBuild packs what is built already. -Output names another folder for the installer.
# -Sign signs the program, has Setup sign the uninstaller it packs and itself, and then reads both files'
# signatures as Windows does (sign.ps1: with what the environment names; CI does it on main). Beside the
# installer it makes FaceRecognizer-win64.zip: the program and its runtime folder, for use without installing.
#
# The WebView2 bootstrapper (MicrosoftEdgeWebview2Setup.exe, 1.8 MB): the app's window is shown by Microsoft's
# WebView2 runtime, which is part of Windows 11 and on nearly every Windows 10, but not on all (a Windows 10 never
# updated, Windows Server, the LTSC editions). Nothing may be assumed to be installed, so Setup carries
# Microsoft's own small installer for it and, only on a computer without the runtime and after asking, runs it:
# it downloads the runtime from Microsoft. Microsoft has one address for the newest bootstrapper and no address
# for a given one, so it is pinned here by size and SHA-256 like every other download, and it must carry
# Microsoft's valid signature. When Microsoft puts out a newer one the pin no longer fits: this script then stops
# and says what it got; look at that, and put the new size and SHA-256 here.
# -WebView2 <file> packs another file in its place: only with -Trial, for the installer's check, which never
# runs Microsoft's (a stand-in that notes how it was started).
param([switch]$Trial, [switch]$NoBuild, [switch]$Sign, [string]$Output, [string]$WebView2)
$ErrorActionPreference = "Stop"
$BootstrapperUrl = "https://go.microsoft.com/fwlink/p/?LinkId=2124703"
$BootstrapperSize = 1826528
$BootstrapperSha256 = "fc2972da71d43e0c57b8d85a5cb9b200ed221f723cf43052d08ca722eb0bb22e"
$rust = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$root = Join-Path $rust "target\root"

function Test-Pinned($file) {
    (Test-Path $file) -and ((Get-Item $file).Length -eq $BootstrapperSize) -and ((Get-FileHash $file -Algorithm SHA256).Hash -eq $BootstrapperSha256)
}

# Microsoft's bootstrapper as pinned above, fetched unless it is here already. The path of the file.
function Get-Bootstrapper {
    $file = Join-Path $rust "target\dist\webview2\MicrosoftEdgeWebview2Setup.exe"
    if (-not (Test-Pinned $file)) {
        New-Item -ItemType Directory -Force (Split-Path $file) | Out-Null
        $new = "$file.part"
        Invoke-WebRequest -Uri $BootstrapperUrl -OutFile $new -UseBasicParsing
        if (-not (Test-Pinned $new)) {
            $signed = Get-AuthenticodeSignature $new
            $got = "{0} bytes, SHA-256 {1}, signature {2} of {3}" -f (Get-Item $new).Length, (Get-FileHash $new -Algorithm SHA256).Hash.ToLower(), $signed.Status, $signed.SignerCertificate.Subject
            Remove-Item $new
            throw "Microsoft's WebView2 bootstrapper isn't the pinned file any more (it came as: $got). If that is Microsoft's newer one, put its size and SHA-256 into build.ps1."
        }
        Move-Item -Force $new $file
    }
    $signed = Get-AuthenticodeSignature $file
    if ($signed.Status -ne "Valid" -or $signed.SignerCertificate.Subject -notmatch "(^|, )O=Microsoft Corporation(,|$)") {
        throw "the WebView2 bootstrapper doesn't carry Microsoft's valid signature ($($signed.Status))"
    }
    $file
}

Push-Location $rust
try {
    if ($WebView2 -and -not $Trial) { throw "-WebView2 packs another file in the bootstrapper's place: only with -Trial" }
    if (-not $NoBuild) {
        cargo build -p face-desktop -p face-tools --release
        if ($LASTEXITCODE -ne 0) { throw "the program didn't build" }
        & (Join-Path $rust "target\release\face-tools.exe") get --root $root runtime ffmpeg heif
        if ($LASTEXITCODE -ne 0) { throw "what the program runs with couldn't be fetched" }
    }
    $bootstrapper = if ($WebView2) { (Resolve-Path $WebView2).Path } else { Get-Bootstrapper }
    $version = (Select-String -Path (Join-Path $rust "Cargo.toml") -Pattern '^version = "(.*)"$' | Select-Object -First 1).Matches[0].Groups[1].Value
    # (Windows takes a version of numbers only: 2.6.0-dev is 2.6.0 to it)
    $number = ($version -replace '[^0-9.].*$', '')
    $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) { throw "Inno Setup 6 isn't installed (ISCC.exe not found)" }
    $dist = if ($Output) { $Output } else { Join-Path $rust "target\dist" }
    $defines = @("/DAppVersion=$number", "/DWebView2=$bootstrapper", "/DOutput=$dist", "/Q")
    if ($Trial) { $defines += "/DTrial=1" }
    $program = Join-Path $rust "target\release\FaceRecognizer.exe"
    $signer = Join-Path $PSScriptRoot "sign.ps1"
    $runtime = Join-Path $root "runtime"          # what Setup and the zip take (signed copies of it with -Sign)
    if ($Sign) {
        # the program first (unless it is signed already): Setup then packs the signed file
        if ((Get-AuthenticodeSignature -LiteralPath $program).Status -ne "Valid") {
            & $signer -Files $program
        }
        # Setup signs the uninstaller it packs and itself with the same script ($f: the file, $q: a quote)
        $defines += @("/DSign=1", ('/Ssign=powershell.exe -NoProfile -ExecutionPolicy Bypass -File $q' + $signer + '$q -Files $f'))
        # What the program runs with goes into Setup and the zip signed too, every program and library of it: the
        # Microsoft Store takes an installer only when all of them are, and Windows trusts a signed file more.
        # Microsoft's own files are signed already and stay as they are; the project's own builds (FFmpeg, the HEIC
        # libraries) are signed here, as copies: the fetched files count only as the pinned bytes, and what an
        # installer brings counts by its names.
        $runtime = Join-Path $dist "runtime-signed"
        if (Test-Path $runtime) { Remove-Item -Recurse -Force $runtime }
        New-Item -ItemType Directory -Force $runtime | Out-Null
        Copy-Item (Join-Path $root "runtime\*") $runtime -Recurse
        $programs = { @(Get-ChildItem $runtime -Recurse -File -Include *.exe, *.dll) }
        $unsigned = @(& $programs | Where-Object { (Get-AuthenticodeSignature -LiteralPath $_.FullName).Status -ne "Valid" } | ForEach-Object FullName)
        if ($unsigned) { & $signer -Files $unsigned }
        $left = @(& $programs | Where-Object { (Get-AuthenticodeSignature -LiteralPath $_.FullName).Status -ne "Valid" })
        if ($left) { throw "not every program and library in runtime carries a valid signature: $($left.Name -join ', ')" }
        "runtime: $($unsigned.Count) of its $((& $programs).Count) programs and libraries signed here, the others were signed already"
    }
    $defines += "/DRuntime=$runtime"
    & $iscc @defines (Join-Path $PSScriptRoot "FaceRecognizer.iss")
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup stopped with $LASTEXITCODE" }
    $name = if ($Trial) { "FaceRecognizer-Trial-Setup-x64.exe" } else { "FaceRecognizer-Setup-x64.exe" }
    $made = Get-Item (Join-Path $dist $name)
    "{0}: {1:N0} MB" -f $made.Name, ($made.Length / 1MB)
    if ($Sign) { & $signer -Check $program, $made.FullName }
    if (-not $Trial) {
        # the program with what it runs with, as a folder to unpack anywhere
        $zip = Join-Path $dist "FaceRecognizer-win64.zip"
        $stage = Join-Path $dist "FaceRecognizer"
        if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
        New-Item -ItemType Directory -Force (Join-Path $stage "runtime") | Out-Null
        Copy-Item $program $stage
        Copy-Item (Join-Path $PSScriptRoot "README.txt") $stage
        Copy-Item (Join-Path $runtime "*") (Join-Path $stage "runtime") -Recurse
        if (Test-Path $zip) { Remove-Item $zip }
        Compress-Archive -Path $stage -DestinationPath $zip
        Remove-Item -Recurse -Force $stage
        "{0}: {1:N0} MB" -f (Split-Path $zip -Leaf), ((Get-Item $zip).Length / 1MB)
    }
    if ($Sign) { Remove-Item -Recurse -Force $runtime }      # the signed copies are in Setup and the zip now
} finally {
    Pop-Location
}
