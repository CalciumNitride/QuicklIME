// 品詞ID表 (Mozc の id.def) の読み込み
//
// フォーマット: "<ID> <品詞情報 (カンマ区切り)>" が1行ずつ。
// 文節区切りでは「付属語 (助詞・助動詞・接尾辞) かどうか」だけを使う。

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

/// 付属語 (前の自立語と同じ文節にまとめる品詞) の ID 集合
pub struct FunctionalIds {
    is_functional: Vec<bool>,
}

impl FunctionalIds {
    /// 空 (すべて自立語扱い。id.def が無い場合のフォールバック)
    pub fn empty() -> Self {
        FunctionalIds { is_functional: Vec::new() }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        Self::load_from(BufReader::new(File::open(path)?))
    }

    pub fn load_from(reader: impl BufRead) -> io::Result<Self> {
        let mut is_functional = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let Some((id, pos)) = line.split_once(' ') else {
                continue;
            };
            let Ok(id) = id.parse::<usize>() else {
                continue;
            };
            let functional =
                pos.starts_with("助詞") || pos.starts_with("助動詞") || pos.starts_with("接尾辞");
            if id >= is_functional.len() {
                is_functional.resize(id + 1, false);
            }
            is_functional[id] = functional;
        }
        Ok(FunctionalIds { is_functional })
    }

    /// この品詞IDの単語を前の文節にまとめるべきか
    pub fn is_functional(&self, id: u16) -> bool {
        self.is_functional.get(id as usize).copied().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 助詞と助動詞と接尾辞が付属語になる() {
        let data = "1 名詞,一般,*,*,*,*,*\n\
                    2 助詞,係助詞,*,*,*,*,は\n\
                    3 助動詞,*,*,*,特殊・デス,基本形,です\n\
                    4 接尾辞,人名,*,*,*,*,さん\n\
                    5 動詞,自立,*,*,*,*,*\n";
        let ids = FunctionalIds::load_from(data.as_bytes()).unwrap();
        assert!(!ids.is_functional(1));
        assert!(ids.is_functional(2));
        assert!(ids.is_functional(3));
        assert!(ids.is_functional(4));
        assert!(!ids.is_functional(5));
    }

    #[test]
    fn 未定義のidは自立語扱い() {
        assert!(!FunctionalIds::empty().is_functional(100));
    }
}
