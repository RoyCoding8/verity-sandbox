// Kani formal verification harnesses for json_laws.

#[cfg(kani)]
use verity_core::json::{
    classify_json_escape, classify_json_number, decode_utf16_surrogate_pair, parse_hex4_bytes,
    validate_json_number_bytes, JsonEscape, JsonNumberKind,
};

/// Law: UTF-16 surrogate pair decoding and 4-byte hex parsing are total,
/// panic-free, and exact for all 16-bit code units and 4-byte inputs.
#[cfg(kani)]
#[kani::proof]
pub fn prove_utf16_surrogate_pair_and_hex4_laws() {
    let hi: u16 = kani::any();
    let lo: u16 = kani::any();

    let decoded = decode_utf16_surrogate_pair(hi, lo);
    let hi_is_high = (0xD800..=0xDBFF).contains(&hi);
    let lo_is_low = (0xDC00..=0xDFFF).contains(&lo);
    let hi_is_low = (0xDC00..=0xDFFF).contains(&hi);
    let hi_is_bmp = hi < 0xD800 || hi > 0xDFFF;

    assert_eq!(decoded.is_some(), hi_is_high && lo_is_low);
    if let Some(ch) = decoded {
        let scalar = ch as u32;
        assert!((0x10000..=0x10FFFF).contains(&scalar));
        let expected = 0x10000 + ((((hi as u32) - 0xD800) << 10) | ((lo as u32) - 0xDC00));
        assert_eq!(scalar, expected);
    }

    kani::cover!(decoded.is_some(), "surrogate_valid_pair");
    kani::cover!(
        hi_is_high && !lo_is_low && decoded.is_none(),
        "surrogate_lone_high"
    );
    kani::cover!(hi_is_low && decoded.is_none(), "surrogate_lone_low");
    kani::cover!(hi_is_bmp && decoded.is_none(), "surrogate_bmp");

    let hex: [u8; 4] = kani::any();
    let parsed = parse_hex4_bytes(hex);
    let all_hex = hex[0].is_ascii_hexdigit()
        && hex[1].is_ascii_hexdigit()
        && hex[2].is_ascii_hexdigit()
        && hex[3].is_ascii_hexdigit();
    assert_eq!(parsed.is_some(), all_hex);

    if let Some(val) = parsed {
        let mut i = 0usize;
        while i < 4 {
            let nibble = ((val >> (12 - 4 * i)) & 0xF) as u8;
            let b = hex[i];
            let expected_nibble = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => unreachable!(),
            };
            assert_eq!(nibble, expected_nibble);
            i += 1;
        }
    }

    kani::cover!(parsed.is_some(), "hex4_valid");
    kani::cover!(parsed.is_none(), "hex4_invalid");
}

