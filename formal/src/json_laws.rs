// Kani formal verification harnesses for json_laws.

#[cfg(kani)]
use verity_core::json::{
    decode_utf16_surrogate_pair, parse_hex4_bytes, validate_json_number_bytes,
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

/// Law: Strict RFC 8259 number byte validation is panic-free on all bounded
/// byte slices (len 0..=8), rejects malformed forms, and accepts valid
/// integers, negatives, decimals, and scientific notation.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(10)]
pub fn prove_json_number_syntax_laws() {
    let buf: [u8; 8] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 8);
    let bytes = &buf[..len];

    let valid = validate_json_number_bytes(bytes);

    if len == 0 {
        assert!(!valid);
    }

    if len >= 2 && bytes[0] == b'0' && bytes[1].is_ascii_digit() {
        assert!(!valid);
    }
    if len >= 3 && bytes[0] == b'-' && bytes[1] == b'0' && bytes[2].is_ascii_digit() {
        assert!(!valid);
    }

    if len >= 1 {
        let first = bytes[0];
        let last = bytes[len - 1];
        if matches!(last, b'.' | b'e' | b'E' | b'+' | b'-') {
            assert!(!valid);
        }
        if valid {
            assert!(first == b'-' || first.is_ascii_digit());
            assert!(last.is_ascii_digit());
        }
    }

    let mut has_dot = false;
    let mut has_exp = false;
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        if valid {
            assert!(matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'));
        }
        if b == b'.' {
            has_dot = true;
        }
        if b == b'e' || b == b'E' {
            has_exp = true;
        }
        i += 1;
    }

    // Valid classes
    kani::cover!(
        valid && len >= 1 && bytes[0] != b'-' && !has_dot && !has_exp,
        "number_valid_integer"
    );
    kani::cover!(
        valid && len >= 2 && bytes[0] == b'-' && !has_dot && !has_exp,
        "number_valid_negative"
    );
    kani::cover!(valid && has_dot && !has_exp, "number_valid_decimal");
    kani::cover!(
        valid && bytes == b"1.2e-6",
        "number_valid_scientific_1_2e_6"
    );

    // Rejected classes
    kani::cover!(!valid && len == 0, "number_reject_empty");
    kani::cover!(!valid && bytes == b"01", "number_reject_leading_zero");
    kani::cover!(!valid && bytes == b"1.", "number_reject_dangling_dot");
    kani::cover!(!valid && bytes == b"1e", "number_reject_dangling_exp");
    kani::cover!(!valid && bytes == b"1e+", "number_reject_dangling_exp_plus");
    kani::cover!(
        !valid && bytes == b"1e-",
        "number_reject_dangling_exp_minus"
    );
}
