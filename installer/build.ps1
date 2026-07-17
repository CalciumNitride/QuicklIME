# QuicklIME インストーラのビルドスクリプト
#
# 実行内容:
#   1. Rust バイナリ (release, CRT 静的リンク) のビルド
#   2. TSF DLL の 64bit / 32bit Release ビルド
#   3. installer/staging/ への集約 (バイナリ・辞書・プリセット・ライセンス)
#   4. ISCC (Inno Setup) で installer/output/quicklime-<ver>-setup.exe を生成
#
# 前提: Visual Studio 2022 Community、Rust ツールチェーン、Inno Setup 6
#       (winget install -e --id JRSoftware.InnoSetup)、references/mozc の辞書

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$staging = Join-Path $PSScriptRoot 'staging'
$cmake = 'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
$dictSrc = Join-Path $root 'references\mozc\src\data\dictionary_oss'
$symbolSrc = Join-Path $root 'references\mozc\src\data\symbol\symbol.tsv'
$mozcLicense = Join-Path $root 'references\mozc\LICENSE'

# ---- 前提チェック ----
if (-not (Test-Path $dictSrc)) {
    throw "辞書ディレクトリがありません: $dictSrc (references/mozc を取得してください)"
}
if (-not (Test-Path $mozcLicense)) {
    throw "Mozc の LICENSE がありません: $mozcLicense"
}
if (-not (Test-Path $cmake)) {
    throw "VS 同梱 cmake がありません: $cmake"
}
$iscc = @(
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    'C:\Program Files (x86)\Inno Setup 6\ISCC.exe'
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    throw 'ISCC.exe が見つかりません。winget install -e --id JRSoftware.InnoSetup で導入してください'
}

# ---- 古いプロセスの停止 (release exe のロック対策) ----
Stop-Process -Name quicklime-engine -Force -ErrorAction SilentlyContinue
Stop-Process -Name quicklime-config -Force -ErrorAction SilentlyContinue
Stop-Process -Name quicklime-regword -Force -ErrorAction SilentlyContinue

# ---- Rust バイナリ (release, CRT 静的リンク) ----
Write-Host '=== Rust release ビルド (crt-static)' -ForegroundColor Cyan
Push-Location (Join-Path $root 'engine')
try {
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build に失敗しました' }
} finally {
    Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
    Pop-Location
}

# ---- TSF DLL (64bit / 32bit Release) ----
Write-Host '=== TSF 64bit Release ビルド' -ForegroundColor Cyan
& $cmake --build (Join-Path $root 'tsf\build') --config Release
if ($LASTEXITCODE -ne 0) { throw '64bit DLL のビルドに失敗しました' }

Write-Host '=== TSF 32bit Release ビルド' -ForegroundColor Cyan
$build32 = Join-Path $root 'tsf\build32'
if (-not (Test-Path (Join-Path $build32 'CMakeCache.txt'))) {
    & $cmake -S (Join-Path $root 'tsf') -B $build32 -A Win32
    if ($LASTEXITCODE -ne 0) { throw '32bit ビルドツリーの構成に失敗しました' }
}
& $cmake --build $build32 --config Release
if ($LASTEXITCODE -ne 0) { throw '32bit DLL のビルドに失敗しました' }

# ---- staging への集約 ----
Write-Host '=== staging の集約' -ForegroundColor Cyan
if (Test-Path $staging) {
    Remove-Item -Recurse -Force $staging  # ビルド生成物のみのディレクトリなので直接消してよい
}
New-Item -ItemType Directory -Path "$staging\x64", "$staging\x86", "$staging\dict", "$staging\presets" | Out-Null

Copy-Item (Join-Path $root 'tsf\build\Release\QuicklIME.dll') "$staging\x64\"
Copy-Item (Join-Path $root 'tsf\build32\Release\QuicklIME.dll') "$staging\x86\"
Copy-Item (Join-Path $root 'engine\target\release\quicklime-engine.exe') $staging
Copy-Item (Join-Path $root 'engine\target\release\quicklime-config.exe') $staging
Copy-Item (Join-Path $root 'engine\target\release\quicklime-regword.exe') $staging

# 辞書一式 (エンジンの load_dictionary / load_matrix / load_functional_ids /
# load_symbols が読むファイル。symbol.tsv は dict 直下が最優先で読まれる)
Copy-Item (Join-Path $dictSrc 'dictionary0*.txt') "$staging\dict\"
Copy-Item (Join-Path $dictSrc 'connection_single_column.txt') "$staging\dict\"
Copy-Item (Join-Path $dictSrc 'id.def') "$staging\dict\"
Copy-Item $symbolSrc "$staging\dict\"

Copy-Item (Join-Path $root 'data\romaji-azik.tsv') "$staging\presets\"
Copy-Item $mozcLicense "$staging\LICENSE-mozc.txt"

$size = (Get-ChildItem -Recurse $staging | Measure-Object -Property Length -Sum).Sum / 1MB
Write-Host ("staging 合計: {0:N1} MB" -f $size)

# ---- インストーラの生成 ----
Write-Host '=== ISCC 実行' -ForegroundColor Cyan
& $iscc (Join-Path $PSScriptRoot 'installer.iss')
if ($LASTEXITCODE -ne 0) { throw 'ISCC に失敗しました' }

Get-ChildItem (Join-Path $PSScriptRoot 'output\*.exe') |
    ForEach-Object { Write-Host ("生成: {0} ({1:N1} MB)" -f $_.FullName, ($_.Length / 1MB)) -ForegroundColor Green }
