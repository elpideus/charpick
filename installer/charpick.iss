; Charpick installer (Inno Setup 6.3 or later). Build with installer\build.ps1.
; Installs per user, without administrator rights. The optional Win+L task asks
; for administrator approval once (see README).

#define AppName "Charpick"
#define AppVersion "1.0.3"
#define AppExe "Charpick.exe"

[Setup]
AppId={{8C4F1E2A-5B7D-4C1E-9F3A-2D6B8E0A4C71}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=elpideus
AppPublisherURL=https://github.com/elpideus/charpick
AppSupportURL=https://github.com/elpideus/charpick/issues
AppUpdatesURL=https://github.com/elpideus/charpick/releases
DefaultDirName={autopf}\{#AppName}
DisableDirPage=auto
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\dist
OutputBaseFilename=Charpick-Setup
SetupIconFile=..\assets\charpick.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
; Charpick is closed by PrepareToInstall below.
CloseApplications=no
RestartApplications=no

[Tasks]
Name: "winl"; Description: "Open Charpick with Win+L (Windows asks for administrator approval once; lock the PC with Ctrl+Win+L instead)"
Name: "autostart"; Description: "Start Charpick when I sign in"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme skipifsourcedoesntexist
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "Pick accented letters and symbols"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; \
    ValueData: """{app}\{#AppExe}"" --autostart"; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Parameters: "--setup-lock"; StatusMsg: "Setting up Win+L..."; Tasks: winl; Flags: waituntilterminated
Filename: "{app}\{#AppExe}"; Parameters: "--welcome"; Description: "Start {#AppName} now"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Quits Charpick, gives Win+L back to Windows and removes the startup entry.
Filename: "{app}\{#AppExe}"; Parameters: "--uninstall-cleanup"; RunOnceId: "CharpickCleanup"; Flags: waituntilterminated

[UninstallDelete]
Type: filesandordirs; Name: "{userappdata}\{#AppName}"

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Exe: String;
  Code: Integer;
begin
  Result := '';
  // Quit a running copy so that its file can be replaced.
  Exe := ExpandConstant('{app}\{#AppExe}');
  if FileExists(Exe) then
    Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;
