//! The captions a document prints beside its data, and where each one's value
//! would stand.
//!
//! A document labels what it prints — "Surname", "Date of expiry", "Gültig
//! bis" — and a driving licence does it by number, the same numbers wherever
//! it is issued (ISO/IEC 18013: 1 the surname, 2 the given names, 3 the birth
//! date, 4a the issue date, 4b the expiry). Both are found here. What the
//! value beside a caption *is* belongs to the reader (`super::fields`): a
//! caption only says what kind of thing to look for and where.

use super::dates;
use crate::scan::words::{is_name_word, words, Word};

/// What a caption captions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Field {
    Surname,
    Given,
    /// The whole name under one caption.
    Name,
    Number,
    Birth,
    Issue,
    Expiry,
    Sex,
    /// A caption for something no entry field takes. It is known so that it
    /// ends the value before it: a name does not run on into "Nationality".
    Other,
}

/// A caption in the text: the line it is on, what it captions, and the
/// stretch of that line its value would stand in — from the caption's end to
/// the next caption's start, or the line's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Caption {
    pub line: usize,
    pub field: Field,
    pub start: usize,
    pub end: usize,
}

/// The captions documents print, as the keys `words` reduces them to. Long
/// before short within a field is not needed: the longest match at a place
/// wins whatever the order here.
const CAPTIONS: &[(Field, &[&str])] = &[
    (
        Field::Surname,
        &[
            "SURNAME",
            "FAMILY NAME",
            "LAST NAME",
            "NOM",
            "NOM DE FAMILLE",
            "NACHNAME",
            "FAMILIENNAME",
            "APELLIDOS",
            "APELLIDO",
            "COGNOME",
            "APELIDO",
            "APELIDOS",
            "ACHTERNAAM",
            "NAZWISKO",
            "EFTERNAMN",
            "ETTERNAVN",
            "EFTERNAVN",
            "SUKUNIMI",
            "PRIJMENI",
            "PRIEZVISKO",
            "SOYADI",
        ],
    ),
    (
        Field::Given,
        &[
            "GIVEN NAMES",
            "GIVEN NAME",
            "FIRST NAME",
            "FIRST NAMES",
            "FORENAMES",
            "FORENAME",
            "PRENOMS",
            "PRENOM",
            "VORNAMEN",
            "VORNAME",
            "NOMBRES",
            "NOMBRE",
            "NOME",
            "VOORNAMEN",
            "IMIONA",
            "IMIE",
            "FORNAMN",
            "FORNAVN",
            "ETUNIMET",
            "JMENO",
        ],
    ),
    (
        Field::Name,
        &["NAME", "FULL NAME", "NAAM", "NAMN", "NAVN", "NIMI"],
    ),
    (
        Field::Number,
        &[
            "DOCUMENT NO",
            "DOCUMENT NUMBER",
            "PASSPORT NO",
            "PASSPORT NUMBER",
            "LICENCE NO",
            "LICENSE NO",
            "LICENCE NUMBER",
            "LICENSE NUMBER",
            "CARD NO",
            "CARD NUMBER",
            "ID NO",
            "ID NUMBER",
            "DLN",
            "DL",
            "DL NO",
            "NO DU DOCUMENT",
            "NUMERO DU DOCUMENT",
            "NO DU PASSEPORT",
            "AUSWEISNUMMER",
            "DOKUMENTENNUMMER",
            "PASS NR",
            "NUMERO DE DOCUMENTO",
            "NUMERO DOCUMENTO",
            "NUMERO DE PASAPORTE",
            "DOCUMENTNUMMER",
            "NR DOKUMENTU",
            "NUMER DOKUMENTU",
        ],
    ),
    (
        Field::Birth,
        &[
            "DATE OF BIRTH",
            "BIRTH DATE",
            "DOB",
            "DATE DE NAISSANCE",
            "NE LE",
            "NEE LE",
            "GEBURTSDATUM",
            "GEBURTSTAG",
            "GEBOREN AM",
            "FECHA DE NACIMIENTO",
            "DATA DI NASCITA",
            "DATA DE NASCIMENTO",
            "GEBOORTEDATUM",
            "DATA URODZENIA",
            "FODELSEDATUM",
            "FODSELSDATO",
            "SYNTYMAAIKA",
            "DATUM NAROZENI",
            "DOGUM TARIHI",
        ],
    ),
    (
        Field::Issue,
        &[
            "DATE OF ISSUE",
            "ISSUE DATE",
            "ISSUED",
            "ISSUED ON",
            "ISS",
            "DATE DE DELIVRANCE",
            "DELIVRE LE",
            "AUSSTELLUNGSDATUM",
            "AUSGESTELLT AM",
            "FECHA DE EXPEDICION",
            "DATA DI RILASCIO",
            "DATA DE EMISSAO",
            "DATUM VAN AFGIFTE",
            "DATA WYDANIA",
            "UTFARDAD",
            "UTSTEDT",
            "DATUM VYDANI",
        ],
    ),
    (
        Field::Expiry,
        &[
            "DATE OF EXPIRY",
            "EXPIRY DATE",
            "EXPIRATION DATE",
            "DATE OF EXPIRATION",
            "EXPIRES",
            "EXPIRY",
            "EXP",
            "VALID UNTIL",
            "VALID THRU",
            "VALID TO",
            "DATE DEXPIRATION",
            "DATE D EXPIRATION",
            "VALABLE JUSQUAU",
            "GULTIG BIS",
            "ABLAUFDATUM",
            "FECHA DE CADUCIDAD",
            "VALIDO HASTA",
            "DATA DI SCADENZA",
            "SCADENZA",
            "DATA DE VALIDADE",
            "VALIDADE",
            "GELDIG TOT",
            "TERMIN WAZNOSCI",
            "DATA WAZNOSCI",
            "GILTIG TILL",
            "GYLDIG TIL",
            "PLATNOST DO",
            "GECERLILIK TARIHI",
        ],
    ),
    (
        Field::Sex,
        &[
            "SEX",
            "SEXE",
            "GENDER",
            "GESCHLECHT",
            "SEXO",
            "SESSO",
            "GESLACHT",
            "PLEC",
            "KJONN",
        ],
    ),
    (
        Field::Other,
        &[
            "ADDRESS",
            "ADRESSE",
            "NATIONALITY",
            "NATIONALITE",
            "STAATSANGEHORIGKEIT",
            "NACIONALIDAD",
            "PLACE OF BIRTH",
            "COUNTRY OF BIRTH",
            "LIEU DE NAISSANCE",
            "GEBURTSORT",
            "AUTHORITY",
            "ISSUING AUTHORITY",
            "AUTORITE",
            "BEHORDE",
            "HEIGHT",
            "HGT",
            "WEIGHT",
            "WGT",
            "EYES",
            "HAIR",
            "CLASS",
            "ENDORSEMENTS",
            "RESTRICTIONS",
            "SIGNATURE",
            "UNTERSCHRIFT",
            "CATEGORY",
            "PERSONAL NO",
            "REMARKS",
            "SONSTIGE ANGABEN",
            "ANMERKUNGEN",
            "DD",
        ],
    ),
];

