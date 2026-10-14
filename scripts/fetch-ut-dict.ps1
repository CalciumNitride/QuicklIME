# QuicklIME UT 辞書の取得・生成スクリプト
#
# Mozc 向けの追加辞書 mozc-ut (utuhiro78 氏) のうち、同梱しやすいライセンスの 3 つ
# (SudachiDict 由来の一般語・日本郵便の郵便番号データ由来の地名・人名) と、SudachiDict の
# ライセンス文書を、リポジトリ直下の references\mozcdic-ut\ (git 管理外) に取得し、
# サイズと SHA-256 を確かめる。リビジョンは docs/design/dict-expansion.md で決めたものに固定している。
# 取得済みでサイズとハッシュが一致するファイルは取り直さない。
# 取得後に engine/examples/build_ut_dict.rs で references\mozcdic-ut\dictionary_ut.txt を生成する。
#
# 使用例:
#   scripts\fetch-ut-dict.ps1

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest の進捗表示は大きなファイルで極端に遅くなるため消す
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
$utDir = Join-Path $root 'references\mozcdic-ut'
$mozcDict = Join-Path $root 'references\mozc\src\data\dictionary_oss'
$output = Join-Path $utDir 'dictionary_ut.txt'

# 全エントリのコストに足す値 (build_ut_dict の --cost-offset)
$costOffset = 0

$files = @(
    @{
        Name     = 'mozcdic-ut-sudachidict.txt.bz2'
        Repo     = 'utuhiro78/mozcdic-ut-sudachidict'
        Revision = 'c686771bada1d59e9b105c81b29e6ac1a239cb54'
        File     = 'mozcdic-ut-sudachidict.txt.bz2'
        Size     = 11472867
        Sha256   = 'd2cb4b2b16d513e257d26d1c33673ebda254a1d5aa0c3c0c9bc94f932619fbdc'
    },
    @{
        Name     = 'mozcdic-ut-place-names.txt.bz2'
        Repo     = 'utuhiro78/mozcdic-ut-place-names'
        Revision = '98cf8e9d7731695a52e5bc72ed633933c71d1913'
        File     = 'mozcdic-ut-place-names.txt.bz2'
        Size     = 1743054
        Sha256   = '71a3e28e8acead3793739ec3466826b01adedc168354957661ec57c18688a0a2'
    },
    @{
        Name     = 'mozcdic-ut-personal-names.txt.bz2'
        Repo     = 'utuhiro78/mozcdic-ut-personal-names'
        Revision = 'b557dd4c1548cc7c2164d5f8b14170c40cbb6f81'
        File     = 'mozcdic-ut-personal-names.txt.bz2'
        Size     = 729168
        Sha256   = '610e2e52fad15df0edac854960a05451a7e9a62f3df09c3f065db9e1ab6f51f4'
    },
    @{
        Name     = 'SudachiDict-LEGAL'
        Repo     = 'WorksApplications/SudachiDict'
        Revision = '3e49051e71011cac7d74df779e80ef1dab818e56'
        File     = 'LEGAL'
        Size     = 6037
        Sha256   = '725a8776b38e058b185e905594bc9a2437dbf3787df022fffeefedb9a84e4665'
    },
    @{
        Name     = 'SudachiDict-LICENSE-2.0.txt'
        Repo     = 'WorksApplications/SudachiDict'
        Revision = '3e49051e71011cac7d74df779e80ef1dab818e56'
        File     = 'LICENSE-2.0.txt'
        Size     = 11358
        Sha256   = 'cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30'
    }
)

function Test-UtFile([string]$path, $file) {
    if (-not (Test-Path $path)) {
        return $false
    }
    if ((Get-Item $path).Length -ne $file.Size) {
        return $false
    }
    return (Get-FileHash -Algorithm SHA256 $path).Hash -eq $file.Sha256.ToUpperInvariant()
}

if (-not (Test-Path $mozcDict)) {
    throw "Mozc 辞書のディレクトリがありません: $mozcDict (references/mozc を取得してください)"
}

New-Item -ItemType Directory -Force -Path $utDir | Out-Null

foreach ($file in $files) {
    $dest = Join-Path $utDir $file.Name
    if (Test-UtFile $dest $file) {
        Write-Host "取得済み: $($file.Name)"
        continue
    }
    $url = "https://github.com/$($file.Repo)/raw/$($file.Revision)/$($file.File)"
    # 途中で失敗したファイルを正しい名前で残さないよう、一時ファイルに取ってから置き換える
    $temp = "$dest.download"
    Write-Host "取得中: $url"
    Invoke-WebRequest -Uri $url -OutFile $temp
    if (-not (Test-UtFile $temp $file)) {
        $actualSize = (Get-Item $temp).Length
        $actualHash = (Get-FileHash -Algorithm SHA256 $temp).Hash
        Remove-Item $temp  # 検証に失敗した取得物なので直接消してよい
        throw "サイズまたはハッシュが一致しません: $($file.Name) (サイズ $actualSize、SHA-256 $actualHash)"
    }
    Move-Item -Force $temp $dest
    Write-Host "取得しました: $dest" -ForegroundColor Green
}

Write-Host '=== dictionary_ut.txt の生成' -ForegroundColor Cyan
cargo run --release --manifest-path (Join-Path $root 'engine\Cargo.toml') --example build_ut_dict -- `
    --mozc $mozcDict --ut $utDir --out $output --cost-offset $costOffset
if ($LASTEXITCODE -ne 0) { throw 'build_ut_dict に失敗しました' }
Write-Host "生成しました: $output" -ForegroundColor Green
