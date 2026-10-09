; The Windows installer (Inno Setup 6): a per-user install under
; %LOCALAPPDATA%\Programs\Endeavor that needs no administrator, a Start menu
; entry, an uninstaller, and Microsoft's WebView2 Runtime when it's missing.
; CI builds it from the package job's layout (.github/workflows/windows.yml):
;
;   ISCC /DVersion=0.1.0 /DBuild=<commit> /DDist=<folder holding Endeavor\>
;        /DWebView2Setup=<MicrosoftEdgeWebview2Setup.exe> scripts\installer.iss
;
; The layout is the one install::resources() looks for: bin\endeavor.exe and
; Resources\ beside bin. Julia, Node and the agents are installed on first
; run into %LOCALAPPDATA%\Endeavor, which uninstalling leaves in place.
; ponytail: not signed, so SmartScreen warns (#12).

#ifndef Version
  #define Version "0.1.0"
#endif
#ifndef Build
  #define Build "unknown"
#endif
#define AppMutex "EndeavorApp"
#ifndef Dist
  #define Dist "..\dist"
#endif

[Setup]
AppId={{1A9E6542-E5A4-4155-9B90-4770044527D8}
AppName=Endeavor
AppVersion={#Version}
AppVerName=Endeavor {#Version} (build {#Build})
AppPublisher=Endeavor
AppPublisherURL=https://github.com/jowch/Endeavor
VersionInfoVersion={#Version}
; Per user, never elevated: {autopf} is %LOCALAPPDATA%\Programs.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Endeavor
DisableDirPage=yes
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; PrepareToInstall asks to quit a running Endeavor; this is the fallback for
; one opened after that. Don't reopen it.
CloseApplications=yes
RestartApplications=no
SetupIconFile=..\assets\icon\endeavor.ico
UninstallDisplayIcon={app}\endeavor.ico
UninstallDisplayName=Endeavor
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
OutputBaseFilename=Endeavor-setup-{#Build}

[Files]
Source: "{#Dist}\Endeavor\bin\endeavor.exe"; DestDir: "{app}\bin"; Flags: ignoreversion
Source: "{#Dist}\Endeavor\Resources\*"; DestDir: "{app}\Resources"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\assets\icon\endeavor.ico"; DestDir: "{app}"; Flags: ignoreversion
#ifdef WebView2Setup
  #define WebView2File ExtractFileName(WebView2Setup)
Source: "{#WebView2Setup}"; Flags: dontcopy
#endif

; An upgrade replaces Resources whole, so a file the new version dropped
; can't linger.
[InstallDelete]
Type: filesandordirs; Name: "{app}\Resources"

[Icons]
Name: "{autoprograms}\Endeavor"; Filename: "{app}\bin\endeavor.exe"; IconFilename: "{app}\endeavor.ico"

[Run]
Filename: "{app}\bin\endeavor.exe"; Description: "Open Endeavor"; Flags: nowait postinstall skipifsilent

; WebView2 keeps its data next to the exe.
[UninstallDelete]
Type: filesandordirs; Name: "{app}\bin\endeavor.exe.WebView2"

[Code]
const
  WebView2Client = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

{ Installed for the machine or for this user, as Microsoft's docs say to check. }
function HasWebView2: Boolean;
var
  V: String;
begin
  Result := (RegQueryStringValue(HKLM32, 'SOFTWARE\' + WebView2Client, 'pv', V) and (V <> '') and (V <> '0.0.0.0'))
    or (RegQueryStringValue(HKCU, 'Software\' + WebView2Client, 'pv', V) and (V <> '') and (V <> '0.0.0.0'));
end;

{ A runtime kept running after Endeavor quit is the app's own exe, so it holds
  bin\endeavor.exe open. Stop it the way Repair runtime does. Its notebooks are
  already saved; the next launch starts a new one. }
procedure StopRuntime;
var
  Exe: String;
  Code: Integer;
begin
  Exe := ExpandConstant('{app}\bin\endeavor.exe');
  if FileExists(Exe) then
    Exec(Exe, '--stop-runtime', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

{ The app holds this mutex while it's open (platform::init). Wait for it to
  quit before stopping its runtime: an app set to keep its runtime starts a
  new one when the old one stops, and Restart Manager then can't close that
  one (it has no window) and says so. }
function AppQuit(const Action: String): Boolean;
begin
  Result := True;
  while Result and CheckForMutexes('{#AppMutex}') do
    Result := SuppressibleMsgBox('Endeavor is open. Quit it, then click OK to ' + Action + '.', mbError, MB_OKCANCEL, IDCANCEL) = IDOK;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if not AppQuit('install the new version') then
    Result := 'Quit Endeavor, then run Setup again.'
  else
    StopRuntime;
end;

procedure InstallWebView2;
var
  Code: Integer;
begin
#ifdef WebView2Setup
  WizardForm.StatusLabel.Caption := 'Installing Microsoft Edge WebView2 Runtime...';
  ExtractTemporaryFile('{#WebView2File}');
  { Not elevated, the bootstrapper installs for this user only. }
  Exec(ExpandConstant('{tmp}\{#WebView2File}'), '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, Code);
#endif
  if not HasWebView2 then
    SuppressibleMsgBox('Endeavor shows notebooks with Microsoft Edge WebView2 Runtime, and it couldn''t be installed. ' +
      'Check your internet connection, then install it from https://developer.microsoft.com/microsoft-edge/webview2/ ' +
      'and open Endeavor again.', mbInformation, MB_OK, IDOK);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  { Again, in case Restart Manager closed an Endeavor that started between
    PrepareToInstall and now, and it left its runtime running. }
  if CurStep = ssInstall then
    StopRuntime;
  if (CurStep = ssPostInstall) and not HasWebView2 then
    InstallWebView2;
end;

{ An open exe can't be removed. }
function InitializeUninstall: Boolean;
begin
  Result := AppQuit('uninstall it');
end;

{ Once the user has confirmed: the "Are you sure?" question comes after
  InitializeUninstall. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    StopRuntime;
end;
