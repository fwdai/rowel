//! Payment-card fields from OCR text lines.
//!
//! Nothing on a card is labelled, so each field is found by what only it can
//! look like: the number is the digit run that passes Luhn, the expiry is the
//! `MM/YY` box, and the holder is the line of plain uppercase words left over.
//! Best effort by design — a wrong guess costs the user one edit, a missing
//! number costs them the whole scan, so the number is the only required field.

use std::collections::BTreeMap;

use super::words::is_name_word;

/// Embossed digits are misread as these letters and no others.
fn as_digit(c: char) -> Option<char> {
    match c {
        '0'..='9' => Some(c),
        'O' | 'o' => Some('0'),
        'I' | 'i' | 'l' | '|' => Some('1'),
        'S' | 's' => Some('5'),
        'B' | 'b' => Some('8'),
        _ => None,
    }
}

/// How many characters of a candidate number may be corrected letters. Without
/// a cap, a line of all-confusable words ("SOBOLS BOSS") reads as a digit run.
const MAX_FIXES: usize = 4;

/// Words that are printed on cards but are never the holder's name.
const NOT_A_NAME: &[&str] = &[
    "VISA",
    "MASTERCARD",
    "MAESTRO",
    "AMEX",
    "AMERICAN",
    "EXPRESS",
    "DISCOVER",
    "JCB",
    "UNIONPAY",
    "DINERS",
    "CLUB",
    "ELECTRON",
    "DEBIT",
    "CREDIT",
    "PREPAID",
    "BANK",
    "BANCO",
    "BANQUE",
    "VALID",
    "THRU",
    "FROM",
    "GOOD",
    "ONLY",
    "EXPIRES",
    "EXPIRY",
    "MEMBER",
    "SINCE",
    "CARD",
    "CARDHOLDER",
    "HOLDER",
    "PLATINUM",
    "GOLD",
    "SILVER",
    "CLASSIC",
    "BUSINESS",
    "WORLD",
    "SIGNATURE",
    "INFINITE",
    "REWARDS",
    "CVV",
    "CVC",
    "CUSTOMER",
    "SERVICE",
];

/// The card the lines describe, or `None` when no Luhn-valid number is there.
pub fn parse(lines: &[String]) -> Option<BTreeMap<String, String>> {
    let number = number(lines)?;
    let (month, year) = expiry(lines).unwrap_or_default();
    Some(BTreeMap::from([
        ("number".into(), number),
        ("month".into(), month),
        ("year".into(), year),
        ("name".into(), cardholder(lines).unwrap_or_default()),
    ]))
}

/// The best Luhn-valid run of 13-19 digits. A run can be longer than a card
/// number (OCR joins the number with whatever sits beside it), so every window
/// of it is tried; a recognised network breaks a tie, then length.
///
/// A window shorter than its run has to belong to a known network, though.
/// Luhn alone is a one-in-ten filter, so a long run of digits reliably contains
/// some shorter window that passes it — `4111 1111 1111 1112` ends in a
/// Luhn-valid 13-digit tail — and accepting those would turn every mistyped
/// number into a different, wrong card.
fn number(lines: &[String]) -> Option<String> {
    let mut best: Option<(bool, String)> = None;
    for line in lines {
        for run in digit_runs(line) {
            for len in 13..=19usize {
                if run.len() < len {
                    break;
                }
                for start in 0..=run.len() - len {
                    let window = &run[start..start + len];
                    if window.iter().filter(|(_, fixed)| *fixed).count() > MAX_FIXES {
                        continue;
                    }
                    let digits: String = window.iter().map(|(c, _)| *c).collect();
                    if !luhn(&digits) {
                        continue;
                    }
                    let branded = crate::cards::card_brand(&digits).is_some();
                    if !branded && len < run.len() {
                        continue;
                    }
                    let better = match &best {
                        Some((b, d)) => (branded, digits.len()) > (*b, d.len()),
                        None => true,
                    };
                    if better {
                        best = Some((branded, digits));
                    }
                }
            }
        }
    }
    best.map(|(_, digits)| digits)
}

/// Runs of digits (and the letters they may have been misread from), with the
/// group separators printed between quads swallowed. Each character is paired
/// with whether it had to be corrected.
fn digit_runs(line: &str) -> Vec<Vec<(char, bool)>> {
    let mut runs = Vec::new();
    let mut run: Vec<(char, bool)> = Vec::new();
    for c in line.chars() {
        match as_digit(c) {
            Some(d) => run.push((d, !c.is_ascii_digit())),
            None if matches!(c, ' ' | '-' | '\u{2013}' | '\u{2014}' | '\t') => {}
            None if !run.is_empty() => runs.push(std::mem::take(&mut run)),
            None => {}
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0;
    for (i, c) in digits.chars().rev().enumerate() {
        let mut d = c.to_digit(10).unwrap_or(0);
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum % 10 == 0
}

/// `(month, year)` as the app stores them: two digits each. A card can print a
/// "valid from" date above the expiry, so the latest date on it wins.
///
/// Both ends of the match must be free of digits, which is what keeps a quad of
/// the card number itself ("1111 1111") from reading as a date.
fn expiry(lines: &[String]) -> Option<(String, String)> {
    let mut best: Option<(u32, u32)> = None;
    for line in lines {
        let b = line.as_bytes();
        for i in 0..b.len().saturating_sub(3) {
            if i > 0 && b[i - 1].is_ascii_digit() {
                continue;
            }
            if !(b[i].is_ascii_digit() && b[i + 1].is_ascii_digit()) {
                continue;
            }
            let month = u32::from(b[i] - b'0') * 10 + u32::from(b[i + 1] - b'0');
            if !(1..=12).contains(&month) || !matches!(b[i + 2], b'/' | b'-' | b' ') {
                continue;
            }
            // A year is written with two digits or four, never three.
            let rest = &b[i + 3..];
            let seen = rest.iter().take_while(|c| c.is_ascii_digit()).count();
            let year = match seen {
                2 => 2000 + number_of(&rest[..2]),
                4 => number_of(&rest[..4]),
                _ => continue,
            };
            let later = match best {
                Some(best) => best < (year, month),
                None => true,
            };
            if later {
                best = Some((year, month));
            }
        }
    }
    best.map(|(year, month)| (format!("{month:02}"), format!("{:02}", year % 100)))
}

fn number_of(digits: &[u8]) -> u32 {
    digits.iter().fold(0, |n, c| n * 10 + u32::from(c - b'0'))
}

/// The longest line that reads like a person's name: uppercase words, no
/// digits, and nothing the card issuer printed there itself.
fn cardholder(lines: &[String]) -> Option<String> {
    let mut best: Option<&str> = None;
    for line in lines {
        let line = line.trim();
        if line.chars().any(|c| c.is_numeric()) || line != line.to_uppercase() {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        if !words.iter().all(|w| is_name_word(w)) {
            continue;
        }
        // Two real words: a name, unlike a single embossed label.
        if words.iter().filter(|w| w.len() >= 2).count() < 2 {
            continue;
        }
        if words.iter().any(|w| NOT_A_NAME.contains(&bare(w).as_str())) {
            continue;
        }
        let better = match best {
            Some(b) => b.len() < line.len(),
            None => true,
        };
        if better {
            best = Some(line);
        }
    }
    best.map(str::to_owned)
}

fn bare(word: &str) -> String {
    word.chars().filter(char::is_ascii_alphabetic).collect()
}
