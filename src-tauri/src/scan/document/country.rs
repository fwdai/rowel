//! Which country issued a document, read from where a document says so: its
//! head.
//!
//! A document names its issuer at the top — "USA", "BUNDESREPUBLIK
//! DEUTSCHLAND", "KONINKRIJK DER NEDERLANDEN" — in its own language, in
//! English, or both. So this is a table of those names against the ISO 3166-1
//! alpha-3 code an entry keeps, the one an MRZ gives. Not every country is in
//! it, and a document whose issuer is not, or whose head the recognizer made
//! nothing of, is left for the user to say.
//!
//! Only the head is asked. Further down, a country's name is as likely the
//! holder's nationality or birthplace on some other country's document.

use crate::scan::words::{keys, says};

/// The names a document prints for its issuer, by that issuer's code. The
/// longest name a head carries is its issuer's — one country's name is often
/// the tail of another's ("REPUBLIC OF KOREA") — and between names as long,
/// the earlier here. So the United States comes first: several of its states
/// share a name with a country ("NEW MEXICO USA"), and the document says
/// which it is.
///
/// Left out on purpose: a name that is also an ordinary word or a state of
/// another country ("ISLAND", "GEORGIA", "JERSEY").
const ISSUERS: &[(&str, &[&str])] = &[
    ("USA", &["USA", "UNITED STATES OF AMERICA", "UNITED STATES"]),
    ("CAN", &["CANADA"]),
    ("GBR", &["UNITED KINGDOM", "GREAT BRITAIN", "UK"]),
    ("IRL", &["IRELAND", "EIRE"]),
    (
        "DEU",
        &["BUNDESREPUBLIK DEUTSCHLAND", "DEUTSCHLAND", "GERMANY"],
    ),
    ("AUT", &["REPUBLIK OSTERREICH", "OSTERREICH", "AUSTRIA"]),
    ("CHE", &["SCHWEIZ", "SUISSE", "SVIZZERA", "SWITZERLAND"]),
    ("FRA", &["REPUBLIQUE FRANCAISE", "FRANCE"]),
    ("ESP", &["REINO DE ESPANA", "ESPANA", "SPAIN"]),
    ("PRT", &["REPUBLICA PORTUGUESA", "PORTUGAL"]),
    ("ITA", &["REPUBBLICA ITALIANA", "ITALIA", "ITALY"]),
    (
        "NLD",
        &["KONINKRIJK DER NEDERLANDEN", "NEDERLAND", "NETHERLANDS"],
    ),
    ("BEL", &["BELGIQUE", "BELGIE", "BELGIEN", "BELGIUM"]),
    ("LUX", &["LUXEMBOURG"]),
    ("DNK", &["DANMARK", "DENMARK"]),
    ("SWE", &["SVERIGE", "SWEDEN"]),
    ("NOR", &["NORGE", "NOREG", "NORWAY"]),
    ("FIN", &["SUOMI", "FINLAND"]),
    ("ISL", &["ICELAND"]),
    ("POL", &["RZECZPOSPOLITA POLSKA", "POLSKA", "POLAND"]),
    ("CZE", &["CESKA REPUBLIKA", "CZECH REPUBLIC", "CZECHIA"]),
    ("SVK", &["SLOVENSKA REPUBLIKA", "SLOVENSKO", "SLOVAKIA"]),
    ("HUN", &["MAGYARORSZAG", "HUNGARY"]),
    ("ROU", &["ROMANIA"]),
    ("BGR", &["BULGARIA"]),
    ("GRC", &["HELLENIC REPUBLIC", "HELLAS", "GREECE"]),
    ("HRV", &["REPUBLIKA HRVATSKA", "HRVATSKA", "CROATIA"]),
    ("SVN", &["REPUBLIKA SLOVENIJA", "SLOVENIJA", "SLOVENIA"]),
    ("EST", &["EESTI", "ESTONIA"]),
    ("LVA", &["LATVIJA", "LATVIA"]),
    ("LTU", &["LIETUVA", "LITHUANIA"]),
    ("UKR", &["УКРАЇНА", "UKRAINE"]),
    ("TUR", &["TURKIYE CUMHURIYETI", "TURKIYE"]),
    ("AUS", &["AUSTRALIA"]),
    ("NZL", &["NEW ZEALAND"]),
    ("MEX", &["ESTADOS UNIDOS MEXICANOS", "MEXICO"]),
    (
        "BRA",
        &["REPUBLICA FEDERATIVA DO BRASIL", "BRASIL", "BRAZIL"],
    ),
    ("ARG", &["REPUBLICA ARGENTINA", "ARGENTINA"]),
    ("IND", &["REPUBLIC OF INDIA", "INDIA"]),
    ("JPN", &["JAPAN"]),
    ("KOR", &["REPUBLIC OF KOREA"]),
    ("PRK", &["DEMOCRATIC PEOPLES REPUBLIC OF KOREA"]),
    ("CHN", &["PEOPLES REPUBLIC OF CHINA"]),
    ("SGP", &["REPUBLIC OF SINGAPORE", "SINGAPORE"]),
    ("ZAF", &["SOUTH AFRICA"]),
    ("ISR", &["ISRAEL"]),
    ("ARE", &["UNITED ARAB EMIRATES"]),
];

