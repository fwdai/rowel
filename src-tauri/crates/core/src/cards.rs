//! Card network detection from the leading digits (IIN/BIN ranges).
//!
//! The network is not a secret — every physical card wears its logo — so the
//! derived slug lives in the plaintext metadata column, letting the list show
//! brand marks without ever decrypting a payload. Mirrored in TypeScript at
//! `src/utils/cardBrand.ts` for the revealed detail view; keep the two in sync.

// Longest/most-specific ranges are tested before the broad ones (e.g. the
// 2221–2720 Mastercard block before UnionPay's bare 62, Discover's 6011
// before Maestro's 60x).
pub fn card_brand(number: &str) -> Option<&'static str> {
    let digits: String = number.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return None;
    }
    let p2: u32 = digits[..2].parse().ok()?;
    let p3: u32 = digits[..3].parse().ok()?;
    let p4: u32 = digits[..4].parse().ok()?;

    let brand = if digits.starts_with('4') {
        "visa"
    } else if (51..=55).contains(&p2) || (2221..=2720).contains(&p4) {
        "mastercard"
    } else if p2 == 34 || p2 == 37 {
        "amex"
    } else if p4 == 6011 || p2 == 65 || (644..=649).contains(&p3) {
        "discover"
    } else if (3528..=3589).contains(&p4) {
        "jcb"
    } else if (300..=305).contains(&p3) || p2 == 36 || p2 == 38 || p2 == 39 {
        "diners"
    } else if p2 == 62 {
        "unionpay"
    } else if p2 == 50 || (56..=58).contains(&p2) || p2 == 67 || p2 == 63 {
        "maestro"
    } else {
        return None;
    };
    Some(brand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_major_networks() {
        assert_eq!(card_brand("4111 1111 1111 1111"), Some("visa"));
        assert_eq!(card_brand("5500-0000-0000-0004"), Some("mastercard"));
        assert_eq!(card_brand("2221000000000009"), Some("mastercard")); // 2-series
        assert_eq!(card_brand("340000000000009"), Some("amex"));
        assert_eq!(card_brand("370000000000002"), Some("amex"));
        assert_eq!(card_brand("6011000000000004"), Some("discover"));
        assert_eq!(card_brand("6759649826438453"), Some("maestro"));
        assert_eq!(card_brand("3530111333300000"), Some("jcb"));
        assert_eq!(card_brand("36700102000000"), Some("diners"));
        assert_eq!(card_brand("6212345678901265"), Some("unionpay"));
    }

    #[test]
    fn specific_ranges_win_over_broad_ones() {
        // 6011 is Discover, not Maestro's 60x; 62 is UnionPay, not Maestro.
        assert_eq!(card_brand("6011 0000"), Some("discover"));
        assert_eq!(card_brand("6200 0000"), Some("unionpay"));
    }

    #[test]
    fn unknown_or_short_input_is_none() {
        assert_eq!(card_brand(""), None);
        assert_eq!(card_brand("411"), None); // too short to trust
        assert_eq!(card_brand("9999 9999"), None);
        assert_eq!(card_brand("no digits"), None);
    }
}
