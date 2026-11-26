//! The dates a document prints, however it prints them.
//!
//! Documents agree on little here: `26.11.2030`, `11/26/2030`, `2030-11-26`,
//! `26 NOV 2030`, `26 NOV/NOV 30`. Finding them is a handful of patterns. What
//! figures alone do not say is which of the first two numbers is the day —
//! and that is settled per document rather than per date: one date that can
//! only be read one way (`11/26/2030` has no 26th month) says how the document
//! writes all of them.

use std::sync::LazyLock;

use chrono::{Datelike, NaiveDate};
use regex::{Captures, Regex};

/// A date as printed: where it stands in its line, in bytes, and what it
/// says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Printed {
    pub start: usize,
    pub end: usize,
    form: Form,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Form {
    /// Two figures and a year: which is the day is the document's convention.
    Figures { first: u32, second: u32, year: Year },
    /// The year first, or the month by name: nothing left to tell apart.
    Plain { day: u32, month: u32, year: Year },
}

/// A year as printed. Two digits leave the century open, and which it is
/// depends on what the date is: an expiry may be years ahead, a birth cannot
/// be. So it is kept as printed until whoever knows what the date is asks.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Year {
    Full(i32),
    Short(i32),
}

/// Which of two figures a document puts first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Order {
    DayFirst,
    MonthFirst,
}

/// How to read one pattern's match, or `None` where what matched is not a
/// date after all: a month that is no month's name.
type Reader = fn(&Captures<'_>) -> Option<Form>;

/// The ways a date is printed, each with how its match is read. In the order
/// they are asked: where two would claim the same stretch of a line, the
/// first has it.
///
/// So every form with the year in full comes before any with it in two
/// digits. Four digits are unmistakably a year; two are a guess, and a guess
/// asked first takes what was never its own — the field number set against
/// a date, "3.12.08.1964", read as the third of December 2008.
static PATTERNS: LazyLock<Vec<(Regex, Reader)>> = LazyLock::new(|| {
    const FULL: &str = r"(\d{4})\b";
    // Two digits that end the run of figures they are in: followed by
    // another mark and figure they are its middle, not a year.
    const SHORT: &str = r"(\d{2})(?:$|[^\d./-]|[./-](?:$|\D))";
    let pattern = |pattern: &str, read: Reader| (Regex::new(pattern).unwrap(), read);
    // 26.11.2030, 11/26/30
    let in_figures = |year: &str| format!(r"\b(\d{{1,2}})[./-](\d{{1,2}})[./-]{year}");
    // 12 AUG 1974, 12 AUG/AOUT 74, 3. März 2020: a second month name after a
    // slash is the same month in the document's other language, and the one
    // to go by when the first is in a language this does not know.
    let by_name = |year: &str| {
        format!(
            r"\b(\d{{1,2}})[ .-]{{0,2}}(\p{{L}}{{3,11}})\.?(?:\s?/\s?(\p{{L}}{{3,11}})\.?)?[ .,-]{{0,2}}{year}"
        )
    };
    vec![
        // 2030-11-26
        pattern(r"\b((?:19|20)\d{2})[./-](\d{1,2})[./-](\d{1,2})\b", |m| {
            Some(Form::Plain {
                year: year(&m[1])?,
                month: figure(&m[2])?,
                day: figure(&m[3])?,
            })
        }),
        pattern(&in_figures(FULL), figures),
        // 26 11 2030
        pattern(r"\b(\d{2}) (\d{2}) ((?:19|20)\d{2})\b", figures),
        pattern(&by_name(FULL), named),
        // Aug 17, 1958
        pattern(r"\b(\p{L}{3,11})\.? (\d{1,2}),? ((?:19|20)\d{2})\b", |m| {
            Some(Form::Plain {
                month: month(&m[1])?,
                day: figure(&m[2])?,
                year: year(&m[3])?,
            })
        }),
        pattern(&in_figures(SHORT), figures),
        pattern(&by_name(SHORT), named),
    ]
});

fn named(m: &Captures<'_>) -> Option<Form> {
    Some(Form::Plain {
        day: figure(&m[1])?,
        month: month(&m[2]).or_else(|| month(m.get(3)?.as_str()))?,
        year: year(&m[4])?,
    })
}

fn figures(m: &Captures<'_>) -> Option<Form> {
    Some(Form::Figures {
        first: figure(&m[1])?,
        second: figure(&m[2])?,
        year: year(&m[3])?,
    })
}

fn figure(digits: &str) -> Option<u32> {
    digits.parse().ok()
}

/// Every date in a line, left to right. Figures that are no day of any year
/// ("31.02.2020", "00/00/0000") are not among them.
pub(super) fn find(line: &str) -> Vec<Printed> {
    let mut found: Vec<Printed> = Vec::new();
    for (pattern, read) in PATTERNS.iter() {
        for m in pattern.captures_iter(line) {
            let Some(form) = read(&m) else { continue };
            // From the first figure or letter to the last: a pattern may
            // look at the mark after a date without that being part of it.
            let parts = || m.iter().skip(1).flatten();
            let date = Printed {
                start: parts().map(|part| part.start()).min().unwrap_or_default(),
                end: parts().map(|part| part.end()).max().unwrap_or_default(),
                form,
            };
            let real = date.on(Order::DayFirst).is_some() || date.on(Order::MonthFirst).is_some();
            let free = found
                .iter()
                .all(|d| date.end <= d.start || date.start >= d.end);
            if real && free {
                found.push(date);
            }
        }
    }
    found.sort_by_key(|d| d.start);
    found
}

/// How the document writes its dates, by the ones that can only be read one
/// way. With none of those to go by it is day first, as nearly everywhere —
/// unless the document is one of the United States', which says so.
pub(super) fn order<'a>(dates: impl IntoIterator<Item = &'a Printed>, american: bool) -> Order {
    let (mut day_first, mut month_first) = (0, 0);
    for date in dates {
        if let Form::Figures { first, second, .. } = date.form {
            if first > 12 && second <= 12 {
                day_first += 1;
            } else if second > 12 && first <= 12 {
                month_first += 1;
            }
        }
    }
    if month_first > day_first || (month_first == day_first && american) {
        Order::MonthFirst
    } else {
        Order::DayFirst
    }
}

