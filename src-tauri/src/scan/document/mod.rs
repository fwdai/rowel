//! Reading the printed face of an identity document: what it is, whose it is,
//! its number and its dates.
//!
//! Nothing here knows a particular document. Documents differ in layout and
//! language but not in kind: each says what it is in a title (`doctype`),
//! captions what it prints (`captions`) and prints its dates in one of a few
//! ways (`dates`). So a field is found the way a person finds it — by the
//! caption beside or above it — and checked by what it has to be: a date is a
//! date, a number has digits in it, a name has none.
//!
//! Where a caption is missing or was misread, the dates still place
//! themselves: a document's birth date comes before its issue date, and its
//! expiry is the last of the three.
//!
//! Text in, fields out, like the other readers. It is a best reading rather
//! than a verified one — nothing here has a check digit — which is why an
//! MRZ, when the picture has one, is read first and believed over this.

mod captions;
mod country;
mod dates;
mod doctype;

use std::collections::{BTreeMap, HashSet};

use chrono::{Months, NaiveDate};

use crate::scan::words;
use captions::{Caption, Field};

pub use doctype::printed as doc_type;

/// The fields the document's print gives, keyed as an identity entry's are:
/// `name`, `number`, `country`, `birth_date`, `issue_date`, `expiry_date` and
/// `sex`, each present only when it was found. `doc_type` is what [`doc_type`] made
/// of the same lines; a driving licence's fields are captioned by number as
/// well as by word.
pub fn fields(lines: &[String], doc_type: Option<&str>) -> BTreeMap<String, String> {
    let print = Print {
        lines,
        captions: captions::find(lines, doc_type == Some("driver_license")),
    };
    let country = country::issuer(print.head());
    // Where a date's month comes first, and nowhere else.
    let dated = Dated::read(&print, country == Some("USA"));
    // Without a caption, a number is guessed at only on a document that at
    // least said what it is.
    let number = print
        .value(Field::Number, number_in)
        .or_else(|| doc_type.and_then(|_| lone_number(lines)));
    [
        ("name", name(&print)),
        ("number", number),
        ("country", country.map(str::to_owned)),
        ("sex", print.value(Field::Sex, sex_in)),
        ("birth_date", dated.birth.map(dates::iso)),
        ("issue_date", dated.issue.map(dates::iso)),
        ("expiry_date", dated.expiry.map(dates::iso)),
    ]
    .into_iter()
    .filter_map(|(key, value)| Some((key.to_owned(), value?)))
    .collect()
}

// How far down a document's head runs where no caption ends it.
const HEAD_LINES: usize = 3;

/// A document's print: its lines and the captions found in them.
struct Print<'a> {
    lines: &'a [String],
    captions: Vec<Caption>,
}

impl Print<'_> {
    /// The document's head, where it says what it is and who issued it: the
    /// lines above its first caption, and no more than [`HEAD_LINES`] of them
    /// where it has none.
    fn head(&self) -> &[String] {
        let lines = self
            .captions
            .first()
            .map_or(HEAD_LINES, |caption| caption.line.clamp(1, HEAD_LINES));
        &self.lines[..lines.min(self.lines.len())]
    }

    fn of(&self, field: Field) -> impl Iterator<Item = &Caption> {
        self.captions.iter().filter(move |c| c.field == field)
    }

    /// What stands beside a caption, on its own line.
    fn beside(&self, caption: &Caption) -> &str {
        &self.lines[caption.line][caption.start..caption.end]
    }

    /// The line under `line`, when it is all value: documents print the
    /// caption above the data as often as before it, and a line with a
    /// caption of its own belongs to that caption.
    fn below(&self, line: usize) -> Option<&str> {
        self.lines
            .get(line + 1)
            .filter(|_| self.captions.iter().all(|c| c.line != line + 1))
            .map(String::as_str)
    }

    /// The value of `field`: what `read` makes of the text beside the first
    /// of its captions to have one, or below it.
    fn value(&self, field: Field, read: impl Fn(&str) -> Option<String>) -> Option<String> {
        self.of(field).find_map(|caption| {
            read(self.beside(caption)).or_else(|| self.below(caption.line).and_then(&read))
        })
    }
}

/// The holder's name, given names first — the way a person writes their own,
/// and the way the MRZ reader hands it over.
///
/// What "Name" captions depends on what else the document captions. Beside a
/// caption for the given names it is the family name ("Name" / "Vornamen");
/// on its own it is the whole name.
fn name(print: &Print<'_>) -> Option<String> {
    let of = |field: Field| print.of(field).find_map(|caption| named(print, caption));
    let given = of(Field::Given);
    let surname = of(Field::Surname).or_else(|| given.as_ref().and_then(|_| of(Field::Name)));
    match (given, surname) {
        // Two captions over one line of print ("Surname  Given names" above
        // "ERIKSSON ANNA MARIA") both find the whole of it.
        (Some(given), Some(surname)) if given == surname => Some(given),
        (Some(given), Some(surname)) => Some(format!("{given} {surname}")),
        (Some(part), None) | (None, Some(part)) => Some(part),
        (None, None) => whole_name(print, print.of(Field::Name).next()?),
    }
}

