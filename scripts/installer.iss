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
PrivilegesRequired=lowest
DefaultDirName={autopf}\Endeavor
DisableDirPage=yes
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
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

[Icons]
Name: "{autoprograms}\Endeavor"; Filename: "{app}\bin\endeavor.exe"; IconFilename: "{app}\endeavor.ico"

[Run]
Filename: "{app}\bin\endeavor.exe"; Description: "Open Endeavor"; Flags: nowait postinstall skipifsilent
Filename: "{app}\bin\endeavor.exe"; Flags: nowait; Check: HasParam('/RELAUNCH')

[UninstallDelete]
Type: filesandordirs; Name: "{app}\Resources.old"
Type: filesandordirs; Name: "{app}\bin\endeavor.exe.WebView2"

[Code]
const
  WebView2Client = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

var
  Prepared, Moved, Installed: Boolean;

function HasWebView2: Boolean;
var
  V: String;
begin
  Result := (RegQueryStringValue(HKLM32, 'SOFTWARE\' + WebView2Client, 'pv', V) and (V <> '') and (V <> '0.0.0.0'))
    or (RegQueryStringValue(HKCU, 'Software\' + WebView2Client, 'pv', V) and (V <> '') and (V <> '0.0.0.0'));
end;

procedure StopRuntime;
var
  Exe: String;
  Code: Integer;
begin
  Exe := ExpandConstant('{app}\bin\endeavor.exe');
  if FileExists(Exe) then
    Exec(Exe, '--stop-runtime', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function HasParam(const Name: String): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), Name) = 0 then
      Result := True;
end;

function AppQuit(const Action: String): Boolean;
var
  Waited: Integer;
begin
  Result := True;
  Waited := 0;
  if HasParam('/UPDATE') then
    while CheckForMutexes('{#AppMutex}') and (Waited < 60) do
    begin
      Sleep(500);
      Waited := Waited + 1;
    end;
  while Result and CheckForMutexes('{#AppMutex}') do
    Result := SuppressibleMsgBox('Endeavor is open. Quit it, then click OK to ' + Action + '.', mbError, MB_OKCANCEL, IDCANCEL) = IDOK;
end;

function MoveResourcesAside: String;
var
  Res: String;
begin
  Result := '';
  Res := ExpandConstant('{app}\Resources');
  if DirExists(Res + '.old') then
    DelTree(Res + '.old', True, True, True);
  if not DirExists(Res) then
    Exit;
  if RenameFile(Res, Res + '.old') then
    Moved := True
  else
    Result := 'Something has a file in ' + Res + ' open. Close it, then run Setup again.';
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  Prepared := True;
  if not AppQuit('install the new version') then
    Result := 'Quit Endeavor, then run Setup again.'
  else
  begin
    StopRuntime;
    Result := MoveResourcesAside;
  end;
end;

procedure InstallWebView2;
var
  Code: Integer;
begin
#ifdef WebView2Setup
  WizardForm.StatusLabel.Caption := 'Installing Microsoft Edge WebView2 Runtime...';
  ExtractTemporaryFile('{#WebView2File}');
  Exec(ExpandConstant('{tmp}\{#WebView2File}'), '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, Code);
#endif
  if not HasWebView2 then
    SuppressibleMsgBox('Endeavor shows notebooks with Microsoft Edge WebView2 Runtime, and it couldn''t be installed. ' +
      'Check your internet connection, then install it from https://developer.microsoft.com/microsoft-edge/webview2/ ' +
      'and open Endeavor again.', mbInformation, MB_OK, IDOK);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
    StopRuntime;
  if CurStep = ssPostInstall then
  begin
    Installed := True;
    DelTree(ExpandConstant('{app}\Resources.old'), True, True, True);
    if not HasWebView2 then
      InstallWebView2;
  end;
end;

procedure DeinitializeSetup;
var
  Res, Exe: String;
  Code: Integer;
begin
  if Installed or not Prepared then
    Exit;
  Res := ExpandConstant('{app}\Resources');
  if Moved then
  begin
    DelTree(Res, True, True, True);
    if not RenameFile(Res + '.old', Res) then
      Log('Couldn''t put the old Resources back.');
  end;
  Exe := ExpandConstant('{app}\bin\endeavor.exe');
  if HasParam('/RELAUNCH') and FileExists(Exe) and not CheckForMutexes('{#AppMutex}') then
    Exec(Exe, '', '', SW_SHOW, ewNoWait, Code);
end;

function InitializeUninstall: Boolean;
begin
  Result := AppQuit('uninstall it');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    StopRuntime;
end;
