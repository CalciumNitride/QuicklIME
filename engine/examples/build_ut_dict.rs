// UT 辞書 (dictionary_ut.txt) の生成 (docs/design/dict-expansion.md)
//
// mozc-ut の元データ (SudachiDict・地名・人名の .bz2) を読み、Mozc 辞書に既にある語・
// 1 文字や長すぎる表記・ASCII だけの表記・元データ間の重複を除いて、品詞 ID を付けた
// Mozc 形式の辞書を書き出す。元データの取得と実行は scripts\fetch-ut-dict.ps1 が行う。
//
// 使い方:
//   cargo run --release --example build_ut_dict -- --mozc <dictionary_oss> --ut <references\mozcdic-ut> --out <出力> [--cost-offset N]
//
// オプション:
//   --mozc <dir>       Mozc の dictionary_oss (dictionary00〜09.txt と id.def)
//   --ut <dir>         元データ (.bz2) の置き場所
//   --out <file>       出力先 (UTF-8、BOM なし、読みでソート)
//   --cost-offset N    全エントリのコストに足す値 (既定 0。i16 に収まらない値はクリップする)

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

/// 元データ。並びが重複時の優先順 (上ほど優先) になる
struct Source {
    file: &'static str,
    /// id.def の品詞名 (2 列目と完全一致させる)
    pos: &'static str,
}

const SOURCES: [Source; 3] = [
    Source { file: "mozcdic-ut-sudachidict.txt.bz2", pos: "名詞,一般,*,*,*,*,*" },
    Source { file: "mozcdic-ut-place-names.txt.bz2", pos: "名詞,固有名詞,地域,一般,*,*,*" },
    Source { file: "mozcdic-ut-personal-names.txt.bz2", pos: "名詞,固有名詞,人名,一般,*,*,*" },
];

/// これ以上の文字数の表記は除く
const MAX_SURFACE_CHARS: usize = 26;

/// 除外の理由
#[derive(Debug, PartialEq, Clone, Copy)]
enum Exclusion {
    /// 表記が 1 文字、または MAX_SURFACE_CHARS 文字以上
    Length,
    /// 表記が ASCII だけ (英単語はモードレス入力の英単語辞書が受け持つ)
    Ascii,
    /// (読み, 正規化した表記) が Mozc 辞書にある
    InMozc,
    /// (読み, 正規化した表記) が先に出力したエントリと重複する
    Duplicate,
}

const EXCLUSIONS: [(Exclusion, &str); 4] = [
    (Exclusion::Length, "表記の文字数"),
    (Exclusion::Ascii, "ASCII だけの表記"),
    (Exclusion::InMozc, "Mozc 辞書にある"),
    (Exclusion::Duplicate, "元データ間の重複"),
];

/// 表記の正規化 (mozc-ut と同じ): NFKC にしてから ~ を 〜 にする
fn normalize_surface(surface: &str) -> String {
    surface.nfkc().collect::<String>().replace('~', "〜")
}

/// 除外すべき行ならその理由を返す。surface は正規化済みの表記
fn exclusion(
    reading: &str,
    surface: &str,
    mozc: &HashSet<(String, String)>,
    seen: &HashSet<(String, String)>,
) -> Option<Exclusion> {
    let chars = surface.chars().count();
    if chars <= 1 || chars >= MAX_SURFACE_CHARS {
        return Some(Exclusion::Length);
    }
    if surface.is_ascii() {
        return Some(Exclusion::Ascii);
    }
    let key = (reading.to_string(), surface.to_string());
    if mozc.contains(&key) {
        return Some(Exclusion::InMozc);
    }
    if seen.contains(&key) {
        return Some(Exclusion::Duplicate);
    }
    None
}

/// id.def ("<ID> <品詞名>" の行) から品詞名が完全一致する ID を引く
fn find_pos_id(id_def: impl BufRead, name: &str) -> BoxResult<Option<u16>> {
    for line in id_def.lines() {
        let line = line?;
        if let Some((id, pos)) = line.split_once(' ') {
            if pos == name {
                return Ok(Some(id.parse()?));
            }
        }
    }
    Ok(None)
}