// How much a recognizer may get wrong in a caption and still have read it.
// Only the long ones are forgiven anything: a short caption is one slip away
// from a name ("GENDER" from "BENDER"), and exact or not at all.
const ONE_SLIP_FROM: usize = 8;
const TWO_SLIPS_FROM: usize = 12;

/// Every caption in the lines, top to bottom and left to right. `numbered`
/// also takes a driving licence's field numbers for captions, which is asked
/// only of a licence: anywhere else a line that opens with "2" is a line that
/// opens with a number.
pub(super) fn find(lines: &[String], numbered: bool) -> Vec<Caption> {
    let mut found = Vec::new();
    for (line, text) in lines.iter().enumerate() {
        let words = words(text);
        // Each caption as the words it spans, first and one past the last.
        let mut spans: Vec<(usize, usize, Field)> = Vec::new();
        // The line's dates, which a field number is told from — asked only
        // where field numbers are.
        let dated = numbered.then(|| dates::find(text));
        let abbreviated = words.first().and_then(|word| abbreviated(text, word));
        let mut at = usize::from(abbreviated.is_some());
        while at < words.len() {
            let number = dated
                .as_deref()
                .and_then(|dated| field_number(text, &words, at, dated));
            let caption = match number {
                // "4d DLN", "1. Family name": the number and the words are
                // one caption, and the words say what it is — a licence's 5
                // is its number in Europe and a discriminator ("5 DD") in
                // America.
                Some((_, Some((len, field)))) => Some((len + 1, field)),
                Some((number, None)) => numbered_field(number).map(|field| (1, field)),
                None => worded(&words[at..]),
            };
            let Some((len, field)) = caption else {
                at += 1;
                continue;
            };
            match spans.last_mut() {
                // "Surname / Nom": the caption again, in the document's other
                // language.
                Some(last) if last.1 == at && last.2 == field => last.1 = at + len,
                _ => spans.push((at, at + len, field)),
            }
            at += len;
        }
        if let Some(field) = abbreviated {
            found.push(Caption {
                line,
                field,
                start: words[0].start + ABBREVIATION,
                end: spans.first().map_or(text.len(), |next| words[next.0].start),
            });
        }
        for (index, (_, past, field)) in spans.iter().enumerate() {
            found.push(Caption {
                line,
                field: *field,
                start: words[past - 1].end,
                end: spans
                    .get(index + 1)
                    .map_or(text.len(), |next| words[next.0].start),
            });
        }
    }
    found
}

