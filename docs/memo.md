# 参考リンク
azookey [text](https://azookey.com)

# 常用時の確認ポイント
5. 普段の文章をいくつか打って、これまで正しく分割されていた文が不自然に繋がっていないか確認 (もし過剰にまとまる例があれば SEGMENT_PENALTY=1000 を下げて調整できるので教えてください)

## 気になった点

# アイデア
- アプリケーションごとの学習
- モードレス入力
- ローマ字テーブル編集を設定ウィンドウから
- 選択ウィンドウのUI改善
- 文脈予測
- 

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


# 旧ファイル
tsf/build/Debug/QuicklIME.dll.old と engine/target/release/quicklime-engine.exe.old を削除する
(tsf/build/Debug/QuicklIME.dll.old2 も)

# 開発版とインストール版の切替
インストール版 (%ProgramFiles%\QuicklIME) と開発版 (tsf/build/Debug) は同じ CLSID で、
regsvr32 した方に InprocServer32 が切り替わる (排他)。

```
# 開発版に切替 (要管理者権限)
regsvr32 tsf\build\Debug\QuicklIME.dll
# インストール版に戻す
regsvr32 "%ProgramFiles%\QuicklIME\QuicklIME.dll"
```

エンジンの探索順 (FindExePath) は「DLL と同じフォルダ → 親 → engine/target/release →
debug」なので、開発版 DLL でもインストール先ではなく開発ツリーのエンジンが起動する点に注意
(逆にインストール版 DLL は %ProgramFiles% のエンジン + dict を使う)。

# 常用環境への変更の適用
ビルド後、
## 常用エンジンを終了
taskkill /IM quicklime-engine.exe /F
## 新しいexeをコピー
copy engine\target\release\quicklime-engine.exe "C:\Program Files\QuicklIME\"