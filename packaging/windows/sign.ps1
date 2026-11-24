# Signs Windows programs of the app, or says how files are signed:
#
#     powershell -File packaging\windows\sign.ps1 -Files <file> [<file> ...]
#     powershell -File packaging\windows\sign.ps1 -Check <file> [<file> ...]
#
# Unsigned unless the environment says with what:
#   FACE_SIGN_ACCOUNT, FACE_SIGN_PROFILE, FACE_SIGN_ENDPOINT   Azure Artifact Signing: the key stays in
#       Microsoft's service; signtool (the Windows SDK's) asks it through Microsoft's plug-in, as whoever is signed
#       in to Azure (CI: this repository's workflow on main, with no stored password; here: az login).
#   FACE_SIGN_CERT   a certificate in the user's Windows store, by thumbprint (a certificate authority's on its
#       token; a self-signed one to try the build).
# The plug-in is one version from nuget.org, taken only with its size and SHA-256 (below); it needs the .NET 10
# runtime. Every signature is dated by a time server: Artifact Signing's certificates last three days, a dated
# signature stays good after that.
# -Check reads each file's signature as Windows does and stops unless it is valid, trusted and dated. It prints
# the certificate's thumbprint and issuer, never whom it names: an individual's certificate carries their name
# and town, and CI's log is public.
param([string[]]$Files, [string[]]$Check)
$ErrorActionPreference = "Stop"
$DlibVersion = "1.0.151"
$DlibSize = 14740357
$DlibSha256 = "35375a4bc4e1102da729fc3d8d9925f774c3baae6b8b7ff6398d948ca9ab0d8a"
$DlibUrl = "https://api.nuget.org/v3-flatcontainer/microsoft.artifactsigning.client/$DlibVersion/microsoft.artifactsigning.client.$DlibVersion.nupkg"
$DlibNet = 10
$SigntoolMin = [version]"10.0.22621.755"      # the oldest signtool the plug-in works with
$Timestamp = "http://timestamp.acs.microsoft.com"
# The plug-in tries these ways of signing in to Azure in turn. Left: the environment and the Azure CLI. The
# others wait for something that isn't there (a managed identity) or open a window.
$NotTheseLogins = @("ManagedIdentityCredential", "WorkloadIdentityCredential", "SharedTokenCacheCredential", "VisualStudioCredential",
    "VisualStudioCodeCredential", "AzurePowerShellCredential", "AzureDeveloperCliCredential", "InteractiveBrowserCredential")
$repo = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$work = Join-Path $repo "target\signing"

function Get-Signtool {
    $kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    if (-not (Test-Path $kits)) { return $null }
    Get-ChildItem $kits -Directory | Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' -and [version]$_.Name -ge $SigntoolMin } |
        Sort-Object { [version]$_.Name } -Descending | ForEach-Object { Join-Path $_.FullName "x64\signtool.exe" } |
        Where-Object { Test-Path $_ } | Select-Object -First 1
}

# signtool's plug-in for Artifact Signing, unpacked under target\signing: Microsoft's package from nuget.org at one
# version, taken only as the pinned file, and of it only the 64-bit plug-in's own files.
function Get-Dlib {
    $folder = Join-Path $work "dlib-$DlibVersion"
    $dll = Join-Path $folder "bin\x64\Azure.CodeSigning.Dlib.dll"
    if (Test-Path $dll) { return $dll }
    New-Item -ItemType Directory -Force $work | Out-Null
    $package = Join-Path $work "dlib.nupkg"
    Invoke-WebRequest -Uri $DlibUrl -OutFile $package -UseBasicParsing
    if ((Get-Item $package).Length -ne $DlibSize -or (Get-FileHash $package -Algorithm SHA256).Hash -ne $DlibSha256) {
        Remove-Item $package
        throw "the Artifact Signing plug-in from nuget.org isn't the pinned file (size or SHA-256 differs)"
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($package)
    try {
        foreach ($entry in $zip.Entries) {
            if ($entry.FullName -match '^bin/x64/[A-Za-z0-9_.\-]+$') {
                $to = Join-Path $folder ($entry.FullName -replace '/', '\')
                New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
                [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $to, $true)
            }
        }
    } finally { $zip.Dispose() }
    Remove-Item $package
    $dll
}

function Read-Signature($file) {
    $s = Get-AuthenticodeSignature -LiteralPath $file
    [pscustomobject]@{ Status = [string]$s.Status; Issuer = [string]$s.SignerCertificate.Issuer; Thumbprint = [string]$s.SignerCertificate.Thumbprint
        Dated = ($null -ne $s.TimeStamperCertificate) }
}

if ($Check) {
    foreach ($file in $Check) {
        $s = Read-Signature $file
        "{0}: {1}, dated: {2}, certificate {3} from {4}" -f (Split-Path $file -Leaf), $s.Status, $s.Dated, $s.Thumbprint, $s.Issuer
        if ($s.Status -ne "Valid" -or -not $s.Dated) { throw "$(Split-Path $file -Leaf) is not signed, trusted and dated" }
    }
    return
}
if (-not $Files) { throw "name the files to sign (-Files), or to look at (-Check)" }
$azure = $env:FACE_SIGN_ACCOUNT -and $env:FACE_SIGN_PROFILE -and $env:FACE_SIGN_ENDPOINT
if (-not $azure -and -not $env:FACE_SIGN_CERT) {
    throw "signing isn't set up (FACE_SIGN_ACCOUNT, FACE_SIGN_PROFILE and FACE_SIGN_ENDPOINT, or FACE_SIGN_CERT)"
}
$signtool = Get-Signtool
if (-not $signtool) { throw "signing needs signtool.exe from the Windows SDK (winget install -e --id Microsoft.WindowsSDK.10.0.26100)" }
$command = @("sign", "/fd", "SHA256", "/tr", $Timestamp, "/td", "SHA256", "/d", "Face Recognizer", "/du", "https://github.com/SingularityResearch/facial-recognition")
if ($azure) {
    $runtimes = (& dotnet --list-runtimes 2>$null) -join "`n"
    if (-not ([regex]::Matches($runtimes, '(?m)^Microsoft\.NETCore\.App (\d+)\.') | Where-Object { [int]$_.Groups[1].Value -ge $DlibNet })) {
        throw "signing with Artifact Signing needs the .NET $DlibNet runtime (winget install -e --id Microsoft.DotNet.Runtime.$DlibNet)"
    }
    $dlib = Get-Dlib
    # the plug-in's description of the account: no secret in it
    $meta = Join-Path $work "metadata.json"
    @{ Endpoint = $env:FACE_SIGN_ENDPOINT; CodeSigningAccountName = $env:FACE_SIGN_ACCOUNT; CertificateProfileName = $env:FACE_SIGN_PROFILE
        ExcludeCredentials = $NotTheseLogins } | ConvertTo-Json | Set-Content -Path $meta -Encoding ascii
    $command += @("/dlib", $dlib, "/dmdf", $meta)
} else {
    $command += @("/sha1", $env:FACE_SIGN_CERT)
}
& $signtool @command @Files
if ($LASTEXITCODE -ne 0) {
    throw "signing failed (signtool: $LASTEXITCODE). With Artifact Signing: is this computer signed in to Azure, with the Signer role on the certificate profile, and is the address the account's region?"
}
foreach ($file in $Files) { "Signed $(Split-Path $file -Leaf)" }
