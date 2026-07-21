# QuicklIME デバッグ反映スクリプト
#
# 実行内容 (既定, 引数なし):
#   1. 常用エンジン (quicklime-engine.exe) を停止
#   2. engine を release ビルド
#   3. C:\Program Files\QuicklIME へコピー (常用インストール版を直接更新)
#   4. 次の変換操作で TSF が自動起動
#
# -Dll 指定時: 上記に加えて TSF DLL (64bit) をビルドする。反映方法は2通り:
#   - 既定 (Configuration Debug): 開発版 (tsf\build\Debug\QuicklIME.dll) に
#     regsvr32 で排他的に切り替える。常用インストール版には触れない。
#     要管理者権限。動作確認が終わったら -Restore でインストール版に戻すこと
#   - -Install 併用時 (Configuration 既定 Release): 常用インストール版
#     (%ProgramFiles%\QuicklIME\QuicklIME.dll) を直接更新する。regsvr32 は使わず、
#     ロード中の DLL を .old-<日時> にリネーム退避してから同じパスに新 DLL を
#     コピーする (パスは変わらないためレジストリ再登録は不要。インストーラの
#     restartreplace と違い再起動不要)。管理者権限は不要
#     (Program Files への書き込みが通常権限で成功する環境の場合)。
#     32bit 版 (x86\QuicklIME.dll) はこのスクリプトの対象外
#
# -Restore 指定時: 開発版からインストール版 (Program Files) の DLL に regsvr32 で戻すだけ。
#                   他の処理は行わない
#
# 前提: cargo (PATH), VS 2022 同梱 cmake, references\mozc の辞書取得済み。
#       tsf\build が未構成なら初回ビルド時に自動生成される
#
# 使用例:
#   scripts\dev-deploy.ps1                       # エンジンのみ常用環境へ反映
#   scripts\dev-deploy.ps1 -Dll                  # エンジン + DLL (Debug) を反映、DLL は開発版へ切替
#   scripts\dev-deploy.ps1 -Dll -Install         # エンジン + DLL (Release) を常用インストール版へ直接反映
#   scripts\dev-deploy.ps1 -Dll -Configuration Release
#   scripts\dev-deploy.ps1 -Restore              # 開発版からインストール版の DLL に戻す

[CmdletBinding(DefaultParameterSetName = 'Deploy')]
param(
    [Parameter(ParameterSetName = 'Deploy')]
    [switch]$Dll,

    [Parameter(ParameterSetName = 'Deploy')]
    [switch]$Install,

    [Parameter(ParameterSetName = 'Deploy')]
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration,

    [Parameter(ParameterSetName = 'Restore', Mandatory)]
    [switch]$Restore
)

$ErrorActionPreference = 'Stop'

if ($Install -and -not $Dll) {
    throw '-Install は -Dll と併用してください (TSF DLL をビルドしないと反映できません)'
}
if (-not $Configuration) {
    $Configuration = if ($Install) { 'Release' } else { 'Debug' }
}

$root = Split-Path -Parent $PSScriptRoot
$installDir = "$env:ProgramFiles\QuicklIME"
$cmake = 'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'

function Test-Admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)
}

# ---- -Restore: インストール版の DLL に戻すだけ ----
if ($Restore) {
    if (-not (Test-Admin)) {
        throw 'regsvr32 には管理者権限が必要です。管理者権限の PowerShell で実行し直してください'
    }
    Write-Host '=== インストール版 DLL に戻す' -ForegroundColor Cyan
    & regsvr32 /s "$installDir\QuicklIME.dll"
    if ($LASTEXITCODE -ne 0) { throw 'regsvr32 に失敗しました' }
    Write-Host 'インストール版に戻しました。新規に起動するアプリから反映されます' -ForegroundColor Green
    exit 0
}

# -Dll で regsvr32 を使う (開発版切替) 場合のみ、時間のかかるビルドに入る前に権限を
# 確認しておく。-Install (常用インストール版への直接反映) は regsvr32 を使わないため
# 管理者権限は不要
if ($Dll -and -not $Install -and -not (Test-Admin)) {
    throw 'regsvr32 には管理者権限が必要です。管理者権限の PowerShell で実行し直してください'
}

