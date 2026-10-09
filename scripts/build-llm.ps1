# QuicklIME LLM 子プロセス (quicklime-llm) のビルドスクリプト
#
# llm\ クレートを release でビルドし、次の2つの exe を llm\target\release\ に置く
# (エンジンは exe と同じディレクトリ → この場所の順で探す)。
#   - quicklime-llm.exe         CPU 版
#   - quicklime-llm-vulkan.exe  Vulkan 版 (feature vulkan)
#
# llama.cpp のビルドに使う環境 (docs/design/experiment-0b-llm-results.md の再現手順と同じ):
#   - LIBCLANG_PATH (bindgen 用の LLVM)、VS 付属の CMake・Ninja、vcvars64 の MSVC 環境
#   - CMAKE_GENERATOR=Ninja: Visual Studio ジェネレータでは Vulkan のシェーダ生成ツールの
#     configure・build・install が並行して走り、順序が崩れて失敗するため
#   - Vulkan 版は subst で割り当てた一時ドライブをターゲットディレクトリにする。
#     深いパスでは try-compile のパスが 260 文字を超えて失敗するため
# llama.cpp は CRT を動的リンク (/MD) でビルドするため、exe は VC++ ランタイム
# (vcruntime140.dll・msvcp140.dll・OpenMP の vcomp140.dll) に依存する。
# 環境変数は子の cmd の中だけで設定し、呼び出し元のセッションには残さない。
#
# 前提: Visual Studio 2022 Community、Rust、LLVM、Vulkan SDK
#
# 使用例:
#   scripts\build-llm.ps1
#   scripts\build-llm.ps1 -VulkanSdk C:\VulkanSDK\1.4.363.0

param(
    [string]$VulkanSdk = 'C:\VulkanSDK\1.4.363.0',
    [string]$LlvmBin = 'C:\Program Files\LLVM\bin'
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$llmDir = Join-Path $root 'llm'
$releaseDir = Join-Path $llmDir 'target\release'
$vulkanTarget = Join-Path $llmDir 'target\vulkan'
$vs = 'C:\Program Files\Microsoft Visual Studio\2022\Community'
$vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvars64.bat'
$cmakeBin = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
$ninjaBin = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja'

# ---- 前提チェック ----
foreach ($path in @($vcvars, $cmakeBin, $ninjaBin, (Join-Path $LlvmBin 'libclang.dll'), (Join-Path $VulkanSdk 'Bin'))) {
    if (-not (Test-Path $path)) {
        throw "ビルドに必要なファイルがありません: $path"
    }
}

# vcvars64 の後に環境変数を設定して cargo を実行する (cmd の中だけで有効)
function Invoke-LlmCargo([string]$cargoArgs, [string]$path) {
    $commands = @(
        "call `"$vcvars`" >nul",
        "set `"LIBCLANG_PATH=$LlvmBin`"",
        "set `"VULKAN_SDK=$VulkanSdk`"",
        "set `"PATH=$VulkanSdk\Bin;$cmakeBin;$ninjaBin;!PATH!`"",
        'set "CMAKE_GENERATOR=Ninja"',
        "cd /d `"$path`"",
        "cargo build --release $cargoArgs"
    )
    # 1行のコマンドは実行前にまとめて展開されるため、vcvars が足した PATH を読むには遅延展開 (!PATH!) が要る。
    # set は引用符で囲まないと && の前の空白まで値に入る
    cmd /v:on /c ($commands -join ' && ')
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build に失敗しました: $cargoArgs"
    }
}

# ---- CPU 版 ----
Write-Host '=== quicklime-llm (CPU) release ビルド' -ForegroundColor Cyan
Invoke-LlmCargo '--bin quicklime-llm' $llmDir

# ---- Vulkan 版 ----
Write-Host '=== quicklime-llm-vulkan release ビルド' -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $vulkanTarget | Out-Null
$used = (Get-PSDrive -PSProvider FileSystem).Name
$drive = [char[]]'ZYXWVUTSRQPONMLKJIHG' | Where-Object { $used -notcontains [string]$_ } | Select-Object -First 1
if (-not $drive) {
    throw 'subst に使える空きドライブがありません'
}
subst "${drive}:" $vulkanTarget
if ($LASTEXITCODE -ne 0) {
    throw "subst に失敗しました: ${drive}: -> $vulkanTarget"
}
try {
    Invoke-LlmCargo "--bin quicklime-llm-vulkan --features vulkan --target-dir ${drive}:\" $llmDir
} finally {
    subst "${drive}:" /d
}
Copy-Item (Join-Path $vulkanTarget 'release\quicklime-llm-vulkan.exe') $releaseDir -Force

Get-ChildItem (Join-Path $releaseDir 'quicklime-llm*.exe') |
    ForEach-Object { Write-Host ("生成: {0} ({1:N1} MB)" -f $_.FullName, ($_.Length / 1MB)) -ForegroundColor Green }