/// The value of a caption for one part of the name: beside it, or else on the
/// line below.
fn named(print: &Print<'_>, caption: &Caption) -> Option<String> {
    name_in(print.beside(caption), false).or_else(|| name_below(print, caption.line))
}

fn name_below(print: &Print<'_>, line: usize) -> Option<String> {
    print.below(line).and_then(|line| name_in(line, true))
}

// The most letters a stray mark beside a caption runs to.
const DEBRIS_LETTERS: usize = 3;

/// The name under a single "Name" caption, which a document sets over up to
/// two lines: the surname, then the given names.
///
/// It starts beside the caption where anything is, and on the line below
/// otherwise. A few letters beside the caption are a name like any other —
/// Li, Ng, Lee — unless two lines of names follow: then the name is whole
/// without them, and they are a mark the recognizer picked up by the caption.
fn whole_name(print: &Print<'_>, caption: &Caption) -> Option<String> {
    let beside = name_in(print.beside(caption), false);
    let below = name_below(print, caption.line);
    let further = || below.as_ref().and(name_below(print, caption.line + 1));
    let (first, next) = match (beside, &below) {
        (Some(beside), Some(below))
            if beside.chars().filter(|c| c.is_alphabetic()).count() <= DEBRIS_LETTERS
                && further().is_some() =>
        {
            (below.clone(), further())
        }
        (Some(beside), _) => (beside, below),
        (None, Some(below)) => (below.clone(), further()),
        (None, None) => return None,
    };
    // One word with a line of names under it is the surname over the given
    // names; more than one is the name already.
    Some(match next {
        Some(given) if !first.contains(' ') => format!("{given} {first}"),
        _ => first,
    })
}

/// The name in a stretch of text. Beside a caption, what is not a name is
/// dropped — a stray figure, the colon after the caption. A line on its own
/// has to be a name through and through (`whole`), or it is something else:
/// an address, a place, a number.
fn name_in(text: &str, whole: bool) -> Option<String> {
    let (names, rest): (Vec<&str>, Vec<&str>) =
        words::tokens(text).partition(|word| words::is_name_word(word));
    if names.is_empty() || (whole && !rest.is_empty()) {
        return None;
    }
    // A word in two alphabets at once is a misreading, and so is its line.
    if names.iter().any(|word| words::mixes_scripts(word)) {
        return None;
    }
    Some(names.join(" "))
}

/// A document or licence number: the first word with enough digits in it to
/// be one. Not a date — those are written with marks a number does not have.
fn number_in(text: &str) -> Option<String> {
    words::tokens(text)
        .find(|word| {
            (5..=20).contains(&word.chars().count())
                && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && word.chars().filter(char::is_ascii_digit).count() >= 3
        })
        .map(str::to_owned)
}

/// The number of a document that does not caption it, when the print leaves
/// no doubt: the one word on it that mixes letters with digits the way a
/// document number does. Two such words are a doubt.
fn lone_number(lines: &[String]) -> Option<String> {
    let mut candidates = lines
        .iter()
        .flat_map(|line| line.split_whitespace())
        .filter(|word| {
            (7..=12).contains(&word.len())
                && word
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                && word.chars().any(|c| c.is_ascii_uppercase())
                && word.chars().filter(char::is_ascii_digit).count() >= 4
        });
    let only = candidates.next()?;
    candidates.next().is_none().then(|| only.to_owned())
}

/// `M` or `F`, as the MRZ reader gives it; anything else is left unsaid.
fn sex_in(text: &str) -> Option<String> {
    words::tokens(text)
        // "V/F": the letter in the document's language, then the
        // international one.
        .flat_map(|word| word.split('/'))
        .find_map(|word| match word.to_uppercase().as_str() {
            "M" | "MALE" => Some("M"),
            "F" | "FEMALE" => Some("F"),
            _ => None,
        })
        .map(str::to_owned)
}

/// The three dates a document carries.
#[derive(Default)]
struct Dated {
    birth: Option<NaiveDate>,
    issue: Option<NaiveDate>,
    expiry: Option<NaiveDate>,
}

// Nobody holds a document that outlives its issue by this long, so a date
// further than this before the expiry is the holder's birth, not the issue.
const LONGEST_VALIDITY_YEARS: u32 = 16;

