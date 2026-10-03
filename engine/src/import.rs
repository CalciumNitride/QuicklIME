// 他 IME のユーザ辞書 (MS-IME / ATOK / Mozc・Google 日本語入力) の取り込み
//
// 文字コード判定・形式判別・品詞対応・行パース・重複除去・件数集計を行う純粋なロジック。
// ファイル入出力と CP932 のデコードは呼び出し側 (単語登録ツール) が受け持つ。
// 出力は QuicklIME の品詞名に正規化した「読み\t表記\t品詞」で、
// エンジンが imported\*.tsv として読み込む (userdict.rs)。

use std::collections::HashSet;

/// 取り込み元の形式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    MsIme,
    Atok,
    /// Mozc / Google 日本語入力 (判別できない場合もこれとして扱う)
    Mozc,
}

/// 取り込み結果
#[derive(Debug, Default)]
pub struct ImportResult {
    /// 取り込む語 (読み, 表記, QuicklIME の品詞名)。ファイル記載順
    pub entries: Vec<(String, String, &'static str)>,
    pub duplicate: usize,
    pub unsupported_pos: usize,
    pub invalid: usize,
}

/// バイト列を文字列にデコードする。BOM → UTF-8 の妥当性 → CP932 の順に判定する。
/// CP932 のデコードは Win32 API に頼るため呼び出し側から注入する (失敗時は None)
pub fn decode(
    bytes: &[u8],
    cp932: impl FnOnce(&[u8]) -> Option<String>,
) -> Result<String, String> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, u16::from_be_bytes);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(body) {
        return Ok(text.to_string());
    }
    cp932(bytes).ok_or_else(|| "文字コードを判別できません".to_string())
}

fn decode_utf16(bytes: &[u8], to_unit: fn([u8; 2]) -> u16) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("UTF-16 のデータが途中で切れています".to_string());
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| to_unit([c[0], c[1]])).collect();
    String::from_utf16(&units).map_err(|_| "UTF-16 として読めない文字が含まれています".to_string())
}

/// デコード済みテキストの先頭行から形式を判別する
pub fn detect_format(text: &str) -> Format {
    let first = text.lines().next().unwrap_or("");
    if first.starts_with("!Microsoft IME Dictionary Tool") {
        Format::MsIme
    } else if first.starts_with("!!ATOK_TANGO_TEXT_HEADER_1") {
        Format::Atok
    } else {
        Format::Mozc
    }
}

/// 他 IME の品詞名を QuicklIME の品詞名へ対応づける。対応外は None。
/// 完全一致の表を先に引き、無ければ「固有」始まり → 「名詞」を含む の順で規則判定する
pub fn map_pos(pos: &str) -> Option<&'static str> {
    // ATOK は自動登録語・手動登録語の区別に品詞末尾へ $ や * を付ける (Mozc の取り込みと同じ扱い)
    let pos = pos.trim();
    let pos = pos.strip_suffix('$').unwrap_or(pos);
    let pos = pos.strip_suffix('*').unwrap_or(pos);
    let exact = match pos {
        "人名" | "固有人他" | "固有人名(姓名)" => Some("人名"),
        "姓" | "固有人姓" => Some("姓"),
        "名" | "固有人名" => Some("名"),
        "地名" | "固有地名" | "地名その他" => Some("地名"),
        "組織" | "固有組織" | "社名" => Some("組織"),
        "固有名詞" | "固有一般" | "固有商品" | "物品" => Some("固有名詞"),
        "短縮よみ" | "短縮読み" | "顔文字" | "記号" => Some("短縮よみ"),
        _ => None,
    };
    if exact.is_some() {
        return exact;
    }
    // 「その他の固有名詞」(ことえり) のように固有名詞が途中に来る名前もある
    if pos.starts_with("固有") || pos.contains("固有名詞") {
        return Some("固有名詞");
    }
    if pos.contains("名詞") {
        return Some("名詞");
    }
    None
}

