# QuicklIME インストーラのビルドスクリプト
#
# 実行内容:
#   1. Rust バイナリ (release, CRT 静的リンク) のビルド
#   2. TSF DLL の 64bit / 32bit Release ビルド
#   3. installer/staging/ への集約 (バイナリ・辞書・英単語辞書・プリセット・LLM の exe とモデル・
#      VC++ ランタイム・ライセンス)
#   4. ISCC (Inno Setup) で installer/output/quicklime-<ver>-setup.exe を生成
#
# 前提: Visual Studio 2022 Community、Rust ツールチェーン、Inno Setup 6
#       (winget install -e --id JRSoftware.InnoSetup)、references/mozc の辞書、
#       scripts\build-llm.ps1 でビルドした LLM の exe、scripts\fetch-models.ps1 で取得したモデル

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$staging = Join-Path $PSScriptRoot 'staging'
$cmake = 'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
$dictSrc = Join-Path $root 'references\mozc\src\data\dictionary_oss'
$symbolSrc = Join-Path $root 'references\mozc\src\data\symbol\symbol.tsv'
$mozcLicense = Join-Path $root 'references\mozc\LICENSE'
$vs = 'C:\Program Files\Microsoft Visual Studio\2022\Community'
$llmRelease = Join-Path $root 'llm\target\release'
$llmExes = @('quicklime-llm.exe', 'quicklime-llm-vulkan.exe')
$modelDir = Join-Path $root 'models'
$models = @('zenz-v3.2-xsmall-Q5_K_M.gguf', 'zenz-v3.2-small-Q5_K_M.gguf')
$llmLicenses = Join-Path $root 'llm\licenses'

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
foreach ($exe in $llmExes) {
    if (-not (Test-Path (Join-Path $llmRelease $exe))) {
        throw "LLM の exe がありません: $exe (scripts\build-llm.ps1 でビルドしてください)"
    }
}
foreach ($model in $models) {
    if (-not (Test-Path (Join-Path $modelDir $model))) {
        throw "LLM のモデルがありません: $model (scripts\fetch-models.ps1 で取得してください)"
    }
}
# LLM の exe は VC++ ランタイムを動的にリンクするため (llama.cpp が静的 CRT でリンクできない)、
# 再頒布可能な DLL を exe と同じディレクトリに置く。ビルド環境の Visual Studio の再頒布用フォルダの
# うち、版の新しいものから取る (OpenMP の vcomp140 は別のサブフォルダにある)
$redistRoot = Join-Path $vs 'VC\Redist\MSVC'
$redistDir = Get-ChildItem $redistRoot -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^\d+(\.\d+)+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    Select-Object -First 1
if (-not $redistDir) {
    throw "VC++ の再頒布用フォルダがありません: $redistRoot"
}
$redistDlls = @(
    @{ Folder = 'Microsoft.VC*.CRT'; Name = 'msvcp140.dll' },
    @{ Folder = 'Microsoft.VC*.CRT'; Name = 'vcruntime140.dll' },
    @{ Folder = 'Microsoft.VC*.CRT'; Name = 'vcruntime140_1.dll' },
    @{ Folder = 'Microsoft.VC*.OpenMP'; Name = 'vcomp140.dll' }
) | ForEach-Object {
    $found = Get-ChildItem (Join-Path $redistDir.FullName "x64\$($_.Folder)\$($_.Name)") -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $found) {
        throw "VC++ の再頒布 DLL がありません: $($_.Name) ($($redistDir.FullName)\x64\$($_.Folder))"
    }
    $found.FullName
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
New-Item -ItemType Directory -Path "$staging\x64", "$staging\x86", "$staging\dict", "$staging\presets", "$staging\models" | Out-Null

Copy-Item (Join-Path $root 'tsf\build\Release\QuicklIME.dll') "$staging\x64\"
Copy-Item (Join-Path $root 'tsf\build32\Release\QuicklIME.dll') "$staging\x86\"
Copy-Item (Join-Path $root 'engine\target\release\quicklime-engine.exe') $staging
Copy-Item (Join-Path $root 'engine\target\release\quicklime-config.exe') $staging
Copy-Item (Join-Path $root 'engine\target\release\quicklime-regword.exe') $staging
# LLM の子プロセスとモデル (エンジンは exe と同じディレクトリ・その下の models\ から探す)
foreach ($exe in $llmExes) {
    Copy-Item (Join-Path $llmRelease $exe) $staging
}
foreach ($model in $models) {
    Copy-Item (Join-Path $modelDir $model) "$staging\models\"
}
foreach ($dll in $redistDlls) {
    Copy-Item $dll $staging
}

# 辞書一式 (エンジンの load_dictionary / load_matrix / load_functional_ids /
# load_symbols が読むファイル。symbol.tsv は dict 直下が最優先で読まれる)
Copy-Item (Join-Path $dictSrc 'dictionary0*.txt') "$staging\dict\"
Copy-Item (Join-Path $dictSrc 'connection_single_column.txt') "$staging\dict\"
Copy-Item (Join-Path $dictSrc 'id.def') "$staging\dict\"
Copy-Item $symbolSrc "$staging\dict\"
# 英単語辞書 (エンジンの load_english が exe と同じディレクトリの dict\ から読む)
Copy-Item (Join-Path $root 'data\english-words.txt') "$staging\dict\"
Copy-Item (Join-Path $root 'data\english-names.txt') "$staging\dict\"

Copy-Item (Join-Path $root 'data\romaji-azik.tsv') "$staging\presets\"
Copy-Item $mozcLicense "$staging\LICENSE-mozc.txt"
Copy-Item (Join-Path $root 'data\LICENSE-SCOWL.txt') $staging
Copy-Item (Join-Path $llmLicenses 'LICENSE-llama.cpp.txt') $staging
Copy-Item (Join-Path $llmLicenses 'LICENSE-zenz.txt') $staging

$size = (Get-ChildItem -Recurse $staging | Measure-Object -Property Length -Sum).Sum / 1MB
Write-Host ("staging 合計: {0:N1} MB" -f $size)

# ---- インストーラの生成 ----
Write-Host '=== ISCC 実行' -ForegroundColor Cyan
& $iscc (Join-Path $PSScriptRoot 'installer.iss')
if ($LASTEXITCODE -ne 0) { throw 'ISCC に失敗しました' }

Get-ChildItem (Join-Path $PSScriptRoot 'output\*.exe') |
    ForEach-Object { Write-Host ("生成: {0} ({1:N1} MB)" -f $_.FullName, ($_.Length / 1MB)) -ForegroundColor Green }