impl Dated {
    fn read(print: &Print<'_>, month_first: bool) -> Self {
        let today = chrono::Utc::now().date_naive();
        let printed: Vec<Vec<dates::Printed>> =
            print.lines.iter().map(|line| dates::find(line)).collect();
        let order = dates::order(printed.iter().flatten(), month_first);
        // Every date as (line, where it starts, the day it is as an expiry,
        // the day it is as a birth or an issue): a year printed with two
        // digits is not the same year for both.
        let all: Vec<(usize, usize, NaiveDate, NaiveDate)> = printed
            .iter()
            .enumerate()
            .flat_map(|(line, dates)| dates.iter().map(move |d| (line, d)))
            // A date lies within one value. One that runs across a caption
            // took the caption for part of itself: a licence's field number
            // for a day ("2 JAN 68", field 2 and somebody called Jan).
            .filter(|(line, d)| {
                !print
                    .captions
                    .iter()
                    .any(|c| c.line == *line && d.start < c.start && c.start <= d.end)
            })
            .filter_map(|(line, d)| Some((line, d.start, d.on(order)?, d.past(order, today)?)))
            .collect();
        let mut taken: HashSet<(usize, usize)> = HashSet::new();
        let mut dated = Self::default();
        let mut born_captioned = false;

        // A date beside its caption first, for every caption: only then is a
        // date on the line below known to be nobody else's.
        for below in [false, true] {
            for caption in &print.captions {
                let found = all.iter().find(|(line, start, ..)| {
                    !taken.contains(&(*line, *start))
                        && if below {
                            *line == caption.line + 1
                        } else {
                            *line == caption.line && (caption.start..caption.end).contains(start)
                        }
                });
                let Some(&(line, start, ahead, past)) = found else {
                    continue;
                };
                let (field, date) = match caption.field {
                    Field::Birth => (&mut dated.birth, past),
                    Field::Issue => (&mut dated.issue, past),
                    Field::Expiry => (&mut dated.expiry, ahead),
                    _ => continue,
                };
                if field.is_none() {
                    *field = Some(date);
                    taken.insert((line, start));
                    // A date under a caption is that caption's when the
                    // caption stands alone above it. Where something else is
                    // printed beside the caption, that was its value — misread
                    // — and the date below is only the next thing down.
                    born_captioned |= caption.field == Field::Birth
                        && (!below || words::tokens(print.beside(caption)).next().is_none());
                }
            }
        }

        let rest = all
            .iter()
            .filter(|(line, start, ..)| !taken.contains(&(*line, *start)))
            .map(|(_, _, ahead, past)| (*ahead, *past))
            .collect();
        dated.place(rest, today);
        // A birth date its caption did not vouch for — taken by order, or
        // from under a caption that had something else beside it — is checked
        // against the expiry: too close to it, with no issue date known, it is
        // the issue date. One the document captioned stands whatever its
        // distance: a child's card runs out well inside sixteen years.
        if !born_captioned && dated.issue.is_none() {
            if let (Some(birth), Some(expiry)) = (dated.birth, dated.expiry) {
                if !outlived(birth, expiry) {
                    dated.issue = dated.birth.take();
                }
            }
        }
        dated
    }

    /// Give the dates no caption claimed to the fields still empty, by what
    /// order they have to come in: birth, then issue, then expiry. Each is
    /// handed over as the day it is as an expiry and the day it is otherwise.
    fn place(&mut self, mut unclaimed: Vec<(NaiveDate, NaiveDate)>, today: NaiveDate) {
        let known = [self.birth, self.issue, self.expiry];
        unclaimed.retain(|(ahead, past)| {
            !known.contains(&Some(*ahead)) && !known.contains(&Some(*past))
        });
        unclaimed.sort();
        unclaimed.dedup();
        if self.expiry.is_none() {
            if let Some(&(last, _)) = unclaimed.last() {
                let known = self.birth.max(self.issue);
                // The last date is the expiry when there is anything for it
                // to be later than; a lone date is one only if still ahead.
                if known.is_none_or(|k| last > k)
                    && (unclaimed.len() >= 2 || known.is_some() || last > today)
                {
                    self.expiry = Some(last);
                    unclaimed.pop();
                }
            }
        }
        // What is left is a birth or an issue, and neither is ahead of today.
        let mut rest: Vec<NaiveDate> = unclaimed.into_iter().map(|(_, past)| past).collect();
        rest.sort();
        rest.dedup();
        if self.birth.is_none() {
            if let Some(&first) = rest.first() {
                let is_birth = match (self.issue, self.expiry) {
                    (Some(issue), _) => first < issue,
                    (None, _) if rest.len() >= 2 => true,
                    (None, Some(expiry)) => outlived(first, expiry),
                    (None, None) => false,
                };
                if is_birth {
                    self.birth = Some(rest.remove(0));
                }
            }
        }
        if self.issue.is_none() {
            self.issue = rest
                .iter()
                .rev()
                .find(|date| {
                    **date <= today
                        && self.birth.is_none_or(|birth| **date > birth)
                        && self.expiry.is_none_or(|expiry| **date < expiry)
                })
                .copied();
        }
    }
}

/// Whether `expiry` is further from `date` than any document stays valid, so
/// that `date` can only be the holder's birth.
fn outlived(date: NaiveDate, expiry: NaiveDate) -> bool {
    date.checked_add_months(Months::new(12 * LONGEST_VALIDITY_YEARS))
        .is_some_and(|latest| latest <= expiry)
}
