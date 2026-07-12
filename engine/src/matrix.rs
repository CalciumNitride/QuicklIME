// 連接コスト行列 (Mozc の connection_single_column.txt)
//
// フォーマット: 1行目が次元 N、以降 N*N 行のコスト値。
// コストの参照は「前の単語の右文脈ID を行、次の単語の左文脈ID を列」とする
// (index = right_id * N + left_id)。

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

pub struct ConnectionMatrix {
    size: usize,
    costs: Vec<i16>,
}

impl ConnectionMatrix {
    /// 空の行列 (連接コストは常に 0 になる。行列ファイルが無い場合のフォールバック)
    pub fn empty() -> Self {
        ConnectionMatrix { size: 0, costs: Vec::new() }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let mut text = String::new();
        File::open(path)?.read_to_string(&mut text)?;
        Self::parse(&text)
    }

    /// テキスト全体をパースする (テストからも使う)
    pub fn parse(text: &str) -> io::Result<Self> {
        let invalid = |msg: &str| io::Error::new(io::ErrorKind::InvalidData, msg.to_string());

        let mut tokens = text.split_ascii_whitespace();
        let size: usize = tokens
            .next()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| invalid("次元行がありません"))?;

        let mut costs = Vec::with_capacity(size * size);
        for token in tokens {
            let cost: i16 = token.parse().map_err(|_| invalid("コスト値が不正です"))?;
            costs.push(cost);
        }
        if costs.len() != size * size {
            return Err(invalid("コスト値の個数が次元と一致しません"));
        }
        Ok(ConnectionMatrix { size, costs })
    }

    /// 前の単語の右文脈ID と次の単語の左文脈ID から連接コストを返す。
    /// 空行列・範囲外は 0 (連接コストなし) として扱う
    pub fn get(&self, right_id: u16, left_id: u16) -> i32 {
        let (r, l) = (right_id as usize, left_id as usize);
        if r >= self.size || l >= self.size {
            return 0;
        }
        i32::from(self.costs[r * self.size + l])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 行列を読み込んで参照できる() {
        let m = ConnectionMatrix::parse("2\n0\n10\n20\n30\n").unwrap();
        assert_eq!(m.get(0, 0), 0);
        assert_eq!(m.get(0, 1), 10);
        assert_eq!(m.get(1, 0), 20);
        assert_eq!(m.get(1, 1), 30);
    }

    #[test]
    fn 範囲外と空行列は0() {
        let m = ConnectionMatrix::parse("2\n0\n10\n20\n30\n").unwrap();
        assert_eq!(m.get(5, 0), 0);
        assert_eq!(ConnectionMatrix::empty().get(0, 0), 0);
    }

    #[test]
    fn 個数不一致はエラー() {
        assert!(ConnectionMatrix::parse("2\n0\n10\n20\n").is_err());
    }
}
