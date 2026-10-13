//! Parser tests over text fixtures. No images: the OCR backends only turn a
//! picture into lines, and everything that can be wrong about a scan is wrong
//! about the lines.

use super::rows::{rows as lines_of, Region};
use super::{card, is_image, local_path, mrz, scan_lines};

// The ICAO 9303 specimen document (Utopia / ANNA MARIA ERIKSSON), the one set
// of MRZ lines whose every check digit is published.
const TD3: [&str; 2] = [
    "P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<",
    "L898902C36UTO7408122F1204159ZE184226B<<<<<10",
];

const TD2: [&str; 2] = [
    "I<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<",
    "D231458907UTO7408122F1204159<<<<<<<6",
];

const TD1: [&str; 3] = [
    "I<UTOD231458907<<<<<<<<<<<<<<<",
    "7408122F1204159UTO<<<<<<<<<<<6",
    "ERIKSSON<<ANNA<MARIA<<<<<<<<<<",
];

fn rows(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| (*l).to_owned()).collect()
}

fn mrz_of(lines: &[&str]) -> std::collections::BTreeMap<String, String> {
    mrz::parse(&rows(lines)).expect("MRZ should parse")
}

fn card_of(lines: &[&str]) -> std::collections::BTreeMap<String, String> {
    card::parse(&rows(lines)).expect("card should parse")
}

#[test]
fn reads_a_td3_passport() {
    let f = mrz_of(&TD3);
    assert_eq!(f["doc_type"], "passport");
    assert_eq!(f["name"], "ANNA MARIA ERIKSSON");
    assert_eq!(f["number"], "L898902C3");
    assert_eq!(f["country"], "UTO");
    assert_eq!(f["nationality"], "UTO");
    assert_eq!(f["birth_date"], "1974-08-12");
    assert_eq!(f["sex"], "F");
    assert_eq!(f["expiry_date"], "2012-04-15");
    assert_eq!(f["personal_number"], "ZE184226B");
}

#[test]
fn reads_a_td2_document() {
    let f = mrz_of(&TD2);
    assert_eq!(f["doc_type"], "id_card");
    assert_eq!(f["name"], "ANNA MARIA ERIKSSON");
    assert_eq!(f["number"], "D23145890");
    assert_eq!(f["birth_date"], "1974-08-12");
    assert_eq!(f["expiry_date"], "2012-04-15");
    assert_eq!(f["sex"], "F");
}

#[test]
fn reads_a_td1_id_card() {
    let f = mrz_of(&TD1);
    assert_eq!(f["doc_type"], "id_card");
    assert_eq!(f["name"], "ANNA MARIA ERIKSSON");
    assert_eq!(f["number"], "D23145890");
    assert_eq!(f["country"], "UTO");
    assert_eq!(f["nationality"], "UTO");
    assert_eq!(f["birth_date"], "1974-08-12");
    assert_eq!(f["expiry_date"], "2012-04-15");
    assert_eq!(f["sex"], "F");
}

// A residence permit is the same TD1 layout under a different document code.
#[test]
fn reads_the_document_code_as_the_type() {
    let permit = TD1[0].replacen("I<", "IR", 1);
    let f = mrz_of(&[&permit, TD1[1], TD1[2]]);
    assert_eq!(f["doc_type"], "residence_permit");
}

// What a real scan hands over: fillers misread as guillemets, spaces broken
// into the fixed-width fields, and `O` for `0` in the dates.
#[test]
fn reads_an_mrz_through_ocr_noise() {
    let l1 = TD3[0].replace('<', "«");
    let l2 = TD3[1]
        .replace('<', "«")
        .replace("740812", "74O812")
        .replace("120415", "12O415")
        .replace("UTO", "UTO ");
    let f = mrz_of(&["Some heading the camera also caught", &l1, &l2]);
    assert_eq!(f["name"], "ANNA MARIA ERIKSSON");
    assert_eq!(f["number"], "L898902C3");
    assert_eq!(f["birth_date"], "1974-08-12");
    assert_eq!(f["expiry_date"], "2012-04-15");
}

// A document number that does not match its own check digit fails the composite
// too, and is not a document.
#[test]
fn rejects_a_bad_check_digit() {
    let tampered = TD3[1].replacen("L898902C3", "L898902C4", 1);
    assert!(mrz::parse(&rows(&[TD3[0], &tampered])).is_none());
}

// Every individual field verifying is enough on its own: some issuers get the
// composite wrong, and OCR only has to misread one character of the optional
// data to invalidate it.
#[test]
fn accepts_a_document_whose_only_bad_digit_is_the_composite() {
    let tampered = format!("{}5", &TD3[1][..43]);
    let f = mrz_of(&[TD3[0], &tampered]);
    assert_eq!(f["number"], "L898902C3");
}

