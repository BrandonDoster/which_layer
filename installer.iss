; Inno Setup script: per-user install, no admin prompt.
; Build: iscc /DVersion=1.2.3 installer.iss  (after cargo build --release)

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId={{DA80D6BF-6EA4-47CF-914A-A5414E8A1B94}
AppName=which_layer
AppVersion={#Version}
DefaultDirName={localappdata}\Programs\which_layer
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=dist
OutputBaseFilename=which_layer-{#Version}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=assets\icon.ico
UninstallDisplayIcon={app}\which_layer.exe
; Closes a running copy so the exe can be replaced on upgrade.
CloseApplications=yes

[Tasks]
Name: autostart; Description: "Start with Windows"

[Files]
Source: "target\release\which_layer.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\which_layer"; Filename: "{app}\which_layer.exe"

[Registry]
; Same value the tray's "Start with Windows" writes (quoted exe path), so the
; tray shows it as on.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "which_layer"; ValueData: """{app}\which_layer.exe"""; Tasks: autostart
; Removed on uninstall even if it was turned on from the tray instead.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "which_layer"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\which_layer.exe"; Description: "Launch which_layer"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/im which_layer.exe /f"; Flags: runhidden; RunOnceId: "StopApp"