# ---- 前提チェック ----
if (-not (Test-Path $installDir)) {
    throw "インストール先が見つかりません: $installDir (インストーラでの導入を確認してください)"
}
if ($Dll -and -not (Test-Path $cmake)) {
    throw "VS 同梱 cmake がありません: $cmake"
}

# ---- エンジンの反映 (常用インストール版を直接更新) ----
Write-Host '=== エンジンを停止' -ForegroundColor Cyan
Stop-Process -Name quicklime-engine -Force -ErrorAction SilentlyContinue

Write-Host '=== エンジンを release ビルド' -ForegroundColor Cyan
Push-Location (Join-Path $root 'engine')
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build に失敗しました' }
} finally {
    Pop-Location
}

Write-Host '=== Program Files へコピー' -ForegroundColor Cyan
Copy-Item (Join-Path $root 'engine\target\release\quicklime-engine.exe') $installDir -Force

# ---- DLL の反映 (任意, 開発版への regsvr32 切替) ----
if ($Dll) {
    Write-Host "=== TSF DLL を $Configuration ビルド" -ForegroundColor Cyan
    $buildDir = Join-Path $root 'tsf\build'
    $dllPath = Join-Path $buildDir "$Configuration\QuicklIME.dll"

    & $cmake --build $buildDir --config $Configuration
    if ($LASTEXITCODE -ne 0) {
        # ロード中の DLL でリンクが LNK1168 になった場合は退避してリトライ
        # (ロード中でもリネームは可能。.claude/CLAUDE.md 記載の対策)
        if (-not (Test-Path $dllPath)) { throw 'DLL のビルドに失敗しました' }
        Write-Host 'DLL がロック中の可能性があるため退避してリトライします' -ForegroundColor Yellow
        Move-Item $dllPath "$dllPath.old" -Force
        & $cmake --build $buildDir --config $Configuration
        if ($LASTEXITCODE -ne 0) { throw 'DLL のビルドに失敗しました (退避後も失敗)' }
    }

    if ($Install) {
        # ---- 常用インストール版を直接更新 ----
        # regsvr32 は使わない。InprocServer32 は既に $installDir\QuicklIME.dll を指して
        # いるので、そのパスの実体を差し替えるだけで新規プロセスから新 DLL が有効になる
        # (ロード中でもリネームは可能。.claude/CLAUDE.md 記載の対策と同じ原理)
        Write-Host '=== インストール版 DLL を更新' -ForegroundColor Cyan
        $installedDll = "$installDir\QuicklIME.dll"
        if (Test-Path $installedDll) {
            $backup = "$installedDll.old-$(Get-Date -Format 'yyyyMMddHHmmss')"
            Move-Item $installedDll $backup -Force
            Write-Host "旧 DLL を退避: $backup" -ForegroundColor DarkGray
        }
        Copy-Item $dllPath $installedDll -Force

        Write-Host ''
        Write-Host 'インストール版 DLL を更新しました。既に起動中のアプリには反映されません。' -ForegroundColor Green
        Write-Host 'メモ帳などで確認する場合は taskkill /IM Notepad.exe /F してから開き直してください。' -ForegroundColor Green
        Write-Host '32bit 版 (x86\QuicklIME.dll) はこのスクリプトの対象外です。' -ForegroundColor DarkYellow
    } else {
        # ---- 開発版への regsvr32 切替 ----
        Write-Host '=== 開発版 DLL に切替 (regsvr32)' -ForegroundColor Cyan
        & regsvr32 /s $dllPath
        if ($LASTEXITCODE -ne 0) { throw 'regsvr32 に失敗しました' }

        Write-Host ''
        Write-Host '開発版 DLL に切り替わりました。既に起動中のアプリには反映されません。' -ForegroundColor Green
        Write-Host 'メモ帳などで確認する場合は taskkill /IM Notepad.exe /F してから開き直してください。' -ForegroundColor Green
        Write-Host '確認が終わったら scripts\dev-deploy.ps1 -Restore でインストール版に戻してください。' -ForegroundColor Green
    }
}

Write-Host ''
Write-Host '反映が完了しました。' -ForegroundColor Green
