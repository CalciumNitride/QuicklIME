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