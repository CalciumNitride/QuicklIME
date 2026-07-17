// 品詞ID表 (Mozc の id.def) の読み込み
//
// フォーマット: "<ID> <品詞情報 (カンマ区切り)>" が1行ずつ。
// 文節区切りの「付属語 (助詞・助動詞・接尾辞) かどうか」の判定と、
// ユーザ辞書の品詞名 → 文脈ID の解決 (find_id) に使う。

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

/// Mozc 辞書の一般名詞相当の文脈ID。
/// id.def が無い・引けないときの未知語ノードとユーザ登録語のフォールバック
pub const DEFAULT_NOUN_ID: u16 = 1851;

/// 品詞ID表。付属語判定とユーザ辞書の品詞名解決に使う
pub struct FunctionalIds {
    is_functional: Vec<bool>,
    /// ID → 品詞パス (id.def の2列目)。未定義IDは空文字列
    names: Vec<String>,
}

impl FunctionalIds {
    /// 空 (すべて自立語扱い。id.def が無い場合のフォールバック)
    pub fn empty() -> Self {
        FunctionalIds { is_functional: Vec::new(), names: Vec::new() }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        Self::load_from(BufReader::new(File::open(path)?))
    }

    pub fn load_from(reader: impl BufRead) -> io::Result<Self> {
        let mut is_functional = Vec::new();
        let mut names = Vec::new();
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
                names.resize(id + 1, String::new());
            }
            is_functional[id] = functional;
            names[id] = pos.to_string();
        }
        Ok(FunctionalIds { is_functional, names })
    }

    /// この品詞IDの単語を前の文節にまとめるべきか
    pub fn is_functional(&self, id: u16) -> bool {
        self.is_functional.get(id as usize).copied().unwrap_or(false)
    }

    /// 品詞パスが prefix で始まる最初のIDを返す (ユーザ辞書の品詞名解決用)。
    /// 呼ばれるのは登録・読み込み時のみなので線形走査で足りる
    pub fn find_id(&self, prefix: &str) -> Option<u16> {
        self.names.iter().position(|n| n.starts_with(prefix)).map(|i| i as u16)
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

    #[test]
    fn 品詞パスの前方一致でidが引ける() {
        let data = "1850 名詞,サ変接続,*,*,*,*,*\n\
                    1851 名詞,一般,*,*,*,*,*\n\
                    1917 名詞,固有名詞,人名,姓,*,*,*\n";
        let ids = FunctionalIds::load_from(data.as_bytes()).unwrap();
        assert_eq!(ids.find_id("名詞,一般"), Some(1851));
        assert_eq!(ids.find_id("名詞,固有名詞,人名,姓"), Some(1917));
        assert_eq!(ids.find_id("動詞"), None);
        assert_eq!(FunctionalIds::empty().find_id("名詞,一般"), None);
    }
}
