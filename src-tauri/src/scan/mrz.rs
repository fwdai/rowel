//! ICAO 9303 machine-readable zone parser: TD3 (2x44, passports), TD2 (2x36)
//! and TD1 (3x30, identity cards and residence permits).
//!
//! Pure text in, fields out — the OCR backend hands over lines and never gets
//! consulted again. Check digits are what make the format self-verifying, so a
//! document is only reported when the arithmetic agrees; that is also what lets
//! `scan_lines` treat an MRZ hit as unambiguous.

use std::collections::BTreeMap;

use chrono::Datelike;

// The 7-3-1 weights of every ICAO check digit, applied cyclically.
const WEIGHTS: [u32; 3] = [7, 3, 1];

/// How many fillers a line may be short of its width and still be read. OCR
/// drops the tail of a long filler run — the end of a name line, or of a
/// TD1's optional data. Only a row that still ends in filler is padded: a name
/// long enough to reach the edge has no check digit to catch its last letters
/// going missing, and a data line that lost its check digits is caught by the
/// arithmetic.
const MISSING_FILLER: usize = 3;

/// Fillers a document can carry only in its MRZ. Fewer than this on one line
/// is a name written with fillers' worth of spaces, or a misread.
const FILLER_RUN: usize = 5;

/// Read the first document the lines describe, or `None` if none of them do.
pub fn parse(lines: &[String]) -> Option<BTreeMap<String, String>> {
    let rows: Vec<String> = lines
        .iter()
        .filter_map(|l| normalize(l))
        .filter(|l| !l.is_empty())
        .collect();

    // The three formats have disjoint line lengths, tolerance included, so the
    // width of a run of candidate rows already says which one it can be.
    for w in rows.windows(2) {
        if let Some(l) = fit_all(w, 44) {
            if let Some(fields) = td3(&l[0], &l[1]) {
                return Some(fields);
            }
        }
        if let Some(l) = fit_all(w, 36) {
            if let Some(fields) = td2(&l[0], &l[1]) {
                return Some(fields);
            }
        }
    }
    for w in rows.windows(3) {
        if let Some(l) = fit_all(w, 30) {
            if let Some(fields) = td1(&l[0], &l[1], &l[2]) {
                return Some(fields);
            }
        }
    }
    None
}

/// Whether the lines hold what only an MRZ prints: a run of filler. `parse`
/// asks for more — the check digits — so this is what tells an MRZ the OCR
/// mangled from no MRZ at all. The difference matters to the caller: the
/// first wants a better photo, and must not be read as something else.
pub fn present(lines: &[String]) -> bool {
    lines
        .iter()
        .any(|l| l.chars().filter(|&c| is_filler(c)).count() >= FILLER_RUN)
}

/// An MRZ line reduced to the ICAO alphabet, or `None` when the line holds
/// something that cannot be one. Spaces are dropped (OCR breaks up filler runs)
/// and a guillemet is taken for the filler it was misread from.
fn normalize(line: &str) -> Option<String> {
    let mut out = String::new();
    for c in line.trim().to_uppercase().chars() {
        match c {
            c if c.is_whitespace() => continue,
            c if is_filler(c) => out.push('<'),
            'A'..='Z' | '0'..='9' => out.push(c),
            _ => return None,
        }
    }
    Some(out)
}

fn is_filler(c: char) -> bool {
    matches!(c, '<' | '«' | '‹' | '≪' | '〈')
}

/// `rows` as lines of `width`, or `None` if any of them is not one: too long,
/// too short to be one with its trailing filler back, or short without ending
/// in the filler that would have been dropped.
fn fit_all(rows: &[String], width: usize) -> Option<Vec<String>> {
    rows.iter().map(|row| fit(row, width)).collect()
}

fn fit(row: &str, width: usize) -> Option<String> {
    // Normalized rows are ASCII, so bytes count characters.
    let len = row.len();
    if len == width {
        return Some(row.to_owned());
    }
    if len > width || len + MISSING_FILLER < width || !row.ends_with('<') {
        return None;
    }
    Some(format!("{row}{}", "<".repeat(width - len)))
}