impl Printed {
    /// The day this is, on a document that writes its dates in `order` — or
    /// in the other order, where this one date leaves no choice. `None` for
    /// figures that are no day of any year.
    ///
    /// A year printed with two digits is read as late as it can be — right
    /// for an expiry; see [`Printed::past`] for a date that cannot lie ahead.
    pub fn on(&self, order: Order) -> Option<NaiveDate> {
        match self.form {
            Form::Plain { day, month, year } => NaiveDate::from_ymd_opt(year.latest(), month, day),
            Form::Figures {
                first,
                second,
                year,
            } => {
                let (day, month) = match order {
                    Order::DayFirst => (first, second),
                    Order::MonthFirst => (second, first),
                };
                NaiveDate::from_ymd_opt(year.latest(), month, day)
                    .or_else(|| NaiveDate::from_ymd_opt(year.latest(), day, month))
            }
        }
    }

    /// The day this is when it cannot be ahead of `today` — a birth, an
    /// issue. A two-digit year that would put it there is the last century's:
    /// "30" on a birth date is 1930, whatever it is on an expiry.
    pub fn past(&self, order: Order, today: NaiveDate) -> Option<NaiveDate> {
        let date = self.on(order)?;
        let (Form::Plain { year, .. } | Form::Figures { year, .. }) = self.form;
        match year {
            Year::Short(_) if date > today => date.with_year(date.year() - 100),
            _ => Some(date),
        }
    }
}

