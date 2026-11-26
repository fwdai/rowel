//! Parser tests over text fixtures. No images: the OCR backends only turn a
//! picture into lines, and everything that can be wrong about a scan is wrong
//! about the lines.

use super::local::scan_lines;
use super::rows::{rows as lines_of, Region};
use super::{card, document, is_image, local_path, mrz};

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

// A name long enough to reach the edge of the line has no check digit, so a
// row that lost its last letters must not be padded into a shorter name.
#[test]
fn does_not_pad_a_name_that_reaches_the_edge() {
    let full = "P<UTOERIKSSON<<ANNA<MARIA<ELISABETH<CHRISTIN";
    assert_eq!(full.len(), 44);
    assert_eq!(
        mrz_of(&[full, TD3[1]])["name"],
        "ANNA MARIA ELISABETH CHRISTIN ERIKSSON"
    );
    assert!(mrz::parse(&rows(&[&full[..42], TD3[1]])).is_none());
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

// What the recognizer made of public specimen documents, noise and all.

const CALIFORNIA_LICENCE: [&str; 16] = [
    "CALIFORNIA DRIVER LICENSE",
    "USA",
    "0557 IS57355/383",
    "4d DLN I1234567 4b EXP 11/26/2030",
    "ЗDов 11/26/1957",
    "1 SAMPLE",
    "2 JANICE 668",
    "8 123 NORTH STREET",
    "SACRAMENTO, CA 00000-1234",
    "9 CLASS A",
    "OSAMPLE 9a END NONE",
    "12 REST NONE",
    "4a ISS 11/09/2025",
    "49S SC4SSC 5 DD 00/00/0000NNNAN/ANFD/YY",
    "15 SEX F 18 EYES BRN",
    "Janice fample 16 HGT 5'-06\" 19 HAIR 17 WGT 160 lb BRN",
];

// Captions over their values, a skewed photo that ran most of the card into
// one line, and a number that happens to pass the card check.
const DC_LICENCE: [&str; 14] = [
    "WASHINGTON, DC USA",
    "DL",
    "DRIVER LICENSE",
    "4d.DLN 4b.EPX",
    "A9999999 02/21/2018",
    "1.FAMILY NAME",
    "JONES",
    "9999999 02211984 #1234 8.ADDRESS WASHINGTON, DC 00000-0000 1234 COMMODORE JOSHUA BARNEY DRIVE, NE 2.GIVEN NAMES ANGELINA GABRIELA 15.SEX 5-02 16.HGT 17.WGT 120 18.EYES BRO 02/21/1984 3.DOB",
    "9.CLASS 4a.ISS",
    "02/17/2010",
    "9a.ENDORSEMENTS",
    "NONE V DONOR",
    "5.DD 12.RESTRICTIONS VETERAN",
    "12348757475974",
];

// The title and the Greek came out as noise; the numbered fields and the
// Latin lines under them did not.
const GREEK_LICENCE: [&str; 11] = [
    "АДЕІА ОДНГНЕНЕ ЕЛЛНNIКН АНМОКРАТІА",
    "•GR",
    "1. ЕAAHNIДA",
    "ELLINIDA",
    "2. ГЕОРГІА МАРІA",
    "GEORGIA RI",
    "3. 05.02 OНNA АTТІКНЕ",
    "4g. 23.08.2013 AOHNAS",
    "48. 23.03.2028 48, 012345678",
    "0800000e3",
    "9. A, B, C",
];

const AUSTRIAN_PERMIT: [&str; 13] = [
    "AUFENTHALTSTITEL A123456789",
    "NAME TOS",
    "KHAN",
    "SAJED ALI",
    "GULTIG BIS 10.02.2021",
    "AUSSTEL LUNGSORT UND-DATUM",
    "WIEN 10.02.2016",
    "ART DES AUFENTHALTSTITELS",
    "DAUERAUFENTHALT - EU",
    "SONSTIGE ANGABEN",
    "KEINE",
    "CAN 08503690",
    "UNTERSCHRIFT",
];

// A passport's data page with its MRZ out of the picture: every caption in
// three languages, above its value, two to a line.
const DUTCH_PASSPORT_PAGE: [&str; 15] = [
    "PASSPORT PASPOORT KONINKRIJK DER NEDERLANDEN",
    "туре code 2 nationaliteit / nationality / nationalité documentnummer / document no. / no. du document",
    "P NLD Nederlandse SPEC 2014",
    "3 näam / surname / nom",
    "De Bruijn",
    "e/v Molenaar",
    "4 voornamen / given names / prenoms",
    "Willeke Liselotte",
    "-5 geboortedatum / date of birth / date de naissance",
    "10 MAA/MAR 1965",
    "6 geboorteplaats / place of birth / lieu de naissance",
    "/ geslacht/ sex / sexe 8 lengte / height / taille",
    "V/F 1,75 m",
    "9 datum van afgifte / date of issue / date de délivrance 10 geldig tot / date of expiry / date d'expiration",
    "09 MAA/MAR 2014 09 MAA/MAR 2024",
];

fn identity_of(lines: &[&str]) -> std::collections::BTreeMap<String, String> {
    let found = scan_lines(&rows(lines)).expect("a document should read");
    assert_eq!(found.kind, "identity");
    found.fields
}

fn type_of(lines: &[&str]) -> Option<String> {
    scan_lines(&rows(lines))
        .filter(|found| found.kind == "identity")?
        .fields
        .get("doc_type")
        .cloned()
}

fn fields_of(lines: &[&str]) -> Vec<(String, String)> {
    identity_of(lines).into_iter().collect()
}

fn pairs(fields: &[(&str, &str)]) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn reads_a_licence_captioned_by_number() {
    assert_eq!(
        fields_of(&CALIFORNIA_LICENCE),
        pairs(&[
            ("birth_date", "1957-11-26"),
            ("doc_type", "driver_license"),
            ("expiry_date", "2030-11-26"),
            ("issue_date", "2025-11-09"),
            ("name", "JANICE SAMPLE"),
            ("number", "I1234567"),
            ("sex", "F"),
        ])
    );
}

#[test]
fn reads_values_printed_under_their_captions() {
    assert_eq!(
        fields_of(&DC_LICENCE),
        pairs(&[
            // Under no caption that could be read: placed by being the
            // earliest of the three.
            ("birth_date", "1984-02-21"),
            ("doc_type", "driver_license"),
            ("expiry_date", "2018-02-21"),
            ("issue_date", "2010-02-17"),
            ("name", "ANGELINA GABRIELA JONES"),
            ("number", "A9999999"),
        ])
    );
}

#[test]
fn reads_what_survives_of_a_document_in_another_script() {
    assert_eq!(
        fields_of(&GREEK_LICENCE),
        pairs(&[
            ("doc_type", "driver_license"),
            // Two dates fifteen years apart are an issue and an expiry, not
            // a birth.
            ("expiry_date", "2028-03-23"),
            ("issue_date", "2013-08-23"),
            // The Latin line under each misread one.
            ("name", "GEORGIA RI ELLINIDA"),
        ])
    );
}

#[test]
fn reads_a_document_with_one_caption_for_the_whole_name() {
    assert_eq!(
        fields_of(&AUSTRIAN_PERMIT),
        pairs(&[
            ("doc_type", "residence_permit"),
            ("expiry_date", "2021-02-10"),
            ("issue_date", "2016-02-10"),
            ("name", "SAJED ALI KHAN"),
            ("number", "A123456789"),
        ])
    );
}

#[test]
fn reads_a_passport_page_by_its_print_when_the_mrz_is_out_of_frame() {
    assert_eq!(
        fields_of(&DUTCH_PASSPORT_PAGE),
        pairs(&[
            ("birth_date", "1965-03-10"),
            ("doc_type", "passport"),
            ("expiry_date", "2024-03-09"),
            ("issue_date", "2014-03-09"),
            ("name", "Willeke Liselotte De Bruijn"),
            ("sex", "F"),
        ])
    );
}

#[test]
fn the_print_adds_the_issue_date_an_mrz_does_not_carry() {
    let mut lines = vec![
        "Date of birth 12 AUG 1974",
        "Date of issue 16 APR 2007",
        "Date of expiry 15 APR 2012",
    ];
    lines.extend(TD3);
    let fields = identity_of(&lines);
    assert_eq!(fields["issue_date"], "2007-04-16");
    // Everything the MRZ says stays the MRZ's.
    assert_eq!(fields["birth_date"], "1974-08-12");
    assert_eq!(fields["expiry_date"], "2012-04-15");
    assert_eq!(fields["name"], "ANNA MARIA ERIKSSON");

    // A printed date that cannot be the issue date is not taken for one.
    let mut lines = vec!["Date of issue 16 APR 2031"];
    lines.extend(TD3);
    assert!(!identity_of(&lines).contains_key("issue_date"));
}

#[test]
fn a_captioned_birth_date_stands_however_soon_the_document_expires() {
    // A child's card: born ten years before it runs out, the date printed
    // under its caption.
    assert_eq!(
        fields_of(&[
            "IDENTITY CARD",
            "Date of birth",
            "01 JAN 2020",
            "Date of expiry 01 JAN 2030",
        ]),
        pairs(&[
            ("birth_date", "2020-01-01"),
            ("doc_type", "id_card"),
            ("expiry_date", "2030-01-01"),
        ])
    );
    // Beside its caption, the same.
    let beside = identity_of(&[
        "IDENTITY CARD",
        "Date of birth 01 JAN 2020",
        "Date of expiry 01 JAN 2030",
    ]);
    assert_eq!(beside["birth_date"], "2020-01-01");
    assert!(!beside.contains_key("issue_date"));
}

#[test]
fn a_two_digit_year_is_read_by_what_the_date_is() {
    use chrono::Datelike;
    // A year a few ahead of this one, printed in two digits: an expiry still
    // to come, and a birth a century back.
    let ahead = chrono::Utc::now().year() + 4;
    let lines = [
        "IDENTITY CARD".to_owned(),
        format!("Date of birth 01/02/{:02}", ahead % 100),
        format!("Date of expiry 03/04/{:02}", ahead % 100),
    ];
    let fields = scan_lines(&lines).unwrap().fields;
    assert_eq!(fields["birth_date"], format!("{}-02-01", ahead - 100));
    assert_eq!(fields["expiry_date"], format!("{ahead}-04-03"));
}

#[test]
fn a_date_is_month_first_only_where_america_issued_the_document() {
    // Figures that read either way, on a document that names the United
    // States at its head.
    assert_eq!(
        fields_of(&["DRIVER LICENSE", "USA", "DOB 02/10/1980", "EXP 03/11/2030"]),
        pairs(&[
            ("birth_date", "1980-02-10"),
            ("doc_type", "driver_license"),
            ("expiry_date", "2030-03-11"),
        ])
    );
    // The same country further down is the holder's, not the document's.
    assert_eq!(
        fields_of(&[
            "AUFENTHALTSTITEL",
            "Name MUSTER ERIKA",
            "Staatsangehörigkeit USA",
            "Geburtsdatum 10.02.1980",
            "Ausstellungsdatum 04.05.2020",
            "Gültig bis 11.03.2030",
        ]),
        pairs(&[
            ("birth_date", "1980-02-10"),
            ("doc_type", "residence_permit"),
            ("expiry_date", "2030-03-11"),
            ("issue_date", "2020-05-04"),
            ("name", "MUSTER ERIKA"),
        ])
    );
}

#[test]
fn knows_a_document_by_the_title_it_prints() {
    assert_eq!(
        type_of(&CALIFORNIA_LICENCE).as_deref(),
        Some("driver_license")
    );
    assert_eq!(
        type_of(&AUSTRIAN_PERMIT).as_deref(),
        Some("residence_permit")
    );
    assert_eq!(
        type_of(&[
            "BUNDESREPUBLIK DEUTSCHLAND",
            "PERSONALAUSWEIS / IDENTITY CARD"
        ])
        .as_deref(),
        Some("id_card")
    );
}

#[test]
fn reads_a_title_through_case_marks_and_line_breaks() {
    let printed = |lines: &[&str]| document::doc_type(&rows(lines));
    assert_eq!(printed(&["Führerschein"]), Some("driver_license"));
    assert_eq!(printed(&["DRIVER'S LICENSE"]), Some("driver_license"));
    assert_eq!(printed(&["Carte nationale d’identité"]), Some("id_card"));
    assert_eq!(printed(&["TITRE DE", "SÉJOUR"]), Some("residence_permit"));
    // Whole words only.
    assert_eq!(printed(&["VALID CARD", "MEMBER SINCE 2019"]), None);
    assert_eq!(printed(&["nothing a document says"]), None);
}

#[test]
fn a_licence_is_not_a_card_whatever_its_digits_add_up_to() {
    assert!(card::parse(&rows(&DC_LICENCE)).is_some());
    assert_eq!(type_of(&DC_LICENCE).as_deref(), Some("driver_license"));
}

#[test]
fn knows_a_licence_by_its_numbered_fields_when_the_title_is_lost() {
    assert_eq!(type_of(&GREEK_LICENCE).as_deref(), Some("driver_license"));
    // A numbered list is not one: it has a 4, never a 4a.
    assert_eq!(
        document::doc_type(&rows(&["1. Eggs", "2. Flour", "3. Milk", "4. Sugar"])),
        None
    );
}

#[test]
fn a_card_that_says_what_it_is_overrides_the_catch_all_mrz_code() {
    let mut lines = vec!["UNITED STATES OF AMERICA", "PERMANENT RESIDENT"];
    lines.extend(TD1);
    assert_eq!(type_of(&lines).as_deref(), Some("residence_permit"));
    // The MRZ still fills everything else.
    assert_eq!(identity_of(&lines)["number"], "D23145890");
    // And a passport stays one whatever else is printed near it.
    let mut lines = vec!["RESIDENCE PERMIT"];
    lines.extend(TD3);
    assert_eq!(type_of(&lines).as_deref(), Some("passport"));
}

#[test]
fn a_card_stays_a_card_and_passport_alone_does_not_claim_one() {
    let card = ["WORLD PASSPORT REWARDS", "4242 4242 4242 4242", "04/27"];
    assert_eq!(scan_lines(&rows(&card)).unwrap().kind, "card");
    assert_eq!(
        type_of(&["PASSPORT", "PASSEPORT"]).as_deref(),
        Some("passport")
    );
}