// The length of a name caption a licence abbreviates: "LN", "FN".
const ABBREVIATION: usize = 2;

/// The abbreviated name caption a line opens with, where it opens with one:
/// "LN SAMPLE", or run into the name it captions, "LNCARDHOLDER" — a licence
/// sets the two so close that a recognizer reads them as one word.
///
/// Only at the head of a line, which is where a licence prints them: further
/// along, the same letters end an address ("123 MAIN LN"). And only in
/// capitals, before nothing or before a name in capitals — no word of its own
/// opens with either pair of letters.
fn abbreviated(text: &str, word: &Word) -> Option<Field> {
    let printed = &text[word.start..word.end];
    let field = match printed.get(..ABBREVIATION)? {
        "LN" => Field::Surname,
        "FN" => Field::Given,
        _ => return None,
    };
    let name = &printed[ABBREVIATION..];
    let run_in = is_name_word(name)
        && !name.chars().any(char::is_lowercase)
        && name.chars().filter(|c| c.is_alphabetic()).count() >= 2;
    (name.is_empty() || run_in).then_some(field)
}

/// The field numbers the lines are set with, as printed: "1", "4A". What a
/// document that carries them is — and whether these are a licence's at all —
/// is for whoever asks (`doctype`).
pub(super) fn field_numbers(lines: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for text in lines {
        let words = words(text);
        let dated = dates::find(text);
        for at in 0..words.len() {
            if let Some((number, _)) = field_number(text, &words, at, &dated) {
                found.push(number.to_owned());
            }
        }
    }
    found
}

