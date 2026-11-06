; Per-user installer: no administrator rights, a shortcut on the desktop and in the Start menu.
; Inno Setup is part of GitHub's Windows runner image. Built by CI:
;   ISCC.exe /DVersion=x.y.z installer\collector.iss

#ifndef Version
  #error Version is required, for example /DVersion=0.1.0
#endif
#define Name "Big Rocket Camp Collector"
#define AppId "BigRocketCampCollector"
#define Exe "bigrocketcamp-collector.exe"

[Setup]
AppId={#AppId}
AppName={#Name}
AppVersion={#Version}
AppVerName={#Name} {#Version}
AppPublisher=Big Rocket Camp
AppPublisherURL=https://github.com/mousrockets/bigrocketcamp-collector
VersionInfoVersion={#Version}
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppId}
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; The collector is closed by [Code] below: a plain WM_CLOSE only hides it.
CloseApplications=no
SetupIconFile=..\assets\icons\app.ico
UninstallDisplayIcon={app}\{#Exe}
UninstallDisplayName={#Name}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir=..
OutputBaseFilename=BigRocketCampCollector-setup

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Files]
Source: "..\target\release\{#Exe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autodesktop}\{#Name}"; Filename: "{app}\{#Exe}"
Name: "{autoprograms}\{#Name}"; Filename: "{app}\{#Exe}"

[Run]
Filename: "{app}\{#Exe}"; Description: "{cm:LaunchProgram,{#Name}}"; Flags: nowait postinstall skipifsilent

[Registry]
; The collector adds itself to the Windows startup list; uninstalling removes that entry.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "{#AppId}"; Flags: uninsdeletevalue dontcreatekey

[UninstallDelete]
; The token, the queue and the settings leave with the collector.
Type: filesandordirs; Name: "{localappdata}\{#AppId}"

[Code]
const
  // WM_APP + 16 in src/win/ui.rs: the collector quits instead of hiding.
  WM_QUIT_COLLECTOR = $8010;

// A running collector holds its executable: ask it to quit before replacing or removing it.
procedure CloseCollector();
var
  Window: HWND;
begin
  Window := FindWindowByClassName('{#AppId}');
  if Window <> 0 then
  begin
    SendMessage(Window, WM_QUIT_COLLECTOR, 0, 0);
    Sleep(1000);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseCollector();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  CloseCollector();
  Result := True;
end;