/// 元データのコストに offset を足し、i16 に収まらなければクリップする
fn adjusted_cost(cost: i32, offset: i32) -> i16 {
    cost.saturating_add(offset).clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// 元データの 1 行 (読み\tID\tID\tコスト\t表記) を (読み, コスト, 表記) に分ける
fn parse_line(line: &str) -> Option<(&str, i32, &str)> {
    let mut fields = line.split('\t');
    let reading = fields.next()?;
    let _left = fields.next()?;
    let _right = fields.next()?;
    let cost = fields.next()?.parse().ok()?;
    let surface = fields.next()?;
    if reading.is_empty() || surface.is_empty() {
        return None;
    }
    Some((reading, cost, surface))
}

/// Mozc 辞書 (dictionary00〜09.txt) の (読み, 正規化した表記) を集める
fn load_mozc_pairs(dir: &Path) -> BoxResult<HashSet<(String, String)>> {
    let mut pairs = HashSet::new();
    let mut found = false;
    for i in 0..10 {
        let path = dir.join(format!("dictionary{i:02}.txt"));
        if !path.is_file() {
            continue;
        }
        found = true;
        for line in BufReader::new(File::open(&path)?).lines() {
            let line = line?;
            if let Some((reading, _, surface)) = parse_line(&line) {
                pairs.insert((reading.to_string(), normalize_surface(surface)));
            }
        }
    }
    if !found {
        return Err(format!("Mozc 辞書が見つかりません: {}", dir.display()).into());
    }
    Ok(pairs)
}

struct Args {
    mozc: PathBuf,
    ut: PathBuf,
    out: PathBuf,
    cost_offset: i32,
}

fn parse_args() -> BoxResult<Args> {
    let (mut mozc, mut ut, mut out) = (None, None, None);
    let mut cost_offset = 0;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--mozc" => mozc = Some(PathBuf::from(value()?)),
            "--ut" => ut = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--cost-offset" => cost_offset = value()?.parse()?,
            other => return Err(format!("不明な引数: {other}").into()),
        }
    }
    let usage = "使い方: build_ut_dict --mozc <dictionary_oss> --ut <dir> --out <file> [--cost-offset N]";
    Ok(Args {
        mozc: mozc.ok_or(usage)?,
        ut: ut.ok_or(usage)?,
        out: out.ok_or(usage)?,
        cost_offset,
    })
}

/// 元データごとの集計
#[derive(Default)]
struct Stats {
    read: usize,
    invalid: usize,
    excluded: [usize; EXCLUSIONS.len()],
    written: usize,
}