/// How a date is kept on an entry.
pub(super) fn iso(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

// How far ahead a two-digit year may be read: a document still to expire
// runs out in this century, and anything further ahead than one lasts was the
// last century's.
const FURTHEST_EXPIRY_YEARS: i32 = 20;

impl Year {
    /// The latest year this can be.
    fn latest(self) -> i32 {
        match self {
            Year::Full(year) => year,
            Year::Short(year) => {
                let now = chrono::Utc::now().year();
                let this_century = now - now % 100 + year;
                if this_century > now + FURTHEST_EXPIRY_YEARS {
                    this_century - 100
                } else {
                    this_century
                }
            }
        }
    }
}

fn year(digits: &str) -> Option<Year> {
    let year: i32 = digits.parse().ok()?;
    match digits.len() {
        4 => (1900..=2100).contains(&year).then_some(Year::Full(year)),
        _ => Some(Year::Short(year)),
    }
}

/// The months by name, January first, in the languages documents are
/// commonly printed in — as `words` reduces them: capitals, no marks.
const MONTHS: [&[&str]; 12] = [
    &[
        "JANUARY", "JANVIER", "JANUAR", "ENERO", "GENNAIO", "JANEIRO", "JANUARI", "STYCZEN",
    ],
    &[
        "FEBRUARY",
        "FEVRIER",
        "FEBRUAR",
        "FEBRERO",
        "FEBBRAIO",
        "FEVEREIRO",
        "FEBRUARI",
        "LUTY",
    ],
    &[
        "MARCH", "MARS", "MARZ", "MARZO", "MARCO", "MAART", "MARZEC", "MARTS",
    ],
    &["APRIL", "AVRIL", "ABRIL", "APRILE", "KWIECIEN"],
    &["MAY", "MAI", "MAYO", "MAGGIO", "MAIO", "MEI", "MAJ"],
    &[
        "JUNE", "JUIN", "JUNI", "JUNIO", "GIUGNO", "JUNHO", "CZERWIEC",
    ],
    &[
        "JULY", "JUILLET", "JULI", "JULIO", "LUGLIO", "JULHO", "LIPIEC",
    ],
    &[
        "AUGUST", "AOUT", "AGOSTO", "AUGUSTUS", "AUGUSTI", "SIERPIEN",
    ],
    &[
        "SEPTEMBER",
        "SEPTEMBRE",
        "SEPTIEMBRE",
        "SETTEMBRE",
        "SETEMBRO",
        "WRZESIEN",
    ],
    &[
        "OCTOBER",
        "OCTOBRE",
        "OKTOBER",
        "OCTUBRE",
        "OTTOBRE",
        "OUTUBRO",
        "PAZDZIERNIK",
    ],
    &["NOVEMBER", "NOVEMBRE", "NOVIEMBRE", "NOVEMBRO", "LISTOPAD"],
    &[
        "DECEMBER",
        "DECEMBRE",
        "DEZEMBER",
        "DICIEMBRE",
        "DICEMBRE",
        "DEZEMBRO",
        "DESEMBER",
        "GRUDZIEN",
    ],
];

/// The abbreviations in print that are not a name's first three letters.
const ABBREVIATIONS: [(&str, u32); 6] = [
    ("JANV", 1),
    ("FEVR", 2),
    ("MRZ", 3),
    ("MRT", 3),
    ("JUIL", 7),
    ("SEPT", 9),
];

/// The month a word names: a month's name in full, or its abbreviation — the
/// name's first three letters, where no other month opens with the same
/// three, or one of the few that print otherwise.
///
/// The whole word, not its opening: a word that merely starts like a month
/// is somebody's name. Janice is not January, nor Maria March.
fn month(word: &str) -> Option<u32> {
    let key: String = crate::scan::words::keys(word).concat();
    let numbered = || MONTHS.iter().zip(1u32..);
    if let Some((_, month)) = numbered().find(|(names, _)| names.contains(&key.as_str())) {
        return Some(month);
    }
    if let Some((_, month)) = ABBREVIATIONS.iter().find(|(short, _)| *short == key) {
        return Some(*month);
    }
    if key.chars().count() != 3 {
        return None;
    }
    let mut opening = numbered()
        .filter(|(names, _)| names.iter().any(|name| name.starts_with(key.as_str())))
        .map(|(_, month)| month);
    let only = opening.next()?;
    opening.next().is_none().then_some(only)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(line: &str, order: Order) -> Vec<String> {
        find(line)
            .iter()
            .filter_map(|d| d.on(order))
            .map(iso)
            .collect()
    }

    #[test]
    fn reads_the_ways_a_date_is_printed() {
        let day_first = |line| read(line, Order::DayFirst);
        assert_eq!(day_first("GULTIG BIS 10.02.2021"), ["2021-02-10"]);
        assert_eq!(day_first("Date of birth 12/08/1974"), ["1974-08-12"]);
        assert_eq!(day_first("2030-11-26"), ["2030-11-26"]);
        assert_eq!(day_first("12 08 1974"), ["1974-08-12"]);
        assert_eq!(day_first("17 AUG 1958 M"), ["1958-08-17"]);
        assert_eq!(day_first("12 AUG/AOUT 74"), ["1974-08-12"]);
        assert_eq!(day_first("3. März 2020"), ["2020-03-03"]);
        assert_eq!(day_first("09 XYZ/MAR 2014"), ["2014-03-09"]);
        assert_eq!(day_first("Aug 17, 1958"), ["1958-08-17"]);
        assert_eq!(
            day_first("WIEN 10.02.2016 bis 09.02.2021"),
            ["2016-02-10", "2021-02-09"]
        );
    }

    #[test]
    fn leaves_what_only_looks_like_a_date() {
        for line in [
            "5 DD 00/00/0000NNNAN/ANFD/YY",
            "SACRAMENTO, CA 00000-1234",
            "000-000-318 IR1",
            "2 JANICE 668",
            "16 HGT 5'-06\" 19 HAIR 17 WGT 160 lb",
            "31.02.2020",
            "9. A, B, C",
            // A word that opens like a month is a name.
            "2 JANICE 68",
            "12 MARIA 1964",
            "5 OTTO 2020",
            "7 ABRAHAM 99",
            // Three letters two months share say neither.
            "1 JUI 2020",
            // A field number and a date are not one longer date.
            "48. 23.03",
        ] {
            assert_eq!(read(line, Order::DayFirst), Vec::<String>::new(), "{line}");
        }
    }

    #[test]
    fn a_month_is_its_whole_name_or_its_abbreviation() {
        let day_first = |line| read(line, Order::DayFirst);
        assert_eq!(day_first("3 OTT 2020"), ["2020-10-03"]);
        assert_eq!(day_first("3 ottobre 2020"), ["2020-10-03"]);
        assert_eq!(day_first("14 JUIL 2021"), ["2021-07-14"]);
        assert_eq!(day_first("14 juillet 2021"), ["2021-07-14"]);
        assert_eq!(day_first("1 SEPT 2019"), ["2019-09-01"]);
        assert_eq!(day_first("9 września 2019"), Vec::<String>::new());
        assert_eq!(day_first("9 WRZ 2019"), ["2019-09-09"]);
        assert_eq!(day_first("11 Dezember 2022"), ["2022-12-11"]);
    }

    #[test]
    fn a_year_in_full_is_read_before_a_year_in_two_digits() {
        // Field 3 set against the date it captions.
        assert_eq!(read("3.12.08.1964 BERLIN", Order::DayFirst), ["1964-08-12"]);
        assert_eq!(read("4a.01.02.2015", Order::DayFirst), ["2015-02-01"]);
        // The same in two digits: the year is what ends the run of figures.
        assert_eq!(read("3.12.08.64", Order::DayFirst), ["1964-08-12"]);
        assert_eq!(read("Born 12.08.64.", Order::DayFirst), ["1964-08-12"]);
        assert_eq!(read("12 AUG 74, Utopia", Order::DayFirst), ["1974-08-12"]);
    }

    #[test]
    fn a_figure_before_a_date_does_not_hide_it() {
        assert_eq!(
            read("48. 23.03.2028 48, 012345678", Order::DayFirst),
            ["2028-03-23"]
        );
    }

    #[test]
    fn one_unambiguous_date_says_how_the_document_writes_them_all() {
        let dates: Vec<Printed> = ["4b EXP 11/26/2030", "4a ISS 11/09/2025"]
            .iter()
            .flat_map(|line| find(line))
            .collect();
        let order = order(&dates, false);
        assert_eq!(order, Order::MonthFirst);
        assert_eq!(iso(dates[1].on(order).unwrap()), "2025-11-09");
    }

    #[test]
    fn figures_that_could_be_either_are_day_first_except_in_america() {
        let dates = find("05/06/2020");
        assert_eq!(order(&dates, false), Order::DayFirst);
        assert_eq!(order(&dates, true), Order::MonthFirst);
        // A date that can only be read one way is read that way regardless.
        assert_eq!(read("02/21/1984", Order::DayFirst), ["1984-02-21"]);
    }

    #[test]
    fn two_digit_years_land_in_the_century_they_can_be_in() {
        assert_eq!(read("12 AUG 74", Order::DayFirst), ["1974-08-12"]);
        assert_eq!(read("01.01.10", Order::DayFirst), ["2010-01-01"]);
    }

    #[test]
    fn a_two_digit_year_is_a_century_back_on_a_date_that_cannot_be_ahead() {
        let today = chrono::Utc::now().date_naive();
        let ahead = today.year() + 4;
        let date = find(&format!("01/02/{:02}", ahead % 100))[0];
        // As late as it can be, for an expiry; behind today, for a birth.
        assert_eq!(date.on(Order::DayFirst).unwrap().year(), ahead);
        assert_eq!(
            date.past(Order::DayFirst, today).unwrap().year(),
            ahead - 100
        );
        // Four digits say which century themselves.
        let full = find(&format!("01/02/{ahead}"))[0];
        assert_eq!(full.past(Order::DayFirst, today).unwrap().year(), ahead);
    }
}