/// The fields of one parsed document, still in MRZ form.
struct Mrz<'a> {
    code: &'a str,
    state: &'a str,
    name: &'a str,
    number: &'a str,
    nationality: &'a str,
    birth: &'a str,
    sex: char,
    expiry: &'a str,
    personal: &'a str,
}

impl Mrz<'_> {
    fn fields(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("doc_type".into(), doc_type(self.code).into()),
            ("name".into(), name(self.name)),
            ("number".into(), trim_filler(self.number)),
            ("country".into(), alpha(self.state)),
            ("nationality".into(), alpha(self.nationality)),
            ("birth_date".into(), iso_date(self.birth, false)),
            ("sex".into(), sex(self.sex).into()),
            ("expiry_date".into(), iso_date(self.expiry, true)),
            ("personal_number".into(), trim_filler(self.personal)),
        ])
    }
}

// Passports (2x44). Line 2 carries every fixed-width field plus the composite
// check digit over all of them.
fn td3(l1: &str, l2: &str) -> Option<BTreeMap<String, String>> {
    let l2 = fix_numeric(l2, &[(9, 10), (13, 20), (21, 28), (42, 44)]);
    let individual = check_ok(&l2[0..9], at(&l2, 9))
        && check_ok(&l2[13..19], at(&l2, 19))
        && check_ok(&l2[21..27], at(&l2, 27))
        && check_ok(&l2[28..42], at(&l2, 42));
    let composite = format!("{}{}{}", &l2[0..10], &l2[13..20], &l2[21..43]);
    if !check_ok(&composite, at(&l2, 43)) && !individual {
        return None;
    }
    Some(
        Mrz {
            code: &l1[0..2],
            state: &l1[2..5],
            name: &l1[5..44],
            number: &l2[0..9],
            nationality: &l2[10..13],
            birth: &l2[13..19],
            sex: at(&l2, 20),
            expiry: &l2[21..27],
            personal: &l2[28..42],
        }
        .fields(),
    )
}

// 2x36: the TD3 layout with a shorter name field and only 7 characters of
// optional data, so the composite runs to position 35 instead of 43.
fn td2(l1: &str, l2: &str) -> Option<BTreeMap<String, String>> {
    let l2 = fix_numeric(l2, &[(9, 10), (13, 20), (21, 28), (35, 36)]);
    let individual = check_ok(&l2[0..9], at(&l2, 9))
        && check_ok(&l2[13..19], at(&l2, 19))
        && check_ok(&l2[21..27], at(&l2, 27));
    let composite = format!("{}{}{}", &l2[0..10], &l2[13..20], &l2[21..35]);
    if !check_ok(&composite, at(&l2, 35)) && !individual {
        return None;
    }
    Some(
        Mrz {
            code: &l1[0..2],
            state: &l1[2..5],
            name: &l1[5..36],
            number: &l2[0..9],
            nationality: &l2[10..13],
            birth: &l2[13..19],
            sex: at(&l2, 20),
            expiry: &l2[21..27],
            personal: &l2[28..35],
        }
        .fields(),
    )
}

// 3x30: the document number moves up to line 1, the dates sit on line 2, and
// the name has a line of its own.
fn td1(l1: &str, l2: &str, l3: &str) -> Option<BTreeMap<String, String>> {
    let l1 = fix_numeric(l1, &[(14, 15)]);
    let l2 = fix_numeric(l2, &[(0, 7), (8, 15), (29, 30)]);
    let individual = check_ok(&l1[5..14], at(&l1, 14))
        && check_ok(&l2[0..6], at(&l2, 6))
        && check_ok(&l2[8..14], at(&l2, 14));
    let composite = format!("{}{}{}{}", &l1[5..30], &l2[0..7], &l2[8..15], &l2[18..29]);
    if !check_ok(&composite, at(&l2, 29)) && !individual {
        return None;
    }
    Some(
        Mrz {
            code: &l1[0..2],
            state: &l1[2..5],
            name: &l3[0..30],
            number: &l1[5..14],
            nationality: &l2[15..18],
            birth: &l2[0..6],
            sex: at(&l2, 7),
            expiry: &l2[8..14],
            // Optional data 2, the field a personal number is written in.
            personal: &l2[18..29],
        }
        .fields(),
    )
}

