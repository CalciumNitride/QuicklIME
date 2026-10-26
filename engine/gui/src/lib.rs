// 設定ツール・単語登録ツールの画面に依存しない処理

pub mod apps;
pub mod config;
// 他 IME の辞書の取り込みはエンジン本体と共有する
#[path = "../../src/import.rs"]
pub mod import;
pub mod keys;
// AppContainer のアプリに設定ファイルの読み取りを許す処理はエンジン本体と共有する
#[path = "../../src/package_acl.rs"]
pub mod package_acl;
pub mod preview;
pub mod regword;
pub mod win32;
