// テキストファイルの行単位の読み込み (辞書・学習・ユーザ辞書・設定で共有する)
//
// UTF-8 として読めない行は、その行だけを読み飛ばして続きを読む。読めない行で後ろを読まずにいると、
// ファイル全体を書き直す保存でその後半が消え、追記した行も次の起動で読まれないため。
// U+FFFD に置き換えて読まないのは、置き換えた読みや表記が正しい語として学習やユーザ辞書に入るため

use std::io::{self, BufRead};
use std::path::Path;

/// reader を 1 行ずつ f に渡す。UTF-8 として読めない行は飛ばし、その数を返す。
/// それ以外の I/O エラーではそこで止め、エラーを返す
pub fn read_lines(reader: impl BufRead, mut f: impl FnMut(String)) -> io::Result<usize> {
    let mut skipped = 0;
    // lines() は不正な UTF-8 の行で InvalidData を返すが、その行のバイト列は読み終えているので
    // 次の呼び出しは次の行から続く。ほかのエラーは同じエラーが返り続けうるので止める
    for line in reader.lines() {
        match line {
            Ok(line) => f(line),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => skipped += 1,
            Err(e) => return Err(e),
        }
    }
    Ok(skipped)
}

/// 読み飛ばした行があれば、その数とファイルをログに出す
pub fn report_skipped(skipped: usize, path: &Path) {
    if skipped > 0 {
        eprintln!("UTF-8 として読めない行を {skipped} 行読み飛ばしました [{}]", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(data: &[u8]) -> (Vec<String>, io::Result<usize>) {
        let mut lines = Vec::new();
        let result = read_lines(data, |line| lines.push(line));
        (lines, result)
    }

    #[test]
    fn 読めない行だけを飛ばして続きを読む() {
        let (lines, result) = collect(b"one\n\xff\xfe\nthree\n");
        assert_eq!(lines, ["one", "three"]);
        assert_eq!(result.unwrap(), 1);
        let (lines, result) = collect("一\n二\n".as_bytes());
        assert_eq!(lines, ["一", "二"]);
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn 改行のない最終行が読めなければそれまでの行を渡す() {
        let (lines, result) = collect(b"one\ntwo\n\xff\xfe");
        assert_eq!(lines, ["one", "two"]);
        assert_eq!(result.unwrap(), 1);
    }

    /// 1 行読んだあとに読み込みのエラーを返す reader
    struct Failing {
        served: bool,
    }

    impl io::Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            unreachable!()
        }
    }

    impl BufRead for Failing {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if self.served {
                Err(io::Error::other("読み込みのエラー"))
            } else {
                Ok(b"one\n")
            }
        }
        fn consume(&mut self, _: usize) {
            self.served = true;
        }
    }

    #[test]
    fn 読めない行以外のエラーではそこで止めてエラーを返す() {
        let mut lines = Vec::new();
        let result = read_lines(Failing { served: false }, |line| lines.push(line));
        assert_eq!(lines, ["one"]);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Other);
    }
}
