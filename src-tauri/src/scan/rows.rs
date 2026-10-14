//! Lines out of the regions a recognizer finds.
//!
//! Vision hands back one observation per region of text it detected, not one
//! per printed line, and a long run of MRZ filler is just the kind of gap that
//! makes it cut a line in two: `P<UTOERIKSSON<<ANNA<MARIA` in one box and the
//! rest of the fillers in another. Neither half has the width of an MRZ line,
//! so the parser never tried either, and the digits fell through to the card
//! parser. The boxes that share a horizontal band are put back together here.

use zeroize::Zeroize;

/// A region of recognized text, in Vision's normalized coordinates: origin at
/// the image's bottom-left corner, every value in `0..=1`. Its width is not
/// needed: the left edge orders regions within a line, the vertical extent
/// assigns them to one.
///
/// The text is a piece of the document in the clear, so it is scrubbed when
/// the region is dropped — on every path, which is what a `Drop` buys over
/// zeroizing at the one place a region is normally consumed.
pub(crate) struct Region {
    pub x: f64,
    pub y: f64,
    pub h: f64,
    pub text: String,
}

impl Drop for Region {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}

/// The lines the regions make, top to bottom, each the texts of its regions
/// left to right with a space between. The parsers already allow for the
/// space: OCR breaks a fixed-width field at the fillers anyway.
///
/// Two regions share a line when their vertical extents overlap by at least
/// half the shorter one's height. A region is measured against the first of
/// its line, not against a growing union, so a skewed photo cannot chain one
/// line into the next.
///
/// A line of one region keeps that region's own string; a joined line is a
/// new string and its parts go to the regions' `Drop`. The strings returned
/// are the caller's to zeroize, as `scan` does.
pub(crate) fn rows(mut regions: Vec<Region>) -> Vec<String> {
    regions.sort_by(|a, b| (b.y + b.h).total_cmp(&(a.y + a.h)));

    let mut lines: Vec<Vec<Region>> = Vec::new();
    for region in regions {
        match lines.last_mut() {
            Some(line) if same_band(&line[0], &region) => line.push(region),
            _ => lines.push(vec![region]),
        }
    }

    lines
        .into_iter()
        .map(|mut line| {
            if line.len() == 1 {
                return std::mem::take(&mut line[0].text);
            }
            line.sort_by(|a, b| a.x.total_cmp(&b.x));
            line.iter()
                .map(|r| r.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

fn same_band(a: &Region, b: &Region) -> bool {
    let overlap = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    overlap >= 0.5 * a.h.min(b.h)
}
