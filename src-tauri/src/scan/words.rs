//! Recognized text as words: what the readers match print by, and what they
//! judge a word by.
//!
//! A recognizer returns what it saw, and what it saw of a caption is unstable
//! in ways that say nothing about the caption: its case, its diacritics
//! (dropped and invented freely), an apostrophe, and — on a document set
//! partly in Greek or Cyrillic — a Latin letter read as its lookalike from
//! the other alphabet. Matching goes by a key with all of that taken out. The
//! word keeps its place in the line, so a value can be cut from the original
//! text beside it.

/// One word of a line: where it stands in the line as recognized, in bytes,
/// and the key it is matched by.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Word {
    pub start: usize,
    pub end: usize,
    pub key: String,
}

/// The words of a line. A word is a run of letters and digits; an apostrophe
/// inside one closes up (`DRIVER'S` is one word, with or without the mark),
/// and anything else is a break.
pub(super) fn words(line: &str) -> Vec<Word> {
    let mut found = Vec::new();
    let mut open: Option<Word> = None;
    for (at, c) in line.char_indices() {
        let past = at + c.len_utf8();
        if matches!(c, '\'' | '’' | '‘' | '`' | '´') {
            if let Some(word) = open.as_mut() {
                word.end = past;
            }
        } else if c.is_alphanumeric() {
            let word = open.get_or_insert_with(|| Word {
                start: at,
                end: at,
                key: String::new(),
            });
            word.end = past;
            word.key.extend(c.to_uppercase().map(fold));
        } else {
            found.extend(open.take());
        }
    }
    found.extend(open);
    found
}

/// The keys alone, for text that is only matched and never cut from.
pub(super) fn keys(line: &str) -> Vec<String> {
    words(line).into_iter().map(|word| word.key).collect()
}

/// Whether the words of `phrase` stand together in `text`, in order — `text`
/// being the keys of some print, its lines run together so that a phrase set
/// over two lines still reads. Whole words: "ID CARD" is not found in "VALID
/// CARD". The phrase goes through the same reduction the text did, so the two
/// meet whatever that does to a letter.
pub(super) fn says(text: &[String], phrase: &str) -> bool {
    let phrase = keys(phrase);
    text.windows(phrase.len()).any(|window| window == phrase)
}

/// A stretch of text as the words a value is judged by — is this a name, a
/// number: split at spaces, with the punctuation around each taken off.
pub(super) fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|word| !word.is_empty())
}

/// Whether a word could be part of a person's name: letters, and the marks a
/// name is written with.
pub(super) fn is_name_word(word: &str) -> bool {
    word.chars().any(char::is_alphabetic)
        && word
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, '-' | '\'' | '’' | '.'))
}

/// Whether a word mixes Latin letters with Greek or Cyrillic ones. No name is
/// spelled that way: it is what a recognizer reading one alphabet makes of a
/// word printed in another, and nothing to put on a form.
pub(super) fn mixes_scripts(word: &str) -> bool {
    let latin = word
        .chars()
        .any(|c| c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c));
    let other = word
        .chars()
        .any(|c| ('\u{0370}'..='\u{03FF}').contains(&c) || ('\u{0400}'..='\u{04FF}').contains(&c));
    latin && other
}

// An uppercase letter as it is matched: without its mark, and as the Latin
// letter (or the digit) it is the double of. Only what captions and titles
// need; anything else passes through as it is.
fn fold(c: char) -> char {
    match c {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ą' | 'Ă' => 'A',
        'Ç' | 'Ć' | 'Č' => 'C',
        'Ď' | 'Đ' => 'D',
        'È' | 'É' | 'Ê' | 'Ë' | 'Ę' | 'Ě' => 'E',
        'Ğ' => 'G',
        'Ì' | 'Í' | 'Î' | 'Ï' | 'İ' => 'I',
        'Ł' => 'L',
        'Ñ' | 'Ń' | 'Ň' => 'N',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ő' => 'O',
        'Ř' => 'R',
        'Ś' | 'Š' | 'Ş' | 'Ș' => 'S',
        'Ť' | 'Ţ' | 'Ț' => 'T',
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ů' | 'Ű' => 'U',
        'Ý' | 'Ÿ' => 'Y',
        'Ź' | 'Ż' | 'Ž' => 'Z',
        // Cyrillic and Greek capitals that are the image of a Latin one.
        'А' | 'Α' => 'A',
        'В' | 'Β' => 'B',
        'С' => 'C',
        'Е' | 'Ε' => 'E',
        'Н' | 'Η' => 'H',
        'І' | 'Ι' => 'I',
        'Ј' => 'J',
        'К' | 'Κ' => 'K',
        'М' | 'Μ' => 'M',
        'Ν' => 'N',
        'О' | 'Ο' => 'O',
        'Р' | 'Ρ' => 'P',
        'Ѕ' => 'S',
        'Т' | 'Τ' => 'T',
        'Х' | 'Χ' => 'X',
        'У' | 'Υ' => 'Y',
        'Ζ' => 'Z',
        'З' => '3',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_matched_without_case_marks_or_apostrophes() {
        assert_eq!(keys("Führerschein"), ["FUHRERSCHEIN"]);
        assert_eq!(keys("Driver's licence"), ["DRIVERS", "LICENCE"]);
        assert_eq!(
            keys("Carte nationale d’identité"),
            ["CARTE", "NATIONALE", "DIDENTITE"]
        );
        assert_eq!(keys("4d.DLN 4b.EXP"), ["4D", "DLN", "4B", "EXP"]);
    }

    #[test]
    fn a_lookalike_letter_reads_as_the_latin_one() {
        // "3 DOB", as a recognizer expecting Cyrillic too gave it back.
        assert_eq!(keys("ЗDов"), ["3DOB"]);
    }

    #[test]
    fn a_word_keeps_its_place_in_the_line() {
        let line = "Né(e) le: 12.08.1974";
        let found = words(line);
        assert_eq!(&line[found[0].start..found[0].end], "Né");
        assert_eq!(&line[found[2].start..found[2].end], "le");
    }

    #[test]
    fn judges_a_word_by_what_is_left_without_its_punctuation() {
        assert_eq!(
            tokens(": O'BRIEN-SMITH, 668 (JR.)").collect::<Vec<_>>(),
            ["O'BRIEN-SMITH", "668", "JR"]
        );
        assert!(is_name_word("O'BRIEN-SMITH"));
        assert!(is_name_word("Łukasz"));
        assert!(!is_name_word("668"));
        assert!(!is_name_word("I1234567"));
        assert!(!is_name_word("-"));
    }

    #[test]
    fn tells_a_misread_word_from_a_name() {
        assert!(mixes_scripts("МАРІA"));
        assert!(!mixes_scripts("MARIA"));
        assert!(!mixes_scripts("МАРІЯ"));
        assert!(!mixes_scripts("Łukasz"));
    }
}
