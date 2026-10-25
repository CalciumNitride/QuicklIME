// 設定ファイル (config.tsv) の読み込み
//
// フォーマット: キー\t値 の行ベース TSV。# 始まりの行はコメント。
// 未知キーは無視し、不正な値はそのキーだけ既定値に落とす (エラーにしない)。
// TSF 層向けのキー (space, punctuation など) もここでは未知キーとして無視される。
// 書き込みは設定ツール (quicklime-config.exe) が行い、エンジンは読むだけ。
// 変更の反映は RELOADCONFIG コマンドによる再読込で行う。
// 保存先: %APPDATA%\QuicklIME\config.tsv (QUICKLIME_CONFIG_FILE で上書き可)

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// エンジン側の設定。小さい値の集まりなので、リクエスト処理では
/// Mutex から Copy で取り出してロックをすぐ手放す
#[derive(Clone, Copy)]
pub struct Config {
    /// 確定履歴を学習するか (無効でも既存の学習データは使い続ける)
    pub learning: bool,
    /// 予測サジェスト (PREDICT) を返すか
    pub suggest: bool,
    /// タイプミス補正 (曖昧一致での候補補充) を使うか
    pub typo_correction: bool,
    /// 予測候補の最大数。TSF 層では候補バーの合計の上限として使う。
    /// 候補バーの最後のページも埋まるよう 1 ページ (9 件) の倍数にそろえる
    pub max_predictions: usize,
    /// 予測を出す最小の読み文字数
    pub min_suggest_chars: usize,
    /// LLM による候補の並べ替え (RERANK) を使うか
    pub llm: bool,
    /// LLM の子プロセスのバックエンド
    pub llm_backend: LlmBackend,
}

/// LLM の推論に使うバックエンド (docs/spec/engine.md「モデルと尤度」)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LlmBackend {
    Cpu,
    Vulkan,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            learning: true,
            suggest: true,
            typo_correction: true,
            max_predictions: 9,
            min_suggest_chars: 2,
            llm: false,
            llm_backend: LlmBackend::Cpu,
        }
    }
}

impl Config {
    /// 既定のパスから読み込む。ファイルが無ければ既定値で始める
    pub fn load_default() -> Self {
        let Some(path) = default_path() else {
            return Config::default();
        };
        match File::open(&path) {
            Ok(file) => Config::load_from(BufReader::new(file)),
            Err(_) => Config::default(),
        }
    }

    /// 1ファイル分の設定を読み込む (テストからも使う)。不正な行・値は既定値のまま
    pub fn load_from(reader: impl BufRead) -> Self {
        let mut config = Config::default();
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if line.starts_with('#') {
                continue; // コメント行
            }
            let Some((key, value)) = line.split_once('\t') else {
                continue;
            };
            match key {
                "learning" => parse_bool(value, &mut config.learning),
                "suggest" => parse_bool(value, &mut config.suggest),
                "typo_correction" => parse_bool(value, &mut config.typo_correction),
                "max_predictions" => parse_page_multiple(value, &mut config.max_predictions),
                "min_suggest_chars" => parse_clamped(value, 1, 5, &mut config.min_suggest_chars),
                "llm" => parse_bool(value, &mut config.llm),
                "llm_backend" => match value {
                    "cpu" => config.llm_backend = LlmBackend::Cpu,
                    "vulkan" => config.llm_backend = LlmBackend::Vulkan,
                    _ => {}
                },
                _ => {} // 未知キー (TSF 層向けを含む) は無視
            }
        }
        config
    }
}

/// "0"/"1" を bool にする。それ以外は変更しない
fn parse_bool(value: &str, out: &mut bool) {
    match value {
        "0" => *out = false,
        "1" => *out = true,
        _ => {}
    }
}

/// 整数として読めれば [min, max] に clamp して設定する。読めなければ変更しない
fn parse_clamped(value: &str, min: usize, max: usize, out: &mut usize) {
    if let Ok(n) = value.parse::<usize>() {
        *out = n.clamp(min, max);
    }
}

/// 整数として読めれば 9 の倍数へ切り上げて [9, 36] に丸めて設定する。読めなければ変更しない。
/// TSF 層 (config.cpp) と設定ツール (quicklime-config.rs) も同じ規則で読む
fn parse_page_multiple(value: &str, out: &mut usize) {
    if let Ok(n) = value.parse::<usize>() {
        *out = n.clamp(9, 36).div_ceil(9) * 9;
    }
}

/// 既定の設定ファイルパス。優先順: QUICKLIME_CONFIG_FILE > %APPDATA%\QuicklIME\config.tsv
fn default_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("QUICKLIME_CONFIG_FILE") {
        return Some(PathBuf::from(path));
    }
    let appdata = std::env::var("APPDATA").ok()?;
    Some(PathBuf::from(appdata).join("QuicklIME").join("config.tsv"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn 既定値で始まる() {
        let config = Config::load_from(Cursor::new(""));
        assert!(config.learning);
        assert!(config.suggest);
        assert!(config.typo_correction);
        assert_eq!(config.max_predictions, 9);
        assert_eq!(config.min_suggest_chars, 2);
        assert!(!config.llm);
        assert_eq!(config.llm_backend, LlmBackend::Cpu);
    }

    #[test]
    fn llmの設定を読み込める() {
        let config = Config::load_from(Cursor::new("llm\t1\nllm_backend\tvulkan\n"));
        assert!(config.llm);
        assert_eq!(config.llm_backend, LlmBackend::Vulkan);
        // 不明なバックエンドは既定のまま
        let config = Config::load_from(Cursor::new("llm_backend\tcuda\n"));
        assert_eq!(config.llm_backend, LlmBackend::Cpu);
    }

    #[test]
    fn 設定を読み込める() {
        let text = "learning\t0\nsuggest\t0\ntypo_correction\t0\nmax_predictions\t27\nmin_suggest_chars\t3\n";
        let config = Config::load_from(Cursor::new(text));
        assert!(!config.learning);
        assert!(!config.suggest);
        assert!(!config.typo_correction);
        assert_eq!(config.max_predictions, 27);
        assert_eq!(config.min_suggest_chars, 3);
    }

    #[test]
    fn 数値は範囲に丸められる() {
        let text = "max_predictions\t100\nmin_suggest_chars\t0\n";
        let config = Config::load_from(Cursor::new(text));
        assert_eq!(config.max_predictions, 36);
        assert_eq!(config.min_suggest_chars, 1);
    }

    #[test]
    fn 予測の件数は9の倍数へ切り上げる() {
        let cases = [
            ("0", 9), ("1", 9), ("8", 9), ("9", 9), ("10", 18), ("18", 18),
            ("19", 27), ("30", 36), ("36", 36), ("37", 36),
        ];
        for (value, expected) in cases {
            let config = Config::load_from(Cursor::new(format!("max_predictions\t{value}\n")));
            assert_eq!(config.max_predictions, expected, "max_predictions {value}");
        }
    }

    #[test]
    fn 不正な値と未知キーは無視する() {
        let text = "# コメント\nlearning\tabc\nmax_predictions\txyz\nspace\tfull\nunknown_key\t1\n壊れた行\n";
        let config = Config::load_from(Cursor::new(text));
        assert!(config.learning);
        assert_eq!(config.max_predictions, 9);
    }
}
