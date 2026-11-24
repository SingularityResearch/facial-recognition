; Face Recognizer's Windows installer (Inno Setup 6): FaceRecognizer-Setup-x64.exe.
; Built by packaging\windows\build.ps1 (and CI), which passes the version, the built program and the folder
; of what it runs with:  ISCC /DAppVersion=2.5.2 /DBuilt=<folder with FaceRecognizer.exe> /DRuntime=<runtime folder>
;                         /DWebView2=<Microsoft's WebView2 bootstrapper, as build.ps1 fetched and checked it>
;
; What it does: copies FaceRecognizer.exe and what the program runs with (in runtime\: the model runtime with the
; Visual C++ runtime it needs and the HEIC libraries; beside the program: FFmpeg. Nothing may be assumed to be
; installed, and the HEIC libraries can't be fetched by the app itself) into a folder the user may write to, and makes the
; Start menu and desktop icons. No administrator rights: the app keeps its face models and the data folder beside
; the .exe, so it can't live under Program Files. The face models are not in it: the app asks for them at its
; first start and fetches them (79 MB).
; The app's window is shown by Microsoft's WebView2 runtime. On a computer that doesn't have it (it is part of
; Windows 11 and on nearly every Windows 10) Setup asks, and on a yes runs Microsoft's own small installer for it,
; which it carries: that downloads the runtime from Microsoft and installs it for this user, without administrator
; rights. Without the runtime the app works in the web browser instead of its window.
; What it never does: touch models\ or data\. An update replaces the .exe and runtime\; uninstalling leaves both
; folders and says where they are.
;
; It is the same app to Windows as every version before it (the same AppId and folder): installed over one, the
; .exe is replaced and the models and the data stay. /DTrial=1 makes an installer for trying it beside the app
; instead: another name, another folder, another AppId, so that the installed app is not touched.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Built
  #define Built "..\..\target\release"
#endif
#ifndef Runtime
  #define Runtime "..\..\target\root\runtime"
#endif
#ifndef Output
  #define Output "..\..\target\dist"
#endif
#ifndef WebView2
  #error Microsoft's WebView2 bootstrapper isn't named (/DWebView2=<file>): build.ps1 fetches and checks it
#endif
#ifdef Trial
  #define AppName "Face Recognizer (trial)"
  #define AppFolder "FaceRecognizerTrial"
  #define SetupName "FaceRecognizer-Trial-Setup-x64"
#else
  #define AppName "Face Recognizer"
  #define AppFolder "FaceRecognizer"
  #define SetupName "FaceRecognizer-Setup-x64"
#endif
#define AppExe "FaceRecognizer.exe"
#define AppUrl "https://github.com/SingularityResearch/facial-recognition"
#define Icons "..\..\assets\icons"