fn at(line: &str, i: usize) -> char {
    line.as_bytes()[i] as char
}

/// Re-read the positions that can only hold digits. A letter there is an OCR
/// misread, and correcting it before the arithmetic is what makes a noisy scan
/// verify instead of being thrown away.
fn fix_numeric(line: &str, ranges: &[(usize, usize)]) -> String {
    let mut chars: Vec<char> = line.chars().collect();
    for &(start, end) in ranges {
        for c in chars[start..end].iter_mut() {
            *c = digit(*c);
        }
    }
    chars.into_iter().collect()
}

fn digit(c: char) -> char {
    match c {
        'O' | 'Q' | 'D' => '0',
        'I' | 'L' => '1',
        'Z' => '2',
        'S' => '5',
        'G' => '6',
        'B' => '8',
        c => c,
    }
}

fn letter(c: char) -> char {
    match c {
        '0' => 'O',
        '1' => 'I',
        '2' => 'Z',
        '5' => 'S',
        '6' => 'G',
        '8' => 'B',
        c => c,
    }
}

fn value(c: char) -> Option<u32> {
    match c {
        '<' => Some(0),
        '0'..='9' => Some(c as u32 - '0' as u32),
        'A'..='Z' => Some(c as u32 - 'A' as u32 + 10),
        _ => None,
    }
}

fn check_digit(field: &str) -> Option<char> {
    let mut sum = 0;
    for (i, c) in field.chars().enumerate() {
        sum += value(c)? * WEIGHTS[i % 3];
    }
    char::from_digit(sum % 10, 10)
}

/// An unused field carries no check digit worth testing, and issuers write
/// either `0` or the filler in place of one.
fn check_ok(field: &str, digit: char) -> bool {
    if field.chars().all(|c| c == '<') {
        return true;
    }
    match check_digit(field) {
        Some(expected) => expected == digit || (digit == '<' && expected == '0'),
        None => false,
    }
}

// The document code says what was scanned. `IR`/`AR` are the residence-permit
// codes; every other I/A/C code is an ordinary identity card, which is also the
// safe default for a TD1 document.
fn doc_type(code: &str) -> &'static str {
    match code.trim_end_matches('<').as_bytes() {
        [b'P', ..] => "passport",
        [b'I', b'R'] | [b'A', b'R'] => "residence_permit",
        _ => "id_card",
    }
}

/// `SURNAME<<GIVEN<NAMES`, filler-padded. Rendered the way a person writes
/// their own name, given names first.
fn name(field: &str) -> String {
    let fixed: String = field.chars().map(letter).collect();
    let (surname, given) = match fixed.split_once("<<") {
        Some((s, g)) => (words(s), words(g)),
        None => (words(&fixed), String::new()),
    };
    match (surname.is_empty(), given.is_empty()) {
        (true, _) => given,
        (_, true) => surname,
        _ => format!("{given} {surname}"),
    }
}

fn words(field: &str) -> String {
    field
        .split('<')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn alpha(field: &str) -> String {
    trim_filler(&field.chars().map(letter).collect::<String>())
}

fn trim_filler(field: &str) -> String {
    field.replace('<', " ").trim().to_owned()
}

fn sex(c: char) -> &'static str {
    match c {
        'M' => "M",
        'F' => "F",
        // `<` (and the occasional `X`) means unspecified.
        _ => "",
    }
}

/// `YYMMDD` as an ISO date, or empty when the six digits are not a date. An
/// expiry is always in this century; a birth year later than the current one
/// has to belong to the last.
fn iso_date(yymmdd: &str, expiry: bool) -> String {
    let digits: Vec<u32> = yymmdd.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() != 6 {
        return String::new();
    }
    let (yy, mm, dd) = (
        digits[0] * 10 + digits[1],
        digits[2] * 10 + digits[3],
        digits[4] * 10 + digits[5],
    );
    if !(1..=12).contains(&mm) || !(1..=31).contains(&dd) {
        return String::new();
    }
    let this_century = expiry || yy <= (chrono::Utc::now().year() % 100) as u32;
    let year = if this_century { 2000 + yy } else { 1900 + yy };
    format!("{year:04}-{mm:02}-{dd:02}")
}
