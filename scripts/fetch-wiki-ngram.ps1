# QuicklIME 後続語の統計 (next_word.tsv) の取得・生成スクリプト
#
# 日本語版ウィキペディアの CirrusSearch のダンプ (jawiki_content-<日付>-NNNNN.json.bz2) のうち、指定した
# 分割をリポジトリ直下の references\jawiki\ (git 管理外) に取得し、名前・サイズ・SHA-256 を
# references\jawiki\manifest.tsv に書く (docs/spec/suggest.md「コーパス」「生成」)。
# ダンプの置き場所は週ごとに作られ直近 4 週だけが残るので、日付は
# https://dumps.wikimedia.org/other/cirrus_search_index/ で今あるものを選ぶ。
# 取得済みでサイズが取得元と一致する分割は取り直さない。
# 取得後に engine/examples/build_next_word.rs で、指定した分割から references\jawiki\next_word.tsv を生成する。
#
# 使用例:
#   scripts\fetch-wiki-ngram.ps1 -Date 20261018 -Parts 0
#   scripts\fetch-wiki-ngram.ps1 -Date 20261018 -Parts (0..13)

param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d{8}$')]
    [string]$Date,

    [Parameter(Mandatory = $true)]
    [int[]]$Parts
)

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest の進捗表示は大きなファイルで極端に遅くなるため消す
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
$wikiDir = Join-Path $root 'references\jawiki'
$mozcDict = Join-Path $root 'references\mozc\src\data\dictionary_oss'
$manifest = Join-Path $wikiDir 'manifest.tsv'
$output = Join-Path $wikiDir 'next_word.tsv'
$baseUrl = "https://dumps.wikimedia.org/other/cirrus_search_index/$Date/index_name=jawiki_content"

if (-not (Test-Path $mozcDict)) {
    throw "Mozc 辞書のディレクトリがありません: $mozcDict (references/mozc を取得してください)"
}

New-Item -ItemType Directory -Force -Path $wikiDir | Out-Null

$inputs = @()
$rows = @()
foreach ($part in $Parts) {
    $name = 'jawiki_content-{0}-{1:D5}.json.bz2' -f $Date, $part
    $url = "$baseUrl/$name"
    $dest = Join-Path $wikiDir $name
    $head = Invoke-WebRequest -Uri $url -Method Head
    $size = [long]($head.Headers['Content-Length'] | Select-Object -First 1)
    if ((Test-Path $dest) -and (Get-Item $dest).Length -eq $size) {
        Write-Host "取得済み: $name"
    } else {
        # 途中で失敗したファイルを正しい名前で残さないよう、一時ファイルに取ってから置き換える
        $temp = "$dest.download"
        Write-Host "取得中: $url ($size バイト)"
        $watch = [System.Diagnostics.Stopwatch]::StartNew()
        Invoke-WebRequest -Uri $url -OutFile $temp
        $actualSize = (Get-Item $temp).Length
        if ($actualSize -ne $size) {
            Remove-Item $temp  # 検証に失敗した取得物なので直接消してよい
            throw "サイズが一致しません: $name (取得元 $size、取得 $actualSize)"
        }
        Move-Item -Force $temp $dest
        Write-Host ("取得しました: {0} ({1:N0} 秒)" -f $dest, $watch.Elapsed.TotalSeconds) -ForegroundColor Green
    }
    $hash = (Get-FileHash -Algorithm SHA256 $dest).Hash.ToLowerInvariant()
    $rows += "$name`t$size`t$hash"
    $inputs += $dest
}

# 生成に使った分割だけを書く (UTF-8、BOM なし)
$lines = @("name`tsize`tsha256") + $rows
[System.IO.File]::WriteAllText($manifest, ($lines -join "`n") + "`n", (New-Object System.Text.UTF8Encoding $false))
Write-Host "書きました: $manifest" -ForegroundColor Green

Write-Host '=== next_word.tsv の生成' -ForegroundColor Cyan
cargo run --release --manifest-path (Join-Path $root 'engine\Cargo.toml') --example build_next_word -- `
    --out $output @inputs
if ($LASTEXITCODE -ne 0) { throw 'build_next_word に失敗しました' }
Write-Host "生成しました: $output" -ForegroundColor Green
