# Builds the Android app (apps/android) on Windows without Gradle: the Rust library for 64-bit ARM, the Java
# shell, the page files and the models, in one signed package. Then, with -Install, puts it on the phone
# connected with USB debugging and starts it; with -Listen <seconds>, shows what the app printed.
#
#   pwsh apps\android\build.ps1 [-Release [-Bundle]] [-Install] [-Listen 60]
#
# Without -Release the package is for trying: debuggable (so that its log can be read from the PC) and signed with
# a debug key made here on first use. With -Release it is what is published: not debuggable, and signed with the
# app's own key (make_key.ps1: %LOCALAPPDATA%\Android\release, or on a build machine the keystore's path in
# ANDROID_KEYSTORE and its password in ANDROID_KEYSTORE_PASSWORD). The two are different apps to a phone only by
# their signature: one has to be removed before the other is installed.
#
# Needs, all per user under %LOCALAPPDATA%\Android: a JDK (jdk\), the SDK (Sdk\: platform-tools,
# build-tools 36.0.0, platforms\android-36, ndk\android-ndk-r29) and Microsoft's onnxruntime-android 1.31.1 AAR
# (downloads\, the file CI pins by size and SHA-256). The package is signed with a debug key made here on first use: for trying on one's own phone.
#
# With -Release -Bundle it also makes what Google Play takes, an app bundle (FaceRecognizer-android.aab), from the
# same files: see "The app bundle" below. That needs Google's bundletool, as the pinned file, in downloads\ (or
# its path in FACE_BUNDLETOOL).
param([switch]$Release, [switch]$Debuggable, [switch]$Bundle, [switch]$Install, [int]$Listen = 0)
if ($Bundle -and (-not $Release -or $Debuggable)) { throw '-Bundle makes what is published: it goes with -Release alone' }
$pkg = 'io.github.singularityresearch.facerecognizer'
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$rust = Resolve-Path "$here\..\.."
$repo = $rust
# The tools: this user's own under %LOCALAPPDATA%\Android, or the folder FACE_ANDROID_HOME names (a developer's
# computer that keeps them on another drive), where they are; else a build
# machine's (ANDROID_HOME, JAVA_HOME, and the model runtime's package in FACE_ORT_ANDROID_AAR). Always the same
# versions, never "the newest that is there": a build machine has newer ones, betas and extension platforms beside
# them, and a package made with those is not the one that was tried on a phone.
$a = if ($env:FACE_ANDROID_HOME) { $env:FACE_ANDROID_HOME } else { "$env:LOCALAPPDATA\Android" }
$sdk = if (Test-Path "$a\Sdk") { "$a\Sdk" } else { $env:ANDROID_HOME }
if (-not $sdk -or -not (Test-Path $sdk)) { throw 'no Android SDK: see the top of this file' }
$bt = "$sdk\build-tools\36.0.0"
$platform = "$sdk\platforms\android-36\android.jar"
if (Test-Path "$a\jdk") { $env:JAVA_HOME = (Get-ChildItem "$a\jdk" -Directory | Select-Object -First 1).FullName }
# (NDK r29 is "android-ndk-r29" as its own download and "29.<build>" as the SDK manager installs it)
$ndk = @("$sdk\ndk\android-ndk-r29") + @(Get-ChildItem "$sdk\ndk" -Directory -Filter '29.*' -ErrorAction SilentlyContinue | ForEach-Object FullName) | Where-Object { Test-Path $_ } | Select-Object -First 1
foreach ($need in $bt, $platform, $ndk) {
    if (-not $need -or -not (Test-Path $need)) { throw 'the Android tools this is built with are not all here (build-tools 36.0.0, platforms\android-36, NDK r29): see the top of this file' }
}
"tools: build-tools $(Split-Path $bt -Leaf), $(Split-Path (Split-Path $platform) -Leaf), NDK $(Split-Path $ndk -Leaf)"
# the model runtime: the version CI's release carries (1.31.1, pinned there), never another one that happens to
# lie in downloads\ beside it (a local build once took an older one that sorted first)
$ortAar = if ($env:FACE_ORT_ANDROID_AAR) { $env:FACE_ORT_ANDROID_AAR } else { "$a\downloads\onnxruntime-android-1.31.1.aar" }
if (-not (Test-Path $ortAar)) { throw "the model runtime's package for Android isn't here ($ortAar): see the top of this file" }
$java = "$env:JAVA_HOME\bin"
$out = "$rust\target\android-app"
Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$out\assets\models", "$out\assets\pages\icons", "$out\classes", "$out\gen", "$out\lib\arm64-v8a" | Out-Null