fn main() -> BoxResult<()> {
    let args = parse_args()?;

    let id_def = args.mozc.join("id.def");
    let mut pos_ids = Vec::new();
    for source in &SOURCES {
        let id = find_pos_id(BufReader::new(File::open(&id_def)?), source.pos)?
            .ok_or_else(|| format!("id.def に品詞がありません: {}", source.pos))?;
        pos_ids.push(id);
    }

    eprintln!("Mozc 辞書を読み込み中: {}", args.mozc.display());
    let mozc = load_mozc_pairs(&args.mozc)?;

    let mut seen: HashSet<(String, String)> = HashSet::new();
    // (読み, ID, コスト, 表記)
    let mut entries: Vec<(String, u16, i16, String)> = Vec::new();
    let mut stats: Vec<Stats> = Vec::new();
    for (source, &id) in SOURCES.iter().zip(&pos_ids) {
        let path = args.ut.join(source.file);
        eprintln!("読み込み中: {}", path.display());
        let reader = BufReader::new(bzip2::read::MultiBzDecoder::new(File::open(&path)?));
        let mut stat = Stats::default();
        for line in reader.lines() {
            let line = line?;
            stat.read += 1;
            let Some((reading, cost, surface)) = parse_line(&line) else {
                stat.invalid += 1;
                continue;
            };
            let surface = normalize_surface(surface);
            if let Some(reason) = exclusion(reading, &surface, &mozc, &seen) {
                let i = EXCLUSIONS.iter().position(|(r, _)| *r == reason).unwrap();
                stat.excluded[i] += 1;
                continue;
            }
            seen.insert((reading.to_string(), surface.clone()));
            entries.push((reading.to_string(), id, adjusted_cost(cost, args.cost_offset), surface));
            stat.written += 1;
        }
        stats.push(stat);
    }

    // 安定ソートで、同じ読みの中は元データの優先順 → 記載順を保つ
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = BufWriter::new(File::create(&args.out)?);
    for (reading, id, cost, surface) in &entries {
        writeln!(out, "{reading}\t{id}\t{id}\t{cost}\t{surface}")?;
    }
    out.flush()?;

    println!("コストの加算: {}", args.cost_offset);
    for ((source, id), stat) in SOURCES.iter().zip(&pos_ids).zip(&stats) {
        println!("{} (品詞 ID {id}: {})", source.file, source.pos);
        println!("  読み込み: {}", stat.read);
        println!("  不正な行: {}", stat.invalid);
        for ((_, label), count) in EXCLUSIONS.iter().zip(&stat.excluded) {
            println!("  除外 ({label}): {count}");
        }
        println!("  出力: {}", stat.written);
    }
    println!("出力件数: {} [{}]", entries.len(), args.out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> HashSet<(String, String)> {
        list.iter().map(|(r, s)| (r.to_string(), s.to_string())).collect()
    }

    #[test]
    fn 表記をnfkcにしてチルダを波ダッシュにする() {
        assert_eq!(normalize_surface("ＡＢＣ～ﾃｽﾄ"), "ABC〜テスト");
        assert_eq!(normalize_surface("a~b"), "a〜b");
        assert_eq!(normalize_surface("東京"), "東京");
    }

    #[test]
    fn 表記の文字数で除外する() {
        let empty = HashSet::new();
        assert_eq!(exclusion("あ", "亜", &empty, &empty), Some(Exclusion::Length));
        let long: String = "長".repeat(MAX_SURFACE_CHARS);
        assert_eq!(exclusion("ながい", &long, &empty, &empty), Some(Exclusion::Length));
        let ok: String = "長".repeat(MAX_SURFACE_CHARS - 1);
        assert_eq!(exclusion("ながい", &ok, &empty, &empty), None);
        assert_eq!(exclusion("あい", "藍色", &empty, &empty), None);
    }

    #[test]
    fn asciiだけの表記を除外し和字混じりは残す() {
        let empty = HashSet::new();
        assert_eq!(exclusion("あーす", "Ah!SKI", &empty, &empty), Some(Exclusion::Ascii));
        assert_eq!(exclusion("えーびーしー", "ABC社", &empty, &empty), None);
    }

    #[test]
    fn mozc辞書にある語と先に出した語を除外する() {
        let mozc = pairs(&[("とうきょう", "東京")]);
        let seen = pairs(&[("かんべ", "神戸")]);
        assert_eq!(exclusion("とうきょう", "東京", &mozc, &seen), Some(Exclusion::InMozc));
        assert_eq!(exclusion("かんべ", "神戸", &mozc, &seen), Some(Exclusion::Duplicate));
        // 読みが違えば別の語
        assert_eq!(exclusion("こうべ", "神戸", &mozc, &seen), None);
    }

    #[test]
    fn 品詞名が完全一致するidを引く() {
        let id_def = "1851 名詞,一般,*,*,*,*,*\n\
                      1921 名詞,固有名詞,人名,一般,*,*,*\n\
                      1924 名詞,固有名詞,地域,一般,*,*,*\n";
        let find = |name| find_pos_id(id_def.as_bytes(), name).unwrap();
        assert_eq!(find("名詞,一般,*,*,*,*,*"), Some(1851));
        assert_eq!(find("名詞,固有名詞,地域,一般,*,*,*"), Some(1924));
        assert_eq!(find("名詞,固有名詞,人名,一般,*,*,*"), Some(1921));
        // 前方一致では引かない
        assert_eq!(find("名詞,固有名詞"), None);
    }

    #[test]
    fn コストに加算しi16に収まらなければクリップする() {
        assert_eq!(adjusted_cost(8999, 0), 8999);
        assert_eq!(adjusted_cost(8999, 1000), 9999);
        assert_eq!(adjusted_cost(30000, 5000), i16::MAX);
        assert_eq!(adjusted_cost(-30000, -5000), i16::MIN);
    }

    #[test]
    fn 元データの行を分ける() {
        assert_eq!(parse_line("あい\t0000\t0000\t8999\t藍"), Some(("あい", 8999, "藍")));
        assert_eq!(parse_line("あい\t0000\t0000\tx\t藍"), None);
        assert_eq!(parse_line("あい\t0000"), None);
    }
}
