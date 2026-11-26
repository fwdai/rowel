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
    Figures { first: u32, second: u32, year: i32 },
    /// The year first, or the month by name: nothing left to tell apart.
    Plain { day: u32, month: u32, year: i32 },
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
static PATTERNS: LazyLock<[(Regex, Reader); 5]> = LazyLock::new(|| {
    let pattern = |pattern: &str, read: Reader| (Regex::new(pattern).unwrap(), read);
    [
        // 2030-11-26
        pattern(r"\b((?:19|20)\d{2})[./-](\d{1,2})[./-](\d{1,2})\b", |m| {
            Some(Form::Plain {
                year: year(&m[1])?,
                month: figure(&m[2])?,
                day: figure(&m[3])?,
            })
        }),
        // 26.11.2030, 11/26/30
        pattern(r"\b(\d{1,2})[./-](\d{1,2})[./-](\d{4}|\d{2})\b", figures),
        // 26 11 2030
        pattern(r"\b(\d{2}) (\d{2}) ((?:19|20)\d{2})\b", figures),
        // 12 AUG 1974, 12 AUG/AOUT 74, 3. März 2020: a second month name after
        // a slash is the same month in the document's other language, and the
        // one to go by when the first is in a language this does not know.
        pattern(
            r"\b(\d{1,2})[ .-]{0,2}(\p{L}{3,9})\.?(?:\s?/\s?(\p{L}{3,9})\.?)?[ .,-]{0,2}(\d{4}|\d{2})\b",
            |m| {
                Some(Form::Plain {
                    day: figure(&m[1])?,
                    month: month(&m[2]).or_else(|| month(m.get(3)?.as_str()))?,
                    year: year(&m[4])?,
                })
            },
        ),
        // Aug 17, 1958
        pattern(r"\b(\p{L}{3,9})\.? (\d{1,2}),? ((?:19|20)\d{2})\b", |m| {
            Some(Form::Plain {
                month: month(&m[1])?,
                day: figure(&m[2])?,
                year: year(&m[3])?,
            })
        }),
    ]
});

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
            let whole = m.get(0).unwrap();
            let Some(form) = read(&m) else { continue };
            let date = Printed {
                start: whole.start(),
                end: whole.end(),
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
    pub fn on(&self, order: Order) -> Option<NaiveDate> {
        match self.form {
            Form::Plain { day, month, year } => NaiveDate::from_ymd_opt(year, month, day),
            Form::Figures {
                first,
                second,
                year,
            } => {
                let (day, month) = match order {
                    Order::DayFirst => (first, second),
                    Order::MonthFirst => (second, first),
                };
                NaiveDate::from_ymd_opt(year, month, day)
                    .or_else(|| NaiveDate::from_ymd_opt(year, day, month))
            }
        }
    }
}

/// How a date is kept on an entry.
pub(super) fn iso(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

// Four digits are the year; two are one within reach of today: a document
// still to expire runs out in this century, and anything further ahead than
// one lasts was the last century's.
const FURTHEST_EXPIRY_YEARS: i32 = 20;

fn year(text: &str) -> Option<i32> {
    let year: i32 = text.parse().ok()?;
    if text.len() == 4 {
        return (1900..=2100).contains(&year).then_some(year);
    }
    let now = chrono::Utc::now().year();
    let this_century = now - now % 100 + year;
    Some(if this_century > now + FURTHEST_EXPIRY_YEARS {
        this_century - 100
    } else {
        this_century
    })
}

// A month by the first letters of its name, in the languages documents are
// commonly printed in. Four letters are asked first, for the one pair three
// cannot tell apart: the French June and July.
fn month(name: &str) -> Option<u32> {
    let key: String = crate::scan::words::keys(name).concat();
    let starts = |prefixes: &[&str]| prefixes.iter().any(|p| key.starts_with(p));
    Some(match () {
        () if starts(&["JUIN"]) => 6,
        () if starts(&["JUIL"]) => 7,
        () if starts(&["JAN", "ENE", "GEN", "STY"]) => 1,
        () if starts(&["FEB", "FEV", "LUT"]) => 2,
        () if starts(&["MAR", "MRZ", "MAA", "MRT"]) => 3,
        () if starts(&["APR", "AVR", "ABR", "KWI"]) => 4,
        () if starts(&["MAY", "MAI", "MAG", "MEI", "MAJ"]) => 5,
        () if starts(&["JUN", "GIU", "CZE"]) => 6,
        () if starts(&["JUL", "LUG", "LIP"]) => 7,
        () if starts(&["AUG", "AOU", "AGO", "SIE"]) => 8,
        () if starts(&["SEP", "SET", "WRZ"]) => 9,
        () if starts(&["OCT", "OKT", "OTT", "OUT", "PAZ"]) => 10,
        () if starts(&["NOV", "LIS"]) => 11,
        () if starts(&["DEC", "DEZ", "DIC", "GRU"]) => 12,
        () => return None,
    })
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
            // A field number and a date are not one longer date.
            "48. 23.03",
        ] {
            assert_eq!(read(line, Order::DayFirst), Vec::<String>::new(), "{line}");
        }
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
}