#[test]
fn ignores_lines_that_are_not_an_mrz() {
    assert!(mrz::parse(&rows(&["hello", "", "1234"])).is_none());
    // Right shape, wrong width.
    assert!(mrz::parse(&rows(&["P<UTOERIKSSON<<ANNA", "L898902C36UTO"])).is_none());
}

// OCR drops the tail of a long filler run. A line that is a filler or three
// short is read with them back; the check digits still guard the data line.
#[test]
fn reads_a_line_that_lost_its_trailing_filler() {
    let short = &TD3[0][..41];
    let f = mrz_of(&[short, TD3[1]]);
    assert_eq!(f["name"], "ANNA MARIA ERIKSSON");
    assert_eq!(f["number"], "L898902C3");

    let short = &TD1[2][..28];
    let f = mrz_of(&[TD1[0], TD1[1], short]);
    assert_eq!(f["name"], "ANNA MARIA ERIKSSON");
}

#[test]
fn does_not_pad_a_line_that_lost_more_than_filler() {
    assert!(mrz::parse(&rows(&[&TD3[0][..40], TD3[1]])).is_none());
    // The data line's tail is its check digits, not filler: padded back, the
    // arithmetic says no.
    assert!(mrz::parse(&rows(&[TD3[0], &TD3[1][..42]])).is_none());
}

#[test]
fn tells_an_mrz_from_other_print_by_its_filler() {
    assert!(mrz::present(&rows(&[TD3[0]])));
    assert!(mrz::present(&rows(&["<<<<<<<<<<<<<<<<<<<"])));
    assert!(mrz::present(&rows(&[&TD3[0].replace('<', "«")])));
    assert!(!mrz::present(&rows(&[
        "JOHN A SMITH",
        "4111 1111 1111 1111"
    ])));
    assert!(!mrz::present(&rows(&["a < b", "x << y"])));
}

