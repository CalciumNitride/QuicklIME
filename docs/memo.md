# 参考リンク
azookey [text](https://azookey.com)


# アイデア
- アプリケーションごとの学習
- モードレス入力

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