/// The caption the words open with, if they open with one, and how many of
/// them it takes: the longest that fits, read exactly or — the long ones —
/// within a slip or two.
fn worded(words: &[Word]) -> Option<(usize, Field)> {
    let mut best: Option<(usize, bool, Field)> = None;
    for (field, captions) in CAPTIONS {
        for caption in *captions {
            let len = caption.split(' ').count();
            if len > words.len() {
                continue;
            }
            let seen = words[..len]
                .iter()
                .map(|w| w.key.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let exact = seen == *caption;
            let slips = match caption.len() {
                n if n >= TWO_SLIPS_FROM => 2,
                n if n >= ONE_SLIP_FROM => 1,
                _ => 0,
            };
            if !exact && (slips == 0 || strsim::levenshtein(&seen, caption) > slips) {
                continue;
            }
            if best.is_none_or(|(most, was_exact, _)| (len, exact) > (most, was_exact)) {
                best = Some((len, exact, *field));
            }
        }
    }
    best.map(|(len, _, field)| (len, field))
}

/// The field number the word at `at` is, where the print sets it as one, and
/// the worded caption that follows it if one does ("4d DLN").
///
/// A field number is one or two digits, up to the high teens a licence runs
/// to, and perhaps a letter: "2", "4a", "12". It stands apart — "2 JANICE",
/// "2.GIVEN", not "2-12" or "5'-06". In the middle of a line it is one only
/// where the print says so: a full stop after it, or the caption's own words.
///
/// And it is no piece of a date, `dated` being the line's: not the day a
/// date opens with ("12 AUG 1974", printed under its caption), nor a figure
/// in the middle of one ("23.08.2013") — though it may be set hard against
/// the date it captions ("3.12.08.1964"). Only a date with its year in full
/// settles that. Three figures with a two-digit year are as likely a field
/// number, a name and a stray figure ("2 JAN 68"), and there the number
/// stands and the date is dropped for running across it (`super::Dated`).
fn field_number<'a>(
    text: &str,
    words: &'a [Word],
    at: usize,
    dated: &[dates::Printed],
) -> Option<(&'a str, Option<(usize, Field)>)> {
    let word = &words[at];
    let key = word.key.as_str();
    let digits = key.chars().take_while(char::is_ascii_digit).count();
    let lettered = key[digits..].chars().all(|c| c.is_ascii_uppercase());
    let in_range = key[..digits]
        .parse::<u32>()
        .is_ok_and(|n| (1..=19).contains(&n));
    if digits > 2 || !in_range || key.len() > digits + 1 || !lettered {
        return None;
    }
    if dated
        .iter()
        .any(|d| d.full() && (d.start..d.end).contains(&word.start))
    {
        return None;
    }

    let mut after = text[word.end..].chars();
    let stopped = match after.next() {
        None | Some(' ') => false,
        Some('.') => {
            // A stop and then a figure is a decimal, or a date under way —
            // unless what follows the stop is a whole date of its own, and
            // this the number of the field it stands in.
            let figure = after.next().is_some_and(|c| c.is_ascii_digit());
            if figure && !dated.iter().any(|d| d.start == word.end + 1) {
                return None;
            }
            true
        }
        _ => return None,
    };
    let before = text[..word.start].chars().next_back();
    if before.is_some_and(|c| matches!(c, '.' | '/' | '-')) {
        return None;
    }

    let caption = worded(&words[at + 1..]);
    (at == 0 || stopped || caption.is_some()).then_some((key, caption))
}