// Vision cuts a line at a wide gap, and a name line's filler is one. The two
// boxes share a band, so they are one line again — in reading order, whatever
// order they were found in.
#[test]
fn rebuilds_a_line_the_recognizer_split_at_the_filler() {
    let region = |x: f64, y: f64, text: &str| Region {
        x,
        y,
        h: 0.05,
        text: text.into(),
    };
    let lines = lines_of(vec![
        region(0.1, 0.10, "L898902C36UTO7408122F1204159ZE184226B<<<<<10"),
        region(0.6, 0.17, "<<<<<<<<<<<<<<<<<<<"),
        region(0.1, 0.80, "PASSPORT"),
        region(0.1, 0.16, "P<UTOERIKSSON<<ANNA<MARIA"),
    ]);
    assert_eq!(
        lines,
        [
            "PASSPORT",
            "P<UTOERIKSSON<<ANNA<MARIA <<<<<<<<<<<<<<<<<<<",
            "L898902C36UTO7408122F1204159ZE184226B<<<<<10",
        ]
    );
    let f = mrz_of(&lines.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(f["name"], "ANNA MARIA ERIKSSON");
}

// The two MRZ lines sit close, but not half a line's height into each other.
#[test]
fn keeps_adjacent_lines_apart() {
    let region = |y: f64, text: &str| Region {
        x: 0.1,
        y,
        h: 0.05,
        text: text.into(),
    };
    let lines = lines_of(vec![region(0.10, "second"), region(0.16, "first")]);
    assert_eq!(lines, ["first", "second"]);
}

#[test]
fn reads_a_card_number_written_in_groups() {
    assert_eq!(
        card_of(&["4111 1111 1111 1111"])["number"],
        "4111111111111111"
    );
    assert_eq!(
        card_of(&["5500-0000-0000-0004"])["number"],
        "5500000000000004"
    );
}

// Embossed digits come back as the letters they look like.
#[test]
fn reads_a_card_number_through_ocr_confusions() {
    assert_eq!(
        card_of(&["4O12 8888 8888 l88l"])["number"],
        "4012888888881881"
    );
    assert_eq!(
        card_of(&["5S00 0000 0000 0004"])["number"],
        "5500000000000004"
    );
}

// One digit off is a different number, not this one. The Luhn-valid 13-digit
// tail inside it is not a card either: it belongs to no network.
#[test]
fn rejects_a_number_that_fails_luhn() {
    assert!(card::parse(&rows(&["4111 1111 1111 1112"])).is_none());
    assert!(card::parse(&rows(&["JOHN SMITH", "VALID THRU 03/26"])).is_none());
}

#[test]
fn reads_every_way_an_expiry_is_printed() {
    for printed in ["03/26", "03/2026", "03-26", "03 26"] {
        let f = card_of(&["4111 1111 1111 1111", printed]);
        assert_eq!(
            (f["month"].as_str(), f["year"].as_str()),
            ("03", "26"),
            "{printed}"
        );
    }
}

// A card printing both dates puts "valid from" first; the later one is the
// expiry.
#[test]
fn takes_the_latest_of_two_dates() {
    let f = card_of(&["4111 1111 1111 1111", "01/22 03/26"]);
    assert_eq!((f["month"].as_str(), f["year"].as_str()), ("03", "26"));
}

// The number's own quads must not read as a date.
#[test]
fn has_no_expiry_when_the_card_shows_none() {
    let f = card_of(&["4111 1111 1111 1111"]);
    assert_eq!(f["month"], "");
    assert_eq!(f["year"], "");
}

#[test]
fn picks_the_cardholder_out_of_the_print() {
    let f = card_of(&[
        "VISA",
        "DEBIT",
        "4111 1111 1111 1111",
        "VALID THRU 03/26",
        "JOHN A SMITH",
        "SOME BANK PLC",
    ]);
    assert_eq!(f["name"], "JOHN A SMITH");
}

#[test]
fn has_no_cardholder_when_no_line_reads_like_one() {
    assert_eq!(card_of(&["4111 1111 1111 1111", "VISA DEBIT"])["name"], "");
}

// An identity document can carry a Luhn-valid number by chance; a verified MRZ
// cannot be anything but what it is, so it decides.
#[test]
fn scan_lines_prefers_a_verified_mrz() {
    let found = scan_lines(&rows(&["4111 1111 1111 1111", TD3[0], TD3[1]])).unwrap();
    assert_eq!(found.kind, "identity");
    assert_eq!(found.fields["number"], "L898902C3");
}

// A passport whose optional data is numeric carries a run of digits long
// enough to hold a card number by chance. When its check digits fail — a
// misread, a dropped filler — it is a worse photo of a passport, not a card.
#[test]
fn scan_lines_does_not_read_an_unverified_mrz_as_a_card() {
    // The specimen's optional data and both check digits replaced by a
    // Luhn-valid Visa number: 44 characters, every check digit wrong.
    let data = format!("{}4111111111111111", &TD3[1][..28]);
    let lines = rows(&[TD3[0], &data]);
    assert!(mrz::parse(&lines).is_none());
    assert_eq!(card::parse(&lines).unwrap()["number"], "4111111111111111");
    assert!(scan_lines(&lines).is_none());
}

#[test]
fn scan_lines_reads_a_card_when_there_is_no_mrz() {
    let found = scan_lines(&rows(&["JOHN A SMITH", "4111 1111 1111 1111", "03/26"])).unwrap();
    assert_eq!(found.kind, "card");
    assert_eq!(found.fields["number"], "4111111111111111");
    assert_eq!(found.fields["name"], "JOHN A SMITH");
}

#[test]
fn scan_lines_finds_nothing_in_plain_text() {
    assert!(scan_lines(&rows(&[
        "Shopping list",
        "milk, eggs",
        "call mum on 555 1234"
    ]))
    .is_none());
}

// The iOS picker hands back the sandbox copy as a `file://` URL; every other
// source gives a plain path, and a Windows path must not be read as a URL.
#[test]
fn a_file_url_resolves_to_the_path_it_names() {
    assert_eq!(
        local_path("file:///var/mobile/Containers/Data/Application/Caches/IMG_0001.HEIC"),
        std::path::PathBuf::from("/var/mobile/Containers/Data/Application/Caches/IMG_0001.HEIC")
    );
}

#[test]
fn a_plain_path_is_left_alone() {
    for path in ["/Users/a/card.png", r"C:\Users\a\card.png", "card.png"] {
        assert_eq!(local_path(path), std::path::PathBuf::from(path));
    }
}

#[test]
fn takes_the_image_types_the_pickers_offer() {
    for path in [
        "/Users/a/card.png",
        "/Users/a/card.JPG",
        "/var/mobile/IMG_0001.HEIC",
        "photo.jpeg",
        "shot.webp",
        "scan.tiff",
        "scan.tif",
        "old.bmp",
        "loop.gif",
        "live.heif",
    ] {
        assert!(is_image(&local_path(path)), "{path}");
    }
}

// A path the scanner has no business opening never reaches the recognizer —
// nor, more to the point, the disk.
#[test]
fn refuses_anything_that_is_not_an_image() {
    for path in [
        "/Users/a/notes.txt",
        "/Users/a/.ssh/id_ed25519",
        "/etc/passwd",
        "card.png.txt",
        "card",
    ] {
        assert!(!is_image(&local_path(path)), "{path}");
    }
}

#[tokio::test]
async fn a_scan_of_a_non_image_is_refused() {
    let err = super::scan("/Users/a/notes.txt".into()).await.unwrap_err();
    assert!(matches!(err, crate::error::Error::Unrecognized), "{err}");
}
