; Obsid Ambilight installer: per-user, no admin rights.
; Build: cargo build --release, then iscc installer\obsid-ambilight.iss

#define AppName "Obsid Ambilight"
#define AppExe "obsid-ambilight.exe"
#define Root SourcePath + "\.."
#define ExePath Root + "\target\release\" + AppExe

#if !FileExists(ExePath)
  #error Build the app first: cargo build --release
#endif

#define AppVersion GetStringFileInfo(ExePath, "ProductVersion")

[Setup]
AppId={{6B0D7F3E-2C4A-4E8B-9F1D-3A5C7E9B1D24}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisherURL=https://github.com/TheZan/Obsid-Ambilight
AppSupportURL=https://github.com/TheZan/Obsid-Ambilight/issues
AppUpdatesURL=https://github.com/TheZan/Obsid-Ambilight/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\{#AppName}
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#Root}\target\installer
OutputBaseFilename=ObsidAmbilight-Setup-{#AppVersion}
SetupIconFile={#Root}\assets\icons\app.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
CloseApplications=no
ShowLanguageDialog=no
LanguageDetectionMethod=uilanguage

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
en.RemoveSettings=Also remove the settings and the strip layout?
ru.RemoveSettings=Удалить также настройки и раскладку ленты?

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  WM_CLOSE = $0010;
  TrayClass = 'ObsidAmbilightTray';
  InstanceMutex = 'Local\ObsidAmbilight.Instance';
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  ApprovedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run';
  RunValue = 'ObsidAmbilight';

{ Asks a running copy to exit (it turns the strip off or keeps the colour, as
  set by the user), waits up to 10 s, then kills whatever is left. }
procedure CloseRunningApp;
var
  Wnd: HWND;
  I, Code: Integer;
begin
  Wnd := FindWindowByClassName(TrayClass);
  if Wnd <> 0 then
    PostMessage(Wnd, WM_CLOSE, 0, 0);
  for I := 1 to 100 do
  begin
    if not CheckForMutexes(InstanceMutex) then
      Break;
    Sleep(100);
  end;
  if CheckForMutexes(InstanceMutex) or (Wnd <> 0) then
  begin
    { Give the process a moment to unload, then remove stragglers such as an
      open settings window. taskkill fails harmlessly when nothing is left. }
    Sleep(300);
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}', '', SW_HIDE,
      ewWaitUntilTerminated, Code);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseRunningApp;
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    { Only now, after the user confirmed: a cancelled uninstall keeps the app. }
    CloseRunningApp;
    RegDeleteValue(HKCU, RunKey, RunValue);
    RegDeleteValue(HKCU, ApprovedKey, RunValue);
  end;
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
    if MsgBox(CustomMessage('RemoveSettings'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      DelTree(ExpandConstant('{userappdata}\ObsidAmbilight'), True, True, True);
end;
