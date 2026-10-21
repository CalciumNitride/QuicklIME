// 設定ツール・単語登録ツールの画面に依存しない処理

pub mod apps;
pub mod config;
// 他 IME の辞書の取り込みはエンジン本体と共有する
#[path = "../../src/import.rs"]
pub mod import;
pub mod keys;
pub mod regword;
pub mod win32;