#[cfg(kani)]
fn decimal_digits(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

#[cfg(kani)]
fn json_number_grammar(bytes: &[u8]) -> bool {
    let mut exponent_parts = bytes.splitn(2, |&byte| matches!(byte, b'e' | b'E'));
    let mantissa = exponent_parts.next().unwrap_or_default();
    if let Some(exponent) = exponent_parts.next() {
        let digits = match exponent {
            [b'+' | b'-', digits @ ..] => digits,
            _ => exponent,
        };
        if !decimal_digits(digits) {
            return false;
        }
    }

    let mut mantissa_parts = mantissa.splitn(2, |&byte| byte == b'.');
    let integer = mantissa_parts.next().unwrap_or_default();
    let integer = match integer {
        [b'-', digits @ ..] => digits,
        _ => integer,
    };
    let integer_valid = match integer {
        b"0" => true,
        [b'1'..=b'9', rest @ ..] => rest.iter().all(u8::is_ascii_digit),
        _ => false,
    };
    integer_valid && mantissa_parts.next().is_none_or(decimal_digits)
}

/// Exact RFC 8259 number syntax over all byte slices of length zero to eight.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(10)]
pub fn prove_json_number_syntax_laws() {
    let buf: [u8; 8] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= buf.len());
    let bytes = &buf[..len];

    let valid = validate_json_number_bytes(bytes);
    assert_eq!(valid, json_number_grammar(bytes));

    kani::cover!(valid && bytes == b"0", "zero_accepted");
    kani::cover!(valid && bytes == b"-12", "negative_integer_accepted");
    kani::cover!(valid && bytes == b"1.2", "decimal_accepted");
    kani::cover!(valid && bytes == b"1.2e-6", "scientific_notation_accepted");
    kani::cover!(!valid && bytes == b"-01", "leading_zero_rejected");
    kani::cover!(
        !valid && bytes == b"1e+",
        "missing_exponent_digits_rejected"
    );
    kani::cover!(!valid && bytes == b"1.2.3", "second_decimal_point_rejected");
}
/// Law: JSON number serialization classification never emits non-finite floats
/// (all NaN/+Inf/-Inf bit patterns map to `NonFinite`), and string escape
/// classification never passes through control chars (< 0x20), `"`, or `\`.
#[cfg(kani)]
#[kani::proof]
pub fn prove_json_number_serialization_and_escape_laws() {
    // 1. All non-finite IEEE-754 bit patterns (sign=0/1, exp=0x7ff, any mantissa) -> NonFinite.
    let sign: u64 = kani::any();
    let mantissa: u64 = kani::any();
    let nonfinite_bits =
        ((sign & 1) << 63) | (0x7ff_u64 << 52) | (mantissa & 0x000f_ffff_ffff_ffff);
    let nonfinite = f64::from_bits(nonfinite_bits);
    assert!(!nonfinite.is_finite());
    assert_eq!(classify_json_number(nonfinite), JsonNumberKind::NonFinite);
    kani::cover!(nonfinite.is_nan(), "nonfinite_nan_classified");
    kani::cover!(
        nonfinite.is_infinite() && nonfinite.is_sign_positive(),
        "nonfinite_pos_inf_classified"
    );
    kani::cover!(
        nonfinite.is_infinite() && nonfinite.is_sign_negative(),
        "nonfinite_neg_inf_classified"
    );

    // 2. Bounded finite integers convert losslessly without overflow.
    let iv: i32 = kani::any();
    let int_f = iv as f64;
    assert_eq!(
        classify_json_number(int_f),
        JsonNumberKind::Integer(iv as i64)
    );
    assert_eq!(classify_json_number(-0.0), JsonNumberKind::Integer(0));

    // 3. Bounded non-integer finite values stay finite Float.
    let half_base: i16 = kani::any();
    let frac_f = (half_base as f64) + 0.5;
    match classify_json_number(frac_f) {
        JsonNumberKind::Float(f) => {
            assert!(f.is_finite());
        }
        _ => unreachable!("x + 0.5 for i16 must classify as finite Float"),
    }

    // Large finite integer at threshold >= 1e15 falls back to finite Float.
    assert_eq!(classify_json_number(1e15), JsonNumberKind::Float(1e15));

    // 4. Total escape classification across all Unicode scalar values `char`.
    let ch: char = kani::any();
    let esc = classify_json_escape(ch);
    if (ch as u32) < 0x20 || ch == '"' || ch == '\\' {
        assert!(!matches!(esc, JsonEscape::Passthrough));
    }
    match esc {
        JsonEscape::Static(s) => {
            assert!(s.as_bytes()[0] == b'\\');
            assert!(matches!(ch, '"' | '\\' | '\n' | '\r' | '\t'));
        }
        JsonEscape::UnicodeControl(b) => {
            assert!(b < 0x20);
            assert_eq!(b as u32, ch as u32);
        }
        JsonEscape::Passthrough => {
            assert!((ch as u32) >= 0x20 && ch != '"' && ch != '\\');
        }
    }

    kani::cover!(
        matches!(esc, JsonEscape::Static(_)),
        "escape_static_reachable"
    );
    kani::cover!(
        matches!(esc, JsonEscape::UnicodeControl(_)),
        "escape_control_reachable"
    );
    kani::cover!(
        matches!(esc, JsonEscape::Passthrough),
        "escape_passthrough_reachable"
    );
}
