# QuicklIME AJIMEE-Bench の取得・変換スクリプト
#
# 変換精度の評価セット AJIMEE-Bench (azooKey) の JWTD_v2/v1/evaluation_items.json を、リポジトリ直下の
# references\AJIMEE-Bench\ (git 管理外) に取得し、サイズと SHA-256 を確かめてから nbest_eval の形式の
# ajimee_cases.tsv (左文脈<TAB>読み<TAB>候補<TAB>正解) に変換する。コミットは docs/design/eval-sets.md で
# 決めたものに固定している。取得済みでサイズとハッシュが一致するファイルは取り直さない。
# データは CC BY-SA 3.0 のため、リポジトリにもインストーラにも入れない。
#
# 使用例:
#   scripts\fetch-ajimee.ps1

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest の進捗表示は大きなファイルで極端に遅くなるため消す
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
$benchDir = Join-Path $root 'references\AJIMEE-Bench'
$output = Join-Path $benchDir 'ajimee_cases.tsv'

$file = @{
    Name     = 'evaluation_items.json'
    Repo     = 'azooKey/AJIMEE-Bench'
    Revision = '401666cd56d1a570c2021798b64b6da4396bfd45'
    File     = 'JWTD_v2/v1/evaluation_items.json'
    Size     = 132231
    Sha256   = 'e9eb668fd6aa14b1e26436f429b5550108af0a1dfd443b8cea0bcb3ab3028fca'
}

# JIS 配列のキーで直接打てない文字。読みに含む問題は除外する
$excluded = @([char]0x300E, [char]0x300F, [char]0x2026)  # 『 』 …

function Test-BenchFile([string]$path, $file) {
    if (-not (Test-Path $path)) {
        return $false
    }
    if ((Get-Item $path).Length -ne $file.Size) {
        return $false
    }
    return (Get-FileHash -Algorithm SHA256 $path).Hash -eq $file.Sha256.ToUpperInvariant()
}

# カタカナ (ァ〜ヶ) をひらがなにする。ヴ は ゔ になり、ー・記号はそのまま残す
function ConvertTo-Hiragana([string]$text) {
    $chars = $text.ToCharArray()
    for ($i = 0; $i -lt $chars.Length; $i++) {
        $code = [int]$chars[$i]
        if ($code -ge 0x30A1 -and $code -le 0x30F6) {
            $chars[$i] = [char]($code - 0x60)
        }
    }
    return -join $chars
}

New-Item -ItemType Directory -Force -Path $benchDir | Out-Null

$json = Join-Path $benchDir $file.Name
if (Test-BenchFile $json $file) {
    Write-Host "取得済み: $($file.Name)"
} else {
    $url = "https://github.com/$($file.Repo)/raw/$($file.Revision)/$($file.File)"
    # 途中で失敗したファイルを正しい名前で残さないよう、一時ファイルに取ってから置き換える
    $temp = "$json.download"
    Write-Host "取得中: $url"
    Invoke-WebRequest -Uri $url -OutFile $temp
    if (-not (Test-BenchFile $temp $file)) {
        $actualSize = (Get-Item $temp).Length
        $actualHash = (Get-FileHash -Algorithm SHA256 $temp).Hash
        Remove-Item $temp  # 検証に失敗した取得物なので直接消してよい
        throw "サイズまたはハッシュが一致しません: $($file.Name) (サイズ $actualSize、SHA-256 $actualHash)"
    }
    Move-Item -Force $temp $json
    Write-Host "取得しました: $json" -ForegroundColor Green
}

Write-Host '=== ajimee_cases.tsv への変換' -ForegroundColor Cyan
# Windows PowerShell 5.1 の Get-Content は BOM の無い UTF-8 を既定で読めないため、文字コードを指定して読む
$items = [System.IO.File]::ReadAllText($json, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
$lines = New-Object System.Collections.Generic.List[string]
$skipped = 0
foreach ($item in $items) {
    if ($item.input.IndexOfAny($excluded) -ge 0) {
        $skipped++
        continue
    }
    $reading = ConvertTo-Hiragana $item.input
    $answers = @($item.expected_output) -join '|'
    $lines.Add("$($item.context_text)`t$reading`t`t$answers")
}
# 何度実行しても同じバイト列になるよう、BOM なしの UTF-8 と LF で書く
$content = ($lines -join "`n") + "`n"
[System.IO.File]::WriteAllText($output, $content, (New-Object System.Text.UTF8Encoding $false))
Write-Host "変換しました: $output ($($items.Count) 件中 $($lines.Count) 件、除外 $skipped 件)" -ForegroundColor Green
