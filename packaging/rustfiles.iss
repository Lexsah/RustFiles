; Inno Setup script for the Windows installer.
; Build with: ISCC.exe packaging\rustfiles.iss
; from the repository root after `cargo build --release`.

#define MyAppName "RustFiles"
#define MyAppVersion "0.3.1"
#define MyAppPublisher "Lexsah"
#define MyAppExeName "rustfiles.exe"

[Setup]
AppId={{8F4E2C1A-6B77-4E3D-9A21-5C0B7D2E4F18}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\RustFiles
DefaultGroupName=RustFiles
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=RustFiles-0.3.1-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
UninstallDisplayIcon={app}\{#MyAppExeName}

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch RustFiles"; Flags: nowait postinstall skipifsilent
