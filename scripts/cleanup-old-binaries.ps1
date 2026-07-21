# QuicklIME 旧バイナリ掃除スクリプト
#
# dev-deploy.ps1 でのビルド退避 (.old, .old2, ...) や、インストール版更新時の
# 退避 (.old-<日時>, .locked-<タイムスタンプ>) で溜まった旧 DLL/exe を検出し、
# ゴミ箱へ移動する (完全削除はしない)。
#
# 対象: tsf のビルド出力 (build/build32 の Debug/Release)、engine のビルド出力
# (target/debug, target/release)、常用インストール環境 (Program Files) の DLL/exe。
# 現役ファイル (リネームされていないもの) とビルドシステムの管理ファイル (.recipe) は対象外
#
# 削除に失敗したファイルは使用中の可能性が高い (自動化ツールは「使用中」と
# 正直に教えてくれないことが多く、Explorer からの手動削除でのみ判明することがある)。
# 失敗しても別の削除方法へ切り替えて粘らず、一覧だけ表示して終える
#
# 前提: npm の trash-cli (`trash` コマンド) が PATH にあること
#
# 使用例:
#   scripts\cleanup-old-binaries.ps1          # 検出したファイルを削除
#   scripts\cleanup-old-binaries.ps1 -WhatIf  # 対象一覧のみ表示、削除はしない

param(
    [switch]$WhatIf
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$installDir = "$env:ProgramFiles\QuicklIME"

# 検索対象: (ディレクトリ, 現役ファイル名) の組
$targets = @(
    @{ Dir = Join-Path $root 'tsf\build\Debug';      Name = 'QuicklIME.dll' }
    @{ Dir = Join-Path $root 'tsf\build\Release';     Name = 'QuicklIME.dll' }
    @{ Dir = Join-Path $root 'tsf\build32\Debug';     Name = 'QuicklIME.dll' }
    @{ Dir = Join-Path $root 'tsf\build32\Release';   Name = 'QuicklIME.dll' }
    @{ Dir = Join-Path $root 'engine\target\debug';   Name = 'quicklime-engine.exe' }
    @{ Dir = Join-Path $root 'engine\target\release'; Name = 'quicklime-engine.exe' }
    @{ Dir = $installDir;                             Name = 'QuicklIME.dll' }
    @{ Dir = Join-Path $installDir 'x86';             Name = 'QuicklIME.dll' }
    @{ Dir = $installDir;                             Name = 'quicklime-engine.exe' }
)

# 現役ファイル本体・ビルドシステムの管理ファイル (.recipe) を除いた退避ファイルを集める。
# -Filter "name.*" は Win32 のレガシーなワイルドカード規則により "name" 自身にも
# マッチしてしまう (8.3 形式互換のための挙動) ため、-Filter は使わず正規表現で厳密に絞る
$found = foreach ($t in $targets) {
    if (-not (Test-Path $t.Dir)) { continue }
    $prefix = [regex]::Escape("$($t.Name).")
    Get-ChildItem -Path $t.Dir -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match "^$prefix" -and $_.Extension -ne '.recipe' }
}

if (-not $found) {
    Write-Host '溜まっている旧バイナリはありません。' -ForegroundColor Green
    exit 0
}

Write-Host "退避ファイルを $(@($found).Count) 件検出:" -ForegroundColor Cyan
$found | ForEach-Object { Write-Host "  $($_.FullName)  ($($_.Length) bytes, $($_.LastWriteTime))" }

if ($WhatIf) {
    Write-Host ''
    Write-Host '-WhatIf のため削除は行いません。' -ForegroundColor Yellow
    exit 0
}

Write-Host ''
$locked = @()
foreach ($file in $found) {
    # trash (npm trash-cli) はバックスラッシュ区切りパスを glob として誤解釈し
    # 静かに失敗する (エラーなし・終了コード0で何もしない) ため、
    # フォワードスラッシュに変換してから渡す
    $forwardPath = $file.FullName -replace '\\', '/'
    & trash $forwardPath *>$null
    if (Test-Path -LiteralPath $file.FullName) {
        Write-Host "使用中の可能性、スキップ: $($file.FullName)" -ForegroundColor Yellow
        $locked += $file.FullName
    } else {
        Write-Host "削除: $($file.FullName)" -ForegroundColor Green
    }
}

if ($locked.Count -gt 0) {
    Write-Host ''
    Write-Host "$($locked.Count) 件は使用中の可能性があり削除できませんでした。" -ForegroundColor Yellow
    Write-Host '時間を置くか、Explorer から手動でゴミ箱へ移動してください:' -ForegroundColor Yellow
    $locked | ForEach-Object { Write-Host "  $_" }
}