/// The country that issued the document whose head these lines are, as its
/// ISO 3166-1 alpha-3 code.
pub(super) fn issuer(head: &[String]) -> Option<&'static str> {
    let text: Vec<String> = head.iter().flat_map(|line| keys(&apart(line))).collect();
    let mut found: Option<(usize, &'static str)> = None;
    for (code, names) in ISSUERS {
        for name in names.iter().filter(|name| says(&text, name)) {
            let words = name.split(' ').count();
            if found.is_none_or(|(most, _)| words > most) {
                found = Some((words, code));
            }
        }
    }
    found.map(|(_, code)| code)
}

/// A line with the words a recognizer ran together set apart again, where
/// the print's own capitals show the join: "CaliforniaUSA" is a state's name
/// in its lettering and the country's code in capitals beside it.
fn apart(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 4);
    let mut after_small = false;
    for c in line.chars() {
        if after_small && c.is_uppercase() {
            out.push(' ');
        }
        after_small = c.is_lowercase();
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(head: &[&str]) -> Option<&'static str> {
        issuer(&head.iter().map(|l| (*l).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn reads_the_issuer_in_its_own_language_or_in_english() {
        assert_eq!(of(&["California USA DRIVER LICENSE"]), Some("USA"));
        // As a recognizer sometimes hands the same head over.
        assert_eq!(of(&["CaliforniaUSA DRIVER LICENSE"]), Some("USA"));
        assert_eq!(
            of(&["BUNDESREPUBLIK DEUTSCHLAND", "PERSONALAUSWEIS"]),
            Some("DEU")
        );
        assert_eq!(of(&["République française"]), Some("FRA"));
        assert_eq!(
            of(&["PASSPORT PASPOORT KONINKRIJK DER NEDERLANDEN"]),
            Some("NLD")
        );
        assert_eq!(of(&["UNITED KINGDOM", "DRIVING LICENCE"]), Some("GBR"));
        assert_eq!(of(&["DRIVING LICENCE"]), None);
    }

    #[test]
    fn a_name_inside_a_longer_one_is_the_longer_ones_country() {
        assert_eq!(
            of(&["DEMOCRATIC PEOPLE'S REPUBLIC OF KOREA", "PASSPORT"]),
            Some("PRK")
        );
        assert_eq!(of(&["REPUBLIC OF KOREA", "PASSPORT"]), Some("KOR"));
        assert_eq!(
            of(&["UNITED KINGDOM OF GREAT BRITAIN AND NORTHERN IRELAND"]),
            Some("GBR")
        );
    }

    #[test]
    fn a_state_that_shares_a_name_with_a_country_is_its_own_countrys() {
        assert_eq!(of(&["NEW MEXICO USA", "DRIVER LICENSE"]), Some("USA"));
        // Whole words: India is not in Indiana.
        assert_eq!(of(&["INDIANA DRIVER'S LICENSE"]), None);
    }
}
