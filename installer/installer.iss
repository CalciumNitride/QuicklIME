; QuicklIME インストーラ (Inno Setup 6)
;
; ビルド方法: installer/build.ps1 を実行する (staging の集約 → ISCC 呼び出し)。
; 直接 ISCC installer.iss を実行する場合は、事前に build.ps1 が作る
; installer/staging/ (バイナリ・辞書・ライセンス) が必要。
;
; 設計メモ:
; - DLL の登録は regserver フラグで DllRegisterServer に委譲する
;   (COM 登録 + TSF プロファイル + カテゴリ。アンインストール時は
;    Inno が対で DllUnregisterServer を呼ぶ)
; - 32bit DLL は x86\ に配置し 32bit フラグで登録する (Wow6432Node 側)。
;   32bit アプリからは x86\QuicklIME.dll → 親ディレクトリの 64bit exe が使われる
; - 更新時にロード中の DLL は restartreplace で再起動時置換になる
;   (初回インストールは未ロードなので再起動不要。AlwaysRestart にはしない)
; - %APPDATA%\QuicklIME (設定・ユーザ辞書・学習) には触れない (アンインストールでも残す)

#define MyAppName "QuicklIME"
#define MyAppVersion "0.1.0"
#define Staging "staging"
#define CommonFileFlags "ignoreversion restartreplace uninsrestartdelete"

[Setup]
AppId={{9C47761E-52E4-47BD-BFED-9C0FBE191FE5}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher=CalciumNitride
DefaultDirName={autopf}\QuicklIME
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
MinVersion=10.0.14393
OutputDir=output
OutputBaseFilename=quicklime-{#MyAppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
CloseApplications=no
RestartApplications=no
SetupLogging=yes
SetupIconFile=QuicklIME.ico
UninstallDisplayIcon={app}\quicklime-config.exe,0

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Files]
Source: "{#Staging}\x64\QuicklIME.dll"; DestDir: "{app}"; Flags: {#CommonFileFlags} regserver 64bit
Source: "{#Staging}\x86\QuicklIME.dll"; DestDir: "{app}\x86"; Flags: {#CommonFileFlags} regserver 32bit
Source: "{#Staging}\quicklime-engine.exe"; DestDir: "{app}"; Flags: {#CommonFileFlags}
Source: "{#Staging}\quicklime-config.exe"; DestDir: "{app}"; Flags: {#CommonFileFlags}
Source: "{#Staging}\quicklime-regword.exe"; DestDir: "{app}"; Flags: {#CommonFileFlags}
; 辞書類はエンジン停止後ならロックされないため restartreplace は不要
Source: "{#Staging}\dict\*"; DestDir: "{app}\dict"; Flags: ignoreversion
Source: "{#Staging}\presets\*"; DestDir: "{app}\presets"; Flags: ignoreversion
Source: "{#Staging}\LICENSE-mozc.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\QuicklIME 設定"; Filename: "{app}\quicklime-config.exe"

[Code]
// 常駐エンジンと各ツールを止める (更新時の exe ロック解除・アンインストール準備)。
// プロセスが居なくても taskkill は失敗扱いにせず続行する
procedure KillQuicklimeProcesses();
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM quicklime-engine.exe', '',
       SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM quicklime-config.exe', '',
       SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM quicklime-regword.exe', '',
       SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  KillQuicklimeProcesses();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  KillQuicklimeProcesses();
  Result := True;
end;
