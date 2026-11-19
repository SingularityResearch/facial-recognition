; Face Recognizer's Windows installer (Inno Setup 6): FaceRecognizer-Setup-x64.exe.
; Built by "python build_exe.py --installer" (and CI), which passes the version: ISCC /DAppVersion=1.2.3 ...
;
; What it does: copies FaceRecognizer.exe into a folder the user may write to, and makes the Start menu and desktop
; icons. No administrator rights: the app keeps its face models and the data folder beside the .exe (paths.ROOT),
; so it can't live under Program Files.
; What it never does: touch models\ or data\. An update replaces only the .exe; uninstalling leaves both and says
; where they are.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppName "Face Recognizer"
#define AppExe "FaceRecognizer.exe"
#define AppUrl "https://github.com/SingularityResearch/facial-recognition"

[Setup]
; AppId names this app to Windows for updates and uninstalling: never change it
AppId={{6E1B7C52-9A4F-4D0B-8C3E-2F5A7D91B6E4}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Singularity Research
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}
AppUpdatesURL={#AppUrl}/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\FaceRecognizer
UsePreviousAppDir=yes
DisableDirPage=no
DirExistsWarning=no
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\..\build\dist
OutputBaseFilename=FaceRecognizer-Setup-x64
SetupIconFile=..\..\icons\icon.ico
WizardSmallImageFile=..\..\icons\wizard.bmp
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
; the .exe is packed already: more compression only makes installing slower
Compression=lzma2/fast
SolidCompression=no
; Windows' Restart Manager can't close the app (see CloseApp below, which does it): with it on, an install while the
; app was open stopped with "Setup was unable to automatically close all applications"
CloseApplications=no
; A signed build (build_exe.py passes /DSign=1 and what "sign" runs, when it is told what to sign with): Setup signs
; the uninstaller it packs and then itself. The .exe it carries was signed before.
#ifdef Sign
SignTool=sign
SignedUninstaller=yes
#endif

[Messages]
SelectDirBrowseLabel=Face Recognizer keeps its face models and your data (faces, names, labels) in this folder too, so it must be one you can write to.%n%nIf you already use Face Recognizer, choose the folder its FaceRecognizer.exe is in: your data there is kept, and only the program is replaced.%n%nTo continue, click Next.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\..\build\dist\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Comment: "Recognize the people you have named, live from your camera"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Comment: "Recognize the people you have named, live from your camera"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
// The app's processes started from this folder's FaceRecognizer.exe: three while it runs, as the .exe unpacks itself,
// starts the app as a second process and that one the tracker as a third, which ends when the app does (a copy in
// another folder is left alone). How: 0 counts them, 1 asks each to
// close as closing its window does, 2 ends them.
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

// True once the app is closed. Its .exe can't be replaced or removed while it runs, and Windows' Restart Manager can't
// close it (the first of its two processes has no window), so Setup does: as closing the window does, waiting up to
// 10 seconds, and only then by ending it.
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

function InitializeUninstall(): Boolean;
begin
  Result := True;
  if AppRunning(0) > 0 then
    Result := (SuppressibleMsgBox('Face Recognizer is open. It is closed now to remove it.', mbInformation, MB_OKCANCEL, IDOK) = IDOK) and CloseApp();
end;

// Uninstalling removes the program and its icons only. The models and the user's data stay where they are: say so.
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