/// What a licence's field number captions. The numbers no entry field takes
/// (8 the address, 9 the categories) are captions all the same. A four with
/// a letter this does not know is a misread 4a or 4b: which one cannot be
/// said, so it is no caption, and its date is left for the dates' own order
/// to settle.
fn numbered_field(number: &str) -> Option<Field> {
    Some(match number {
        "1" => Field::Surname,
        "2" => Field::Given,
        "3" => Field::Birth,
        "4A" => Field::Issue,
        "4B" => Field::Expiry,
        "4D" | "5" => Field::Number,
        "4C" => Field::Other,
        four if four.len() == 2 && four.starts_with('4') => return None,
        _ => Field::Other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(lines: &[&str], numbered: bool) -> Vec<(Field, String)> {
        let lines: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
        find(&lines, numbered)
            .into_iter()
            .map(|c| (c.field, lines[c.line][c.start..c.end].trim().to_owned()))
            .collect()
    }

    #[test]
    fn a_caption_is_followed_by_its_value_up_to_the_next_one() {
        assert_eq!(
            found(&["Surname: ERIKSSON  Sex F"], false),
            [
                (Field::Surname, ": ERIKSSON".into()),
                (Field::Sex, "F".into())
            ]
        );
        assert_eq!(
            found(&["GÜLTIG BIS 10.02.2021"], false),
            [(Field::Expiry, "10.02.2021".into())]
        );
    }

    #[test]
    fn a_caption_in_two_languages_is_one_caption() {
        assert_eq!(
            found(
                &["Surname / Nom", "Date of birth / Date de naissance"],
                false
            ),
            [
                (Field::Surname, String::new()),
                (Field::Birth, String::new())
            ]
        );
    }

    #[test]
    fn a_long_caption_is_read_through_a_slip_and_a_short_one_is_not() {
        assert_eq!(
            found(&["Dafe of blrth"], false),
            [(Field::Birth, String::new())]
        );
        // One letter from "GENDER", and somebody's name.
        assert_eq!(found(&["BENDER"], false), []);
    }

    #[test]
    fn a_licence_captions_by_number() {
        assert_eq!(
            found(
                &[
                    "4d DLN I1234567 4b EXP 11/26/2030",
                    "1 SAMPLE",
                    "2 JANICE 668",
                    "5 DD 00/00/0000",
                    "2.GIVEN NAMES ANGELINA GABRIELA 15.SEX 5-02 16.HGT",
                ],
                true
            ),
            [
                (Field::Number, "I1234567".into()),
                (Field::Expiry, "11/26/2030".into()),
                (Field::Surname, "SAMPLE".into()),
                (Field::Given, "JANICE 668".into()),
                // America's 5 is not the licence's number, and says so.
                (Field::Other, "00/00/0000".into()),
                (Field::Given, "ANGELINA GABRIELA".into()),
                (Field::Sex, "5-02".into()),
                (Field::Other, String::new()),
            ]
        );
    }

    #[test]
    fn a_licence_abbreviates_the_name_captions_and_runs_them_into_the_name() {
        assert_eq!(
            found(&["LN SAMPLE", "FN SEIJI"], false),
            [
                (Field::Surname, "SAMPLE".into()),
                (Field::Given, "SEIJI".into())
            ]
        );
        assert_eq!(
            found(
                &[
                    "LNCARDHOLDER",
                    "FNIMA",
                    "LNSMITH-JONES SEX F",
                    "LNO'BRIEN",
                    "FNZOË"
                ],
                false
            ),
            [
                (Field::Surname, "CARDHOLDER".into()),
                (Field::Given, "IMA".into()),
                (Field::Surname, "SMITH-JONES".into()),
                (Field::Sex, "F".into()),
                (Field::Surname, "O'BRIEN".into()),
                (Field::Given, "ZOË".into()),
            ]
        );
    }

    #[test]
    fn an_abbreviated_caption_is_one_only_at_the_head_of_a_line() {
        // A lane, a word that happens to open with the letters, a number.
        assert_eq!(
            found(
                &["123 MAIN LN", "STREET FNORD", "FN12345", "Fnord", "Ln 5"],
                false
            ),
            []
        );
    }

    #[test]
    fn a_field_number_may_stand_hard_against_its_date() {
        assert_eq!(
            found(
                &["3.12.08.1964 BERLIN", "4a.01.02.2015", "4b.31.01.2030"],
                true
            ),
            [
                (Field::Birth, ".12.08.1964 BERLIN".into()),
                (Field::Issue, ".01.02.2015".into()),
                (Field::Expiry, ".31.01.2030".into()),
            ]
        );
        // A date's own first figure is not one, nor a decimal's.
        assert_eq!(found(&["12.08.1964", "1.75 m"], true), []);
    }

    #[test]
    fn the_day_a_date_opens_with_is_not_a_field_number() {
        assert_eq!(
            found(
                &["12 AUG 1974", "01 JAN 2020", "12 08 1964", "Aug 17, 1958"],
                true
            ),
            []
        );
        // A field number before a date is still one.
        assert_eq!(
            found(&["3 12 AUG 1974", "4a 01 JAN 2020"], true),
            [
                (Field::Birth, "12 AUG 1974".into()),
                (Field::Issue, "01 JAN 2020".into()),
            ]
        );
    }

    #[test]
    fn a_figure_in_a_date_or_out_of_range_is_no_caption() {
        assert_eq!(
            found(
                &["4g. 23.08.2013 AOHNAS", "48. 23.03.2028 48, 012345678"],
                true
            ),
            []
        );
    }

    #[test]
    fn hands_over_the_field_numbers_as_printed() {
        let lines: Vec<String> = [
            "1. ЕAAHNIДA",
            "4g. 23.08.2013 AOHNAS",
            "9. A, B, C",
            "48. 23.03.2028",
        ]
        .iter()
        .map(|l| (*l).to_owned())
        .collect();
        assert_eq!(field_numbers(&lines), ["1", "4G", "9"]);
    }

    #[test]
    fn a_number_is_a_caption_only_on_a_licence() {
        assert_eq!(found(&["1 SAMPLE", "2 JANICE"], false), []);
    }
}