# the Rust side
# the TLS library has a little C and assembly: built with the NDK too
$ndkbin = "$ndk\toolchains\llvm\prebuilt\windows-x86_64\bin"
$env:CC_aarch64_linux_android = "$ndkbin\aarch64-linux-android24-clang.cmd"
$env:AR_aarch64_linux_android = "$ndkbin\llvm-ar.exe"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = "$ndkbin\aarch64-linux-android24-clang.cmd"
$cargo = if ($env:CARGO_HOME -and (Test-Path "$env:CARGO_HOME\bin\cargo.exe")) { "$env:CARGO_HOME\bin\cargo.exe" } elseif (Get-Command cargo -ErrorAction SilentlyContinue) { (Get-Command cargo).Source } else { "$env:USERPROFILE\.cargo\bin\cargo.exe" }
& $cargo build -p face-app --release --target aarch64-linux-android --manifest-path "$rust\Cargo.toml" 2>&1 | Select-String -Pattern "^error|Finished|^\s+-->" | ForEach-Object { $_.ToString() }
# a failed build must stop here: the library of the build before is still in the target folder, and a package
# made with it looks like the new one (it went onto the phone that way once)
if ($LASTEXITCODE -ne 0) { throw 'the Rust library did not build' }
Copy-Item "$rust\target\aarch64-linux-android\release\libface_app.so" "$out\lib\arm64-v8a\"
Add-Type -AssemblyName System.IO.Compression.FileSystem
$aar = [IO.Compression.ZipFile]::OpenRead($ortAar)
[IO.Compression.ZipFileExtensions]::ExtractToFile(($aar.Entries | Where-Object FullName -eq 'jni/arm64-v8a/libonnxruntime.so'), "$out\lib\arm64-v8a\libonnxruntime.so", $true); $aar.Dispose()

