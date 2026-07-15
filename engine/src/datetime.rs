// 日付・時刻の動的候補
//
// 「きょう」「あした」「いま」などの読みに対して、現在日時から作った
// 表記 (2026/07/15、14時30分 など) を候補として返す。
// 候補は変換のたびに現在日時で生成されて古くなるため、学習の対象にしない
// (main.rs の LEARN 処理で除外する)。

use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, Timelike};

/// 日時に基づく動的候補を返す。対象外の読みなら空
pub fn candidates_at(reading: &str, now: NaiveDateTime) -> Vec<String> {
    let date = now.date();
    match reading {
        "きょう" => date_candidates(Some(date)),
        "あした" | "あす" => date_candidates(date.checked_add_days(Days::new(1))),
        "あさって" => date_candidates(date.checked_add_days(Days::new(2))),
        "きのう" => date_candidates(date.checked_sub_days(Days::new(1))),
        "おととい" => date_candidates(date.checked_sub_days(Days::new(2))),
        "いま" => time_candidates(now),
        "ことし" => year_candidates(date.year()),
        "らいねん" => year_candidates(date.year() + 1),
        "きょねん" => year_candidates(date.year() - 1),
        _ => Vec::new(),
    }
}

/// 日付の表記いろいろ (2026/07/15、2026年7月15日、7月15日(水)、令和8年7月15日)
fn date_candidates(date: Option<NaiveDate>) -> Vec<String> {
    let Some(d) = date else {
        return Vec::new(); // 日付計算があふれた場合 (実用上は起きない)
    };
    let weekday = ["月", "火", "水", "木", "金", "土", "日"]
        [d.weekday().num_days_from_monday() as usize];
    let mut result = vec![
        format!("{}/{:02}/{:02}", d.year(), d.month(), d.day()),
        format!("{}年{}月{}日", d.year(), d.month(), d.day()),
        format!("{}月{}日({})", d.month(), d.day(), weekday),
    ];
    if let Some(wareki) = wareki_year(d.year()) {
        result.push(format!("{}年{}月{}日", wareki, d.month(), d.day()));
    }
    result
}

/// 時刻の表記いろいろ (14:30、14時30分、午後2時30分)
fn time_candidates(now: NaiveDateTime) -> Vec<String> {
    let (hour, minute) = (now.hour(), now.minute());
    let half = if hour < 12 { "午前" } else { "午後" };
    vec![
        format!("{}:{:02}", hour, minute),
        format!("{}時{}分", hour, minute),
        format!("{}{}時{}分", half, hour % 12, minute),
    ]
}

/// 年の表記 (2026年、令和8年)
fn year_candidates(year: i32) -> Vec<String> {
    let mut result = vec![format!("{}年", year)];
    if let Some(wareki) = wareki_year(year) {
        result.push(format!("{}年", wareki));
    }
    result
}

/// 西暦年の和暦表記 (「令和8」「平成30」)。平成より前は対応しない
fn wareki_year(year: i32) -> Option<String> {
    let (era, first_year) = match year {
        2019.. => ("令和", 2019),
        1989..=2018 => ("平成", 1989),
        _ => return None,
    };
    let n = year - first_year + 1;
    if n == 1 {
        Some(format!("{era}元"))
    } else {
        Some(format!("{era}{n}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    #[test]
    fn きょうは当日の日付になる() {
        let got = candidates_at("きょう", at(2026, 7, 15, 14, 30));
        assert_eq!(got, vec!["2026/07/15", "2026年7月15日", "7月15日(水)", "令和8年7月15日"]);
    }

    #[test]
    fn あしたとあさっては日付が進む() {
        assert_eq!(candidates_at("あした", at(2026, 7, 15, 0, 0))[0], "2026/07/16");
        assert_eq!(candidates_at("あす", at(2026, 7, 15, 0, 0))[0], "2026/07/16");
        assert_eq!(candidates_at("あさって", at(2026, 7, 15, 0, 0))[0], "2026/07/17");
    }

    #[test]
    fn きのうとおとといは日付が戻る() {
        // 月またぎも正しく計算される
        assert_eq!(candidates_at("きのう", at(2026, 7, 1, 0, 0))[0], "2026/06/30");
        assert_eq!(candidates_at("おととい", at(2026, 7, 1, 0, 0))[0], "2026/06/29");
    }

    #[test]
    fn いまは現在時刻になる() {
        assert_eq!(
            candidates_at("いま", at(2026, 7, 15, 14, 5)),
            vec!["14:05", "14時5分", "午後2時5分"]
        );
        // 午前と正午またぎ
        assert_eq!(candidates_at("いま", at(2026, 7, 15, 9, 30))[2], "午前9時30分");
        assert_eq!(candidates_at("いま", at(2026, 7, 15, 12, 0))[2], "午後0時0分");
    }

    #[test]
    fn 年の読みは西暦と和暦になる() {
        assert_eq!(candidates_at("ことし", at(2026, 7, 15, 0, 0)), vec!["2026年", "令和8年"]);
        assert_eq!(candidates_at("らいねん", at(2026, 7, 15, 0, 0))[0], "2027年");
        assert_eq!(candidates_at("きょねん", at(2026, 7, 15, 0, 0))[0], "2025年");
    }

    #[test]
    fn 和暦は元年と平成に対応する() {
        assert_eq!(wareki_year(2019), Some("令和元".to_string()));
        assert_eq!(wareki_year(2018), Some("平成30".to_string()));
        assert_eq!(wareki_year(1989), Some("平成元".to_string()));
        assert_eq!(wareki_year(1988), None);
    }

    #[test]
    fn 対象外の読みは空を返す() {
        assert!(candidates_at("きょうは", at(2026, 7, 15, 0, 0)).is_empty());
        assert!(candidates_at("はれ", at(2026, 7, 15, 0, 0)).is_empty());
    }
}
