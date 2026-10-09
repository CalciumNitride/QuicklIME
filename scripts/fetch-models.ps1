# QuicklIME LLM モデル取得スクリプト
#
# LLM による順位補正 (docs/design/llm-rerank.md) で使う zenz 系モデルを Hugging Face から
# リポジトリ直下の models\ (git 管理外) に取得し、サイズと SHA-256 を確かめる。
# リビジョンは計測 (docs/design/experiment-0b-llm-results.md) で使ったものに固定している。
# 取得済みでサイズとハッシュが一致するファイルは取り直さない。
#
# 使用例:
#   scripts\fetch-models.ps1

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest の進捗表示は大きなファイルで極端に遅くなるため消す
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
$modelDir = Join-Path $root 'models'

# 取得元のファイル名はどちらも ggml-model-Q5_K_M.gguf なので、保存名でモデルを区別する
$models = @(
    @{
        Name     = 'zenz-v3.2-xsmall-Q5_K_M.gguf'
        Repo     = 'Miwa-Keita/zenz-v3.2-xsmall-gguf'
        Revision = '4f5423f0fad41a73b1242eb96fe5c12ae4fdca83'
        File     = 'ggml-model-Q5_K_M.gguf'
        Size     = 20970304
        Sha256   = '00c64b3d318045a708d0cad5434faccab10f5481a49e6362864551fd0995fa58'
    },
    @{
        Name     = 'zenz-v3.2-small-Q5_K_M.gguf'
        Repo     = 'Miwa-Keita/zenz-v3.2-small-gguf'
        Revision = 'c67e03e07d215c869f591b274c1631170d3e11fe'
        File     = 'ggml-model-Q5_K_M.gguf'
        Size     = 73871936
        Sha256   = '29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673'
    }
)

function Test-ModelFile([string]$path, $model) {
    if (-not (Test-Path $path)) {
        return $false
    }
    if ((Get-Item $path).Length -ne $model.Size) {
        return $false
    }
    return (Get-FileHash -Algorithm SHA256 $path).Hash -eq $model.Sha256.ToUpperInvariant()
}

New-Item -ItemType Directory -Force -Path $modelDir | Out-Null

foreach ($model in $models) {
    $dest = Join-Path $modelDir $model.Name
    if (Test-ModelFile $dest $model) {
        Write-Host "取得済み: $($model.Name)"
        continue
    }
    $url = "https://huggingface.co/$($model.Repo)/resolve/$($model.Revision)/$($model.File)"
    # 途中で失敗したファイルを正しい名前で残さないよう、一時ファイルに取ってから置き換える
    $temp = "$dest.download"
    Write-Host "取得中: $url"
    Invoke-WebRequest -Uri $url -OutFile $temp
    if (-not (Test-ModelFile $temp $model)) {
        $actualSize = (Get-Item $temp).Length
        $actualHash = (Get-FileHash -Algorithm SHA256 $temp).Hash
        Remove-Item $temp  # 検証に失敗した取得物なので直接消してよい
        throw "サイズまたはハッシュが一致しません: $($model.Name) (サイズ $actualSize、SHA-256 $actualHash)"
    }
    Move-Item -Force $temp $dest
    Write-Host "取得しました: $dest" -ForegroundColor Green
}