/// よみのカタカナをひらがなへ正規化する (変換時の読みはひらがなのため)
pub fn to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| {
            // カタカナ (ァ U+30A1 〜 ヶ U+30F6) はひらがなと 0x60 差で並んでいる
            if ('ァ'..='ヶ').contains(&c) {
                char::from_u32(c as u32 - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// QuicklIME の TSV (userdict.tsv・imported\*.tsv) から既存の (読み, 表記) を集める。
/// 取り込み時の重複判定に使う
pub fn collect_pairs(text: &str, pairs: &mut HashSet<(String, String)>) {
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut fields = line.trim_end_matches('\r').split('\t');
        if let (Some(reading), Some(surface)) = (fields.next(), fields.next()) {
            if !reading.is_empty() && !surface.is_empty() {
                pairs.insert((reading.to_string(), surface.to_string()));
            }
        }
    }
}

/// デコード済みテキストをパースし、取り込む語と件数を返す。
/// existing は userdict.tsv・他のインポート済みファイルにある (読み, 表記)
pub fn parse(text: &str, existing: &HashSet<(String, String)>) -> ImportResult {
    let format = detect_format(text);
    let mut result = ImportResult::default();
    let mut seen: HashSet<(String, String, &'static str)> = HashSet::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{FEFF}').trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let is_header = match format {
            Format::MsIme | Format::Atok => line.starts_with('!'),
            Format::Mozc => line.starts_with('#'),
        };
        if is_header {
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(reading), Some(surface), Some(pos)) = (fields.next(), fields.next(), fields.next())
        else {
            result.invalid += 1;
            continue;
        };
        let reading = to_hiragana(reading);
        if reading.is_empty() || surface.is_empty() {
            result.invalid += 1;
            continue;
        }
        let Some(pos) = map_pos(pos) else {
            result.unsupported_pos += 1;
            continue;
        };
        let pair = (reading, surface.to_string());
        if existing.contains(&pair) {
            result.duplicate += 1;
            continue;
        }
        if !seen.insert((pair.0.clone(), pair.1.clone(), pos)) {
            result.duplicate += 1;
            continue;
        }
        result.entries.push((pair.0, pair.1, pos));
    }
    result
}

/// 取り込む語を保存用の TSV (読み\t表記\t品詞、改行区切り) にする
pub fn to_tsv(entries: &[(String, String, &'static str)]) -> String {
    let mut out = String::new();
    for (reading, surface, pos) in entries {
        out.push_str(reading);
        out.push('\t');
        out.push_str(surface);
        out.push('\t');
        out.push_str(pos);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_cp932(_: &[u8]) -> Option<String> {
        None
    }

    fn utf16le_with_bom(s: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in s.encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn utf16leのbomを判定してデコードする() {
        let bytes = utf16le_with_bom("あい\t愛\t名詞\r\n");
        assert_eq!(decode(&bytes, no_cp932).unwrap(), "あい\t愛\t名詞\r\n");
    }

    #[test]
    fn utf16beのbomを判定してデコードする() {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in "あい".encode_utf16() {
            bytes.extend(unit.to_be_bytes());
        }
        assert_eq!(decode(&bytes, no_cp932).unwrap(), "あい");
    }

    #[test]
    fn utf8のbomは除いてデコードする() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend("あい".as_bytes());
        assert_eq!(decode(&bytes, no_cp932).unwrap(), "あい");
        // BOM なしの UTF-8 もそのまま
        assert_eq!(decode("あい".as_bytes(), no_cp932).unwrap(), "あい");
    }

    #[test]
    fn utf8として不正ならcp932のデコードに回す() {
        // 「あ」の Shift_JIS (0x82 0xA0) は UTF-8 として不正
        let sjis = [0x82u8, 0xA0];
        let decoded = decode(&sjis, |b| {
            assert_eq!(b, &sjis);
            Some("あ".to_string())
        });
        assert_eq!(decoded.unwrap(), "あ");
        // CP932 でも読めなければエラー
        assert!(decode(&sjis, no_cp932).is_err());
    }

    #[test]
    fn 奇数バイトのutf16はエラー() {
        assert!(decode(&[0xFF, 0xFE, 0x42], no_cp932).is_err());
    }

    #[test]
    fn 先頭行で形式を判別する() {
        assert_eq!(detect_format("!Microsoft IME Dictionary Tool\n!Version:\n"), Format::MsIme);
        assert_eq!(detect_format("!!ATOK_TANGO_TEXT_HEADER_1\n"), Format::Atok);
        assert_eq!(detect_format("# Mozc\nあい\t愛\t名詞\n"), Format::Mozc);
        assert_eq!(detect_format(""), Format::Mozc);
    }

    #[test]
    fn 品詞の完全一致と規則で対応づける() {
        assert_eq!(map_pos("名詞"), Some("名詞"));
        assert_eq!(map_pos("固有人姓"), Some("姓"));
        assert_eq!(map_pos("固有人名"), Some("名"));
        assert_eq!(map_pos("固有人他"), Some("人名"));
        assert_eq!(map_pos("固有地名"), Some("地名"));
        assert_eq!(map_pos("地名その他"), Some("地名"));
        assert_eq!(map_pos("固有組織"), Some("組織"));
        assert_eq!(map_pos("固有一般"), Some("固有名詞"));
        assert_eq!(map_pos("固有名詞"), Some("固有名詞"));
        assert_eq!(map_pos("顔文字"), Some("短縮よみ"));
        assert_eq!(map_pos("短縮読み"), Some("短縮よみ"));
        // 規則: 「固有」始まり・固有名詞を含む → 固有名詞、「名詞」を含む → 名詞
        assert_eq!(map_pos("固有その他"), Some("固有名詞"));
        assert_eq!(map_pos("その他の固有名詞"), Some("固有名詞"));
        assert_eq!(map_pos("さ変名詞"), Some("名詞"));
        assert_eq!(map_pos("名詞サ変"), Some("名詞"));
        assert_eq!(map_pos("形動名詞"), Some("名詞"));
        assert_eq!(map_pos("副詞的名詞"), Some("名詞"));
        // 対応外
        assert_eq!(map_pos("動詞一段"), None);
        assert_eq!(map_pos("形容詞"), None);
        assert_eq!(map_pos("副詞"), None);
        assert_eq!(map_pos("数"), None);
        assert_eq!(map_pos("サジェストのみ"), None);
    }

    #[test]
    fn atokの品詞末尾の記号は除いて判定する() {
        assert_eq!(map_pos("名詞*"), Some("名詞"));
        assert_eq!(map_pos("固有人姓$"), Some("姓"));
        assert_eq!(map_pos("固有人名*$"), Some("名"));
        assert_eq!(map_pos("動詞*"), None);
    }

    #[test]
    fn カタカナの読みをひらがなにする() {
        assert_eq!(to_hiragana("カンベヴ"), "かんべゔ");
        assert_eq!(to_hiragana("ABCー"), "ABCー");
    }

    #[test]
    fn msimeの形式を取り込む() {
        let text = "!Microsoft IME Dictionary Tool\r\n\
                    !Version:\r\n\
                    !Format:WORDLIST\r\n\
                    \r\n\
                    かんべ\t神戸\t姓\r\n\
                    ニコニコ\tニコニコ大百科\t固有名詞\t\r\n\
                    たべる\t食べる\t一段動詞\r\n\
                    かおもじ\t(^^)\t顔文字\r\n";
        let result = parse(text, &HashSet::new());
        assert_eq!(
            result.entries,
            vec![
                ("かんべ".to_string(), "神戸".to_string(), "姓"),
                ("にこにこ".to_string(), "ニコニコ大百科".to_string(), "固有名詞"),
                ("かおもじ".to_string(), "(^^)".to_string(), "短縮よみ"),
            ]
        );
        assert_eq!(result.unsupported_pos, 1);
        assert_eq!(result.invalid, 0);
    }

    #[test]
    fn atokの形式を取り込む() {
        let text = "!!ATOK_TANGO_TEXT_HEADER_1\n\
                    !!一覧出力\n\
                    かんべ\t神戸\t固有人姓*\n\
                    ぴくしぶ\tpixiv\t固有商品\n";
        let result = parse(text, &HashSet::new());
        assert_eq!(
            result.entries,
            vec![
                ("かんべ".to_string(), "神戸".to_string(), "姓"),
                ("ぴくしぶ".to_string(), "pixiv".to_string(), "固有名詞"),
            ]
        );
    }

    #[test]
    fn mozcの形式を取り込みコメントは飛ばす() {
        let text = "# コメント\n\
                    !びっくり\t!始まりの表記\t名詞\n\
                    めーる\tmail@example.com\t短縮よみ\tコメント列\n";
        let result = parse(text, &HashSet::new());
        // Mozc 形式では ! 始まりは通常の行
        assert_eq!(
            result.entries,
            vec![
                ("!びっくり".to_string(), "!始まりの表記".to_string(), "名詞"),
                ("めーる".to_string(), "mail@example.com".to_string(), "短縮よみ"),
            ]
        );
    }

    #[test]
    fn 列不足と空の読み表記は不正な行として数える() {
        let text = "よみだけ\n\
                    よみ\t表記\n\
                    \t表記\t名詞\n\
                    よみ\t\t名詞\n\
                    よみ\t表記\t名詞\n";
        let result = parse(text, &HashSet::new());
        assert_eq!(result.invalid, 4);
        assert_eq!(result.entries.len(), 1);
    }

    #[test]
    fn ファイル内と既存の重複を除く() {
        let text = "かんべ\t神戸\t姓\n\
                    カンベ\t神戸\t固有人姓\n\
                    かんべ\t神戸\t地名\n\
                    てすと\tテスト\t名詞\n\
                    めーる\tmail@example.com\t短縮よみ\n";
        let mut existing = HashSet::new();
        collect_pairs(
            "# 手動登録\nてすと\tテスト\t固有名詞\nめーる\tmail@example.com\n",
            &mut existing,
        );
        let result = parse(text, &existing);
        // 2行目は読みの正規化後に1行目と同じ。3行目は品詞が違うので別の語
        assert_eq!(
            result.entries,
            vec![
                ("かんべ".to_string(), "神戸".to_string(), "姓"),
                ("かんべ".to_string(), "神戸".to_string(), "地名"),
            ]
        );
        assert_eq!(result.duplicate, 3);
    }

    #[test]
    fn 取り込む語をtsvにする() {
        let entries = vec![
            ("かんべ".to_string(), "神戸".to_string(), "姓"),
            ("めーる".to_string(), "mail@example.com".to_string(), "短縮よみ"),
        ];
        assert_eq!(to_tsv(&entries), "かんべ\t神戸\t姓\nめーる\tmail@example.com\t短縮よみ\n");
    }
}