[Setup]
; AppId names this app to Windows for updates and uninstalling: never change it (the trial has its own)
#ifdef Trial
AppId={{B2C4D8A1-3F6E-4E57-9D21-7A0C5E8F4B13}
#else
AppId={{6E1B7C52-9A4F-4D0B-8C3E-2F5A7D91B6E4}
#endif
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Singularity Research
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}
AppUpdatesURL={#AppUrl}/releases
DefaultDirName={localappdata}\Programs\{#AppFolder}
UsePreviousAppDir=yes
DisableDirPage=no
DirExistsWarning=no
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#Output}
OutputBaseFilename={#SetupName}
SetupIconFile={#Icons}\icon.ico
WizardSmallImageFile={#Icons}\wizard.bmp
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/normal
SolidCompression=no
; Windows' Restart Manager can't be relied on to close the app (see CloseApp below, which does it)
CloseApplications=no
; A signed build (/DSign=1 and what "sign" runs, when it is told what to sign with): Setup signs the uninstaller it
; packs and then itself. The .exe it carries was signed before.
#ifdef Sign
SignTool=sign
SignedUninstaller=yes
#endif

[Messages]
SelectDirBrowseLabel=Face Recognizer keeps its face models and your data (faces, names, labels) in this folder too, so it must be one you can write to.%n%nIf you already use Face Recognizer, choose the folder it is in: its models and data stay as they are.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#Built}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
; what the program runs with, each file as it was fetched or built and checked (face-tools get)
Source: "{#Runtime}\*"; DestDir: "{app}\runtime"; Excludes: "ffmpeg.exe,ffprobe.exe"; Flags: ignoreversion
; FFmpeg goes beside the program: a signed copy is no longer the pinned file, and in runtime\ the app counts only
; the pinned file as there (it would ask to download FFmpeg after every install).
; Beside the program, what Setup brings counts by its name (face-tools first_run.rs). On a build that isn't
; signed these are the pinned files themselves, and the place is right all the same.
Source: "{#Runtime}\ffmpeg.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Runtime}\ffprobe.exe"; DestDir: "{app}"; Flags: ignoreversion
; Microsoft's installer for the WebView2 runtime: never put into the app's folder, taken out only where the runtime
; is missing (CurStepChanged below) and gone again with Setup's own temporary folder
Source: "{#WebView2}"; DestName: "MicrosoftEdgeWebview2Setup.exe"; Flags: dontcopy

[InstallDelete]
; the copies of FFmpeg an earlier Setup put into runtime\ (218 MB that nothing would use now); never models\ or data\
Type: files; Name: "{app}\runtime\ffmpeg.exe"
Type: files; Name: "{app}\runtime\ffprobe.exe"

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Comment: "Recognize the people you have named, live from your camera"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Comment: "Recognize the people you have named, live from your camera"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
// The app's processes started from this folder's FaceRecognizer.exe: two while it runs, the app and the tracker it
// starts as a second process, which ends when the app does (a copy in another folder is left alone). How: 0 counts
// them, 1 asks each to close as closing its window does, 2 ends them.
function AppRunning(How: Integer): Integer;
var
  Locator, Service, Found, P: Variant;
  I, Code: Integer;
  Exe: String;
begin
  Result := 0;
  Exe := ExpandConstant('{app}\{#AppExe}');
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Service := Locator.ConnectServer('.', 'root\CIMV2');
    Found := Service.ExecQuery('SELECT ProcessId, ExecutablePath FROM Win32_Process WHERE Name = "{#AppExe}"');
    for I := 0 to Found.Count - 1 do
    begin
      P := Found.ItemIndex(I);
      if not VarIsNull(P.ExecutablePath) and (CompareText(P.ExecutablePath, Exe) = 0) then
      begin
        Result := Result + 1;
        if How = 1 then
          Exec(ExpandConstant('{sys}\taskkill.exe'), '/PID ' + IntToStr(P.ProcessId), '', SW_HIDE, ewWaitUntilTerminated, Code)
        else if How = 2 then
          P.Terminate();
      end;
    end;
  except
  end;
end;

// True once the app is closed. Its files can't be replaced or removed while it runs, so Setup closes it: as closing
// the window does, waiting up to 10 seconds, and only then by ending it.
function CloseApp(): Boolean;
var
  I: Integer;
begin
  if AppRunning(0) > 0 then
  begin
    AppRunning(1);
    for I := 1 to 20 do
      if AppRunning(0) = 0 then Break else Sleep(500);
    if AppRunning(0) > 0 then
    begin
      AppRunning(2);
      for I := 1 to 10 do
        if AppRunning(0) = 0 then Break else Sleep(500);
    end;
    Sleep(1000);
  end;
  Result := AppRunning(0) = 0;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if AppRunning(0) > 0 then
  begin
    if SuppressibleMsgBox('Face Recognizer is open. Setup closes it now to replace it.', mbInformation, MB_OKCANCEL, IDOK) = IDCANCEL then
      Result := 'Face Recognizer is still open. Close it, then run Setup again.'
    else if not CloseApp() then
      Result := 'Face Recognizer could not be closed. Close it, then run Setup again.';
  end;
end;

// The app's window is shown by Microsoft's WebView2 runtime, which is part of Windows 11 and on nearly every
// Windows 10. Whether this computer has it: Microsoft's own test, a version in one of three places of the
// registry (for the computer, on 64-bit and 32-bit Windows, and for this user).
function HasWebView2(): Boolean;
var
  Version: String;
  Key: String;
begin
#ifdef Trial
  // (the trial's installer only: its check has it take the runtime for missing, on a computer that has it)
  if ExpandConstant('{param:NOWEBVIEW2|0}') = '1' then
  begin
    Result := False;
    Exit;
  end;
#endif
  Key := 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  Result := (RegQueryStringValue(HKLM, 'SOFTWARE\WOW6432Node\' + Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'))
    or (RegQueryStringValue(HKLM, 'SOFTWARE\' + Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'))
    or (RegQueryStringValue(HKCU, 'Software\' + Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'));
end;

// Where the runtime is missing: ask, and on a yes run Microsoft's own installer for it (carried in Setup), which
// downloads it from Microsoft and installs it for this user without administrator rights. A Setup that asks
// nothing (silent) brings it too. Where that fails (no internet) the app still works, in the web browser: say so.
procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
  Installer: String;
begin
  if (CurStep <> ssPostInstall) or HasWebView2() then
    Exit;
  Log('WebView2 runtime: not on this computer');
  // A Setup told to bring nothing from the internet (/OFFLINE=1) leaves the runtime alone: the Microsoft Store
  // installs the app that way, since an installer it lists may download nothing while it runs. Without the
  // runtime the app works in the web browser, and says so; running Setup again without the switch brings it.
  if ExpandConstant('{param:OFFLINE|0}') = '1' then
  begin
    Log('WebView2 runtime: not installed (Setup was told to download nothing)');
    Exit;
  end;
  if SuppressibleMsgBox('The window of Face Recognizer is shown by Microsoft''s WebView2 runtime, which this computer doesn''t have.' + #13#10#13#10 +
       'Setup can install it now: it is downloaded from Microsoft and installed for you alone, without administrator rights. ' +
       'Without it Face Recognizer opens in your web browser instead of its own window.' + #13#10#13#10 + 'Install it now?',
       mbConfirmation, MB_YESNO, IDYES) <> IDYES then
  begin
    Log('WebView2 runtime: not installed (answered with no)');
    Exit;
  end;
  ExtractTemporaryFile('MicrosoftEdgeWebview2Setup.exe');
  Installer := ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe');
  WizardForm.StatusLabel.Caption := 'Installing Microsoft''s WebView2 runtime (a download from Microsoft) ...';
  if not Exec(Installer, '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Code := -1;
  Log('WebView2 runtime: Microsoft''s installer ended with ' + IntToStr(Code));
  if not HasWebView2() then
    SuppressibleMsgBox('Microsoft''s WebView2 runtime couldn''t be installed (' + IntToStr(Code) + '). Is this computer connected to the internet?' + #13#10#13#10 +
      'Face Recognizer works without it, in your web browser. For its own window, run this Setup again later.', mbInformation, MB_OK, IDOK);
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  if AppRunning(0) > 0 then
    Result := (SuppressibleMsgBox('Face Recognizer is open. It is closed now to remove it.', mbInformation, MB_OKCANCEL, IDOK) = IDOK) and CloseApp();
end;

// Uninstalling removes the program, what it runs with and its icons only. The models and the user's data stay
// where they are: say so.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Dir: String;
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
  begin
    Dir := ExpandConstant('{app}');
    if DirExists(Dir + '\data') or DirExists(Dir + '\models') then
      MsgBox('Face Recognizer was removed. Its face models and your data (faces, names, labels) were left in:' + #13#10#13#10 +
             Dir + #13#10#13#10 + 'Delete that folder yourself if you no longer want them.', mbInformation, MB_OK);
  end;
end;
