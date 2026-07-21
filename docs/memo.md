# 参考リンク
azookey [text](https://azookey.com)

# 常用時の確認ポイント
5. 普段の文章をいくつか打って、これまで正しく分割されていた文が不自然に繋がっていないか確認 (もし過剰にまとまる例があれば SEGMENT_PENALTY=1000 を下げて調整できるので教えてください)

## 気になった点

# アイデア
- アプリケーションごとの学習
- モードレス入力
- ローマ字テーブル編集を設定ウィンドウから
- 文脈予測
- UIカスタマイズ

# エンジン終了

- 停止

```shell
Stop-Process -Name quicklime-engine -Force -ErrorAction SilentlyContinue
```

- 起動(手動起動は基本的に不要)

```
Start-Process -FilePath "D:\project\QuicklIME\engine\target\release\quicklime-engine.exe" -WindowStyle Hidden
```

- 確認

```
Get-Process quicklime-engine -ErrorAction SilentlyContinue | Select-Object Id, StartTime, Path
```


# 常用環境への変更の適用
## Rustエンジンの変更を反映
- 常用エンジンを終了
taskkill /IM quicklime-engine.exe /F
- 新しいexeをコピー
copy engine\target\release\quicklime-engine.exe "C:\Program Files\QuicklIME\"

## TSF DLLの変更を反映
開発版dllをレジストリに登録
```
regsvr32 tsf\build\Debug\QuicklIME.dll
```
インストール版に戻す
```
regsvr32 "C:\Program Files\QuicklIME\QuicklIME.dll"
```

ビルド
```
cargo build --release
```
ビルド時、旧dllがロックされている場合、リネームして退避
```
mv QuicklIME.dll QuicklIME.dll.old 
```

※切り替えは新規プロセスにのみ有効