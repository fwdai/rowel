//! What kind of identity document the recognized text is off, read from what
//! the document prints about itself.
//!
//! No model and nothing per country: a document says what it is in words —
//! "DRIVING LICENCE", "AUFENTHALTSTITEL", "PERMANENT RESIDENT" — and a driving
//! licence also numbers its fields the same way wherever it is issued. Both
//! are plain text, so this is a table and a pattern.
//!
//! It answers with one of the entry's `doc_type` values, or `None` for a
//! document that names itself in no way listed here: a title in a script the
//! recognizer was not reading, or none at all.

use super::captions;
use crate::scan::words::keys;

/// The titles documents print, by the type they name, in the order they are
/// asked: a licence or a permit says so in words nothing else carries, an
/// identity card's title is the most generic of the three, and "passport" is
/// last because other documents mention one.
///
/// Uppercase and without diacritics or apostrophes: near enough the form the
/// recognized text is reduced to for matching (`scan::words`).
const TITLES: [(&str, &[&str]); 4] = [
    (
        "driver_license",
        &[
            "DRIVER LICENSE",
            "DRIVERS LICENSE",
            "DRIVER LICENCE",
            "DRIVERS LICENCE",
            "DRIVING LICENCE",
            "DRIVING LICENSE",
            "PERMIS DE CONDUIRE",
            "FUHRERSCHEIN",
            "PERMISO DE CONDUCCION",
            "PERMISO DE CONDUCIR",
            "LICENCIA DE CONDUCIR",
            "LICENCIA DE CONDUCCION",
            "PATENTE DI GUIDA",
            "RIJBEWIJS",
            "CARTA DE CONDUCAO",
            "CARTEIRA NACIONAL DE HABILITACAO",
            "PRAWO JAZDY",
            "KORKORT",
            "KOREKORT",
            "FORERKORT",
            "AJOKORTTI",
            "RIDICSKY PRUKAZ",
            "VODICSKY PREUKAZ",
            "VEZETOI ENGEDELY",
            "PERMIS DE CONDUCERE",
            "SURUCU BELGESI",
            "VOZACKA DOZVOLA",
            "ВОДИТЕЛЬСКОЕ УДОСТОВЕРЕНИЕ",
            "ПОСВІДЧЕННЯ ВОДІЯ",
        ],
    ),
    (
        "residence_permit",
        &[
            "RESIDENCE PERMIT",
            "RESIDENCE CARD",
            "PERMANENT RESIDENT",
            "PERMANENT RESIDENCE",
            "AUFENTHALTSTITEL",
            "TITRE DE SEJOUR",
            "PERMESSO DI SOGGIORNO",
            "PERMISO DE RESIDENCIA",
            "VERBLIJFSTITEL",
            "VERBLIJFSDOCUMENT",
            "TITULO DE RESIDENCIA",
            "AUTORIZACAO DE RESIDENCIA",
            "KARTA POBYTU",
            "UPPEHALLSTILLSTAND",
            "OPPHOLDSTILLATELSE",
            "OPHOLDSTILLADELSE",
            "OLESKELULUPA",
            "POVOLENI K POBYTU",
            "PERMIS DE SEDERE",
            "IKAMET IZNI",
            "ВИД НА ЖИТЕЛЬСТВО",
            "ПОСВІДКА НА ПРОЖИВАННЯ",
        ],
    ),
    (
        "id_card",
        &[
            "IDENTITY CARD",
            "IDENTIFICATION CARD",
            "ID CARD",
            "NATIONAL ID",
            "PERSONALAUSWEIS",
            "CARTE NATIONALE DIDENTITE",
            "CARTE DIDENTITE",
            "CARTA DIDENTITA",
            "CARTA DI IDENTITA",
            "DOCUMENTO NACIONAL DE IDENTIDAD",
            "CEDULA DE IDENTIDAD",
            "CEDULA DE CIUDADANIA",
            "IDENTITEITSKAART",
            "CARTAO DE CIDADAO",
            "DOWOD OSOBISTY",
            "IDENTITETSKORT",
            "HENKILOKORTTI",
            "OBCANSKY PRUKAZ",
            "OBCIANSKY PREUKAZ",
            "CARTE DE IDENTITATE",
            "KIMLIK KARTI",
            "OSOBNA ISKAZNICA",
            "LICNA KARTA",
            "ISIKUTUNNISTUS",
        ],
    ),
    (
        "passport",
        &[
            "PASSPORT",
            "PASSEPORT",
            "REISEPASS",
            "PASAPORTE",
            "PASSAPORTO",
            "PASPOORT",
            "PASSAPORTE",
            "PASZPORT",
        ],
    ),
];

/// The type the document's own print gives it: the title it carries or,
/// failing a title, the numbered fields of a driving licence.
pub fn printed(lines: &[String]) -> Option<&'static str> {
    let text: Vec<String> = lines.iter().flat_map(|line| keys(line)).collect();
    TITLES
        .iter()
        .find(|(_, titles)| titles.iter().any(|title| says(&text, title)))
        .map(|(doc_type, _)| *doc_type)
        .or_else(|| numbered_like_a_licence(lines).then_some("driver_license"))
}

/// Whether the words of `title` stand together in `text`, in order. Whole
/// words: "ID CARD" is not found in "VALID CARD". The text is the document's
/// lines run together, so a title set over two lines still reads. The title
/// goes through the same reduction the text did, so the two meet whatever
/// that does to a letter.
fn says(text: &[String], title: &str) -> bool {
    let title = keys(title);
    text.windows(title.len()).any(|window| window == title)
}

/// Whether the lines carry a driving licence's numbered fields.
///
/// A licence labels what it prints by number — 1 the surname, 2 the given
/// names, 3 the birth date, 4a the issue date, 4b the expiry — under ISO/IEC
/// 18013, the EU's model and the Vienna Convention alike, so the same marks
/// are on a Greek licence and a Californian one, whatever script the title is
/// in. The lettered fours are the tell: a numbered list has a 4, never a 4a.
/// One of them beside two of the first three is asked for, because a
/// recognizer misreads any one mark often enough.
fn numbered_like_a_licence(lines: &[String]) -> bool {
    let numbers = captions::field_numbers(lines);
    let has = |number: &str| numbers.iter().any(|n| n == number);
    let lettered_four = numbers.iter().any(|n| n.len() == 2 && n.starts_with('4'));
    lettered_four && ["1", "2", "3"].into_iter().filter(|n| has(n)).count() >= 2
}