# what the app carries: the page files and the models
$pages = 'tracker_ui.html', 'tracker_ui.js', 'expression.js', 'signals.js', 'signals_chart.js', 'signals_chart.css', 'settings_bridge.js', 'page_store.js', 'ask.js', 'place.js', 'jpeg_worker.js', 'cameras_ui.html', 'cameras_ui.js'
foreach ($f in $pages) { Copy-Item "$repo\web\$f" "$out\assets\pages\" }
Copy-Item "$repo\assets\icons\icon.ico" "$out\assets\pages\icons\"
$models = 'opsis_1.onnx', 'melos_1.onnx', 'ediffiqa_tiny_jun2024.onnx', 'face_detection_yunet_2023mar.onnx', 'face_detection_yunet_2023mar_dynamic.onnx', 'facial_expression_recognition_mobilefacenet_2022july.onnx'
foreach ($f in $models) { Copy-Item "$repo\target\root\models\$f" "$out\assets\models\" }
# the sound model for the Signals chart's sound (what the phone's own microphone heard, in a word), when it is
# there; without it the chart has the background's numbers and no word for music or speech
if (Test-Path "$repo\target\root\models\yamnet.onnx") { Copy-Item "$repo\target\root\models\yamnet.onnx" "$out\assets\models\" }
# MediaPipe's parts run in the page: its three models and its web runtime (face-tools get mediapipe), when
# they are there; without them the app has no cues, posture or fingers and says so
foreach ($f in 'face_landmarker.task', 'pose_landmarker_lite.task', 'hand_landmarker.task') {
    if (Test-Path "$repo\target\root\models\$f") { Copy-Item "$repo\target\root\models\$f" "$out\assets\models\" }
}
if (Test-Path "$repo\target\root\models\mediapipe-web") {
    Copy-Item "$repo\target\root\models\mediapipe-web" "$out\assets\models\" -Recurse
}

# resources and the manifest, the Java shell, its dex
& "$bt\aapt2.exe" compile --dir "$here\res" -o "$out\res.zip"
# the app's version is the workspace's (Cargo.toml): 2.7.0 is version name 2.7.0 and version code 20700
$version = (Select-String -Path "$repo\Cargo.toml" -Pattern '^version = "(\d+)\.(\d+)\.(\d+)"' | Select-Object -First 1).Matches[0].Groups
$code = [int]$version[1].Value * 10000 + [int]$version[2].Value * 100 + [int]$version[3].Value
$versions = @('--min-sdk-version', '26', '--target-sdk-version', '36', '--replace-version', '--version-code', "$code",
              '--version-name', "$($version[1].Value).$($version[2].Value).$($version[3].Value)")
$link = @('link', '-o', "$out\base.apk", '-I', $platform, '--manifest', "$here\AndroidManifest.xml", "$out\res.zip", '--java', "$out\gen") + $versions
# (a package that is published must not be debuggable. -Release -Debuggable is the one in between: the app's own key
# and readable from the PC, for moving a trial app's data into the released app once; a release goes over it after)
if (-not $Release -or $Debuggable) { $link += '--debug-mode' }
& "$bt\aapt2.exe" @link
$sources = @(Get-ChildItem "$here\src" -Recurse -Filter *.java | ForEach-Object FullName) + @(Get-ChildItem "$out\gen" -Recurse -Filter *.java | ForEach-Object FullName)
& "$java\javac.exe" --release 17 -Xlint:-options -cp $platform -d "$out\classes" $sources
& "$java\java.exe" -cp "$bt\lib\d8.jar" com.android.tools.r8.D8 --lib $platform --min-api 26 --output $out (Get-ChildItem "$out\classes" -Recurse -Filter *.class | ForEach-Object FullName)

# The dex, the page files, the models and the two libraries into the package. The page files and models are
# put in here and not by aapt2 (-A): on Windows it names them with a backslash ("assets/models\x.onnx"), which
# to Android is one oddly named file and no folder "models" (the app then found no model). Models and
# libraries are stored as they are.
$zip = [IO.Compression.ZipFile]::Open("$out\base.apk", 'Update')
[IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, "$out\classes.dex", 'classes.dex') | Out-Null
foreach ($f in Get-ChildItem "$out\assets" -Recurse -File) {
    $name = 'assets/' + $f.FullName.Substring("$out\assets\".Length).Replace('\', '/')
    $how = if ($f.Extension -eq '.onnx') { [IO.Compression.CompressionLevel]::NoCompression } else { [IO.Compression.CompressionLevel]::Optimal }
    [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $f.FullName, $name, $how) | Out-Null
}
foreach ($so in Get-ChildItem "$out\lib\arm64-v8a" -Filter *.so) {
    [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $so.FullName, "lib/arm64-v8a/$($so.Name)", [IO.Compression.CompressionLevel]::NoCompression) | Out-Null
}
$zip.Dispose()
& "$bt\zipalign.exe" -f -p 4 "$out\base.apk" "$out\aligned.apk"
$apk = "$out\FaceRecognizer-android-arm64.apk"
# The app's own key: on this computer the one make_key.ps1 made (its password protected for this user), on a build
# machine the two it is given. The password goes to a signer through the environment, never as text on a command
# line, and is in this process's environment only while that signer runs.
$releaseKey = if ($env:ANDROID_KEYSTORE) { $env:ANDROID_KEYSTORE } else { "$a\release\facerecognizer.keystore" }
function Use-ReleaseKey([scriptblock]$Signer) {
    if (-not (Test-Path $releaseKey)) { throw "no release key ($releaseKey): make it with make_key.ps1" }
    $mine = -not $env:ANDROID_KEYSTORE_PASSWORD
    if ($mine) {
        Add-Type -AssemblyName System.Security
        $env:ANDROID_KEYSTORE_PASSWORD = [Text.Encoding]::UTF8.GetString([Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes("$a\release\facerecognizer.password"), $null, 'CurrentUser'))
    }
    try { & $Signer } finally { if ($mine) { Remove-Item Env:ANDROID_KEYSTORE_PASSWORD } }
}
if ($Release) {
    Use-ReleaseKey { & "$java\java.exe" -jar "$bt\lib\apksigner.jar" sign --ks $releaseKey --ks-key-alias facerecognizer --ks-pass env:ANDROID_KEYSTORE_PASSWORD --out $apk "$out\aligned.apk" }
    if ($LASTEXITCODE -ne 0) { throw 'the package was not signed' }
    "signed with the app's own key: " + ((& "$java\java.exe" -jar "$bt\lib\apksigner.jar" verify --print-certs $apk 2>&1 | Select-String -Pattern 'SHA-256 digest: (\S+)' | Select-Object -First 1).Matches[0].Groups[1].Value)
}
$key = "$a\debug.keystore"
if (-not $Release -and -not (Test-Path $key)) {
    & "$java\keytool.exe" -genkeypair -keystore $key -storepass android -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 -dname 'CN=Android Debug,O=Android,C=US' 2>&1 | Out-Null
}
if (-not $Release) { & "$java\java.exe" -jar "$bt\lib\apksigner.jar" sign --ks $key --ks-pass pass:android --key-pass pass:android --out $apk "$out\aligned.apk" }
"package: {0:N0} MB  $apk  (version $code$(if ($Release) { ', release' } else { ', for trying' }))" -f ((Get-Item $apk).Length / 1MB)

# The app bundle: what Google Play takes in place of a package, and makes each phone's packages from. The same
# files as two modules: "base" is the app (its code, libraries and page files), "models" the models as an asset
# pack that Play installs together with the app (install-time), because a base module may not be as large as the
# models are. The app finds them where it always did, in its assets, so nothing in it knows about the pack. The
# libraries stay compressed and are taken out at install (the bundle's settings below), as the package has them:
# the app opens the model runtime by its path. Signed with the app's own key, like the package.
if ($Bundle) {
    $tool = if ($env:FACE_BUNDLETOOL) { $env:FACE_BUNDLETOOL } else { "$a\downloads\bundletool-all-1.18.3.jar" }
    if (-not (Test-Path $tool) -or (Get-Item $tool).Length -ne 32520401 -or (Get-FileHash $tool).Hash -ne 'A099CFA1543F55593BC2ED16A70A7C67FE54B1747BB7301F37FDFD6D91028E29') {
        throw "Google's bundletool isn't there as the pinned file (bundletool-all-1.18.3.jar from github.com/google/bundletool/releases, in $a\downloads or named by FACE_BUNDLETOOL)"
    }
    $b = "$out\bundle"
    New-Item -ItemType Directory -Force $b | Out-Null
    # a module is a zip: the manifest and resources as aapt2 writes them for a bundle (--proto-format), and the
    # files under dex/, lib/ and assets/
    function New-Module([string]$Zip, [string]$Linked, [bool]$Resources, [object[]]$Files) {
        $z = [IO.Compression.ZipFile]::Open($Zip, 'Create')
        $l = [IO.Compression.ZipFile]::OpenRead($Linked)
        foreach ($e in $l.Entries) {
            $name = if ($e.FullName -eq 'AndroidManifest.xml') { 'manifest/AndroidManifest.xml' } elseif ($Resources) { $e.FullName } else { continue }
            $from = $e.Open(); $to = $z.CreateEntry($name, [IO.Compression.CompressionLevel]::NoCompression).Open()
            $from.CopyTo($to); $to.Dispose(); $from.Dispose()
        }
        $l.Dispose()
        foreach ($f in $Files) { [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($z, $f[0], $f[1], [IO.Compression.CompressionLevel]::NoCompression) | Out-Null }
        $z.Dispose()
    }
    $inAssets = { param($f) 'assets/' + $f.FullName.Substring("$out\assets\".Length).Replace('\', '/') }
    & "$bt\aapt2.exe" link --proto-format -o "$b\base-linked.apk" -I $platform --manifest "$here\AndroidManifest.xml" "$out\res.zip" @versions
    if ($LASTEXITCODE -ne 0) { throw "the bundle's manifest and resources were not made" }
    $files = @(, @("$out\classes.dex", 'dex/classes.dex'))
    $files += Get-ChildItem "$out\lib\arm64-v8a" -Filter *.so | ForEach-Object { , @($_.FullName, "lib/arm64-v8a/$($_.Name)") }
    $files += Get-ChildItem "$out\assets\pages" -Recurse -File | ForEach-Object { , @($_.FullName, (& $inAssets $_)) }
    New-Module "$b\base.zip" "$b\base-linked.apk" $true $files
    Set-Content "$b\models.xml" -Encoding utf8 @"
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" xmlns:dist="http://schemas.android.com/apk/distribution"
    package="$pkg" split="models">
    <dist:module dist:type="asset-pack">
        <dist:fusing dist:include="true" />
        <dist:delivery><dist:install-time /></dist:delivery>
    </dist:module>
</manifest>
"@
    & "$bt\aapt2.exe" link --proto-format -o "$b\models-linked.apk" -I $platform --manifest "$b\models.xml"
    if ($LASTEXITCODE -ne 0) { throw "the models' module was not made" }
    New-Module "$b\models.zip" "$b\models-linked.apk" $false @(Get-ChildItem "$out\assets\models" -Recurse -File | ForEach-Object { , @($_.FullName, (& $inAssets $_)) })
    Set-Content "$b\BundleConfig.json" -Encoding ascii '{ "optimizations": { "uncompressNativeLibraries": { "enabled": false } }, "compression": { "uncompressedGlob": ["assets/models/*.onnx"] } }'
    $aab = "$out\FaceRecognizer-android.aab"
    & "$java\java.exe" -jar $tool build-bundle "--modules=$b\base.zip,$b\models.zip" "--config=$b\BundleConfig.json" "--output=$aab" --overwrite
    if ($LASTEXITCODE -ne 0) { throw 'the app bundle was not made' }
    Use-ReleaseKey { & "$java\jarsigner.exe" -keystore $releaseKey -storetype PKCS12 -storepass:env ANDROID_KEYSTORE_PASSWORD -sigalg SHA256withRSA -digestalg SHA-256 $aab facerecognizer 2>&1 | Select-String -Pattern '^jar signed|rror' | ForEach-Object { "$_" } }
    if (-not (& "$java\jarsigner.exe" -verify $aab 2>&1 | Select-String -Pattern 'jar verified' -Quiet)) { throw 'the app bundle was not signed' }
    & "$java\java.exe" -jar $tool validate "--bundle=$aab" 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "bundletool doesn't take the app bundle for a valid one" }
    "app bundle: {0:N0} MB  $aab  (version $code)" -f ((Get-Item $aab).Length / 1MB)
}

if ($Install) {
    $adb = "$sdk\platform-tools\adb.exe"
    & $adb install -r $apk 2>&1 | Select-Object -Last 1
    & $adb shell am force-stop $pkg
    & $adb shell am start -n "$pkg/.MainActivity" 2>&1 | Select-Object -Last 1
    if ($Listen -gt 0) {
        Start-Sleep -Seconds $Listen
        # (the app's log can be read from here only in the package for trying: a release isn't debuggable)
        & $adb shell "run-as $pkg cat files/home/app.log" 2>&1 | Select-Object -Last 40
    }
}
