// Copyright 2023 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! An exact decimal of any magnitude PostgreSQL's `numeric` holds.

use std::cmp::Ordering;
use std::fmt::Display;
use std::fmt::Formatter;

use crate::error::Error;
use crate::error::Result;

/// Most digits PostgreSQL's `numeric` keeps before the decimal point.
pub const NUMERIC_MAX_INTEGER_DIGITS: usize = 131_072;

/// Most digits PostgreSQL's `numeric` keeps after the decimal point.
pub const NUMERIC_MAX_FRACTION_DIGITS: usize = 16_383;

/// An exact decimal held as PostgreSQL's `numeric` output text: an optional
/// `-`, the integer digits without leading zeros (`0` when there are none),
/// and, when the display scale is positive, a `.` and exactly that many
/// fraction digits. Zero carries no sign. `numeric_in` gives a value the
/// display scale its spelling implies, `max(0, fraction digits - exponent)`,
/// so `1.50` keeps two fraction digits and `1.5e2` none.
#[derive(Debug, Clone)]
pub struct Numeric {
    text: Box<str>,
}

impl Numeric {
    /// Read a decimal spelling, `[+-]digits[.digits][(e|E)[+-]digits]` with
    /// at least one mantissa digit, as `numeric_in` reads it. NaN, the
    /// infinities and any magnitude beyond `numeric`'s range are refused.
    pub fn parse(text: &str) -> Result<Numeric> {
        let invalid = || Error::Message(format!("invalid numeric value: {text:?}"));
        let bytes = text.as_bytes();
        let mut position = 0;
        let mut negative = false;
        match bytes.first() {
            Some(b'-') => {
                negative = true;
                position += 1;
            }
            Some(b'+') => position += 1,
            _ => {}
        }
        let integer_start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        let integer = &text[integer_start..position];
        let mut fraction = "";
        if bytes.get(position) == Some(&b'.') {
            position += 1;
            let fraction_start = position;
            while bytes.get(position).is_some_and(u8::is_ascii_digit) {
                position += 1;
            }
            fraction = &text[fraction_start..position];
        }
        if integer.is_empty() && fraction.is_empty() {
            return Err(invalid());
        }
        let mut exponent: i64 = 0;
        if matches!(bytes.get(position), Some(b'e' | b'E')) {
            position += 1;
            let mut exponent_negative = false;
            match bytes.get(position) {
                Some(b'-') => {
                    exponent_negative = true;
                    position += 1;
                }
                Some(b'+') => position += 1,
                _ => {}
            }
            let exponent_start = position;
            while let Some(digit) = bytes.get(position).filter(|byte| byte.is_ascii_digit()) {
                // Any exponent past numeric's range is refused below, so the
                // accumulator only has to stay above that bound.
                exponent = (exponent * 10 + i64::from(digit - b'0')).min(i64::from(i32::MAX));
                position += 1;
            }
            if position == exponent_start {
                return Err(invalid());
            }
            if exponent_negative {
                exponent = -exponent;
            }
            // numeric refuses an exponent of half the 32-bit range or more,
            // whatever its digits, zero included.
            if exponent.abs() > i64::from(i32::MAX / 2) {
                return Err(Error::Message(format!("numeric value out of range: {text:?}")));
            }
        }
        if position != bytes.len() {
            return Err(invalid());
        }

        // The digits of the spelling, and where the decimal point falls in
        // them once the exponent has moved it.
        let digit_count = integer.len() + fraction.len();
        let point = integer.len() as i64 + exponent;
        let scale = (fraction.len() as i64 - exponent).max(0);
        let digit_at = |index: i64| -> u8 {
            if index < 0 || index >= digit_count as i64 {
                return b'0';
            }
            let index = index as usize;
            if index < integer.len() {
                integer.as_bytes()[index]
            } else {
                fraction.as_bytes()[index - integer.len()]
            }
        };
        // The integer digits run from the first nonzero digit before the
        // point to the point; the fraction is the `scale` digits after it.
        let first_significant = (0..digit_count as i64)
            .find(|&index| digit_at(index) != b'0')
            .unwrap_or(digit_count as i64);
        let is_zero = first_significant == digit_count as i64;
        // A zero has no significant integer digit, wherever its point falls.
        let integer_digits = if is_zero { 0 } else { (point - first_significant).max(0) };
        if integer_digits > NUMERIC_MAX_INTEGER_DIGITS as i64
            || scale > NUMERIC_MAX_FRACTION_DIGITS as i64
        {
            return Err(Error::Message(format!(
                "numeric value out of range: {text:?}"
            )));
        }
        let mut out = String::with_capacity(integer_digits as usize + scale as usize + 3);
        if negative && !is_zero {
            out.push('-');
        }
        if integer_digits == 0 {
            out.push('0');
        } else {
            for index in (point - integer_digits)..point {
                out.push(char::from(digit_at(index)));
            }
        }
        if scale > 0 {
            out.push('.');
            for index in point..point + scale {
                out.push(char::from(digit_at(index)));
            }
        }
        Ok(Numeric {
            text: out.into_boxed_str(),
        })
    }

    /// The `numeric` output text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The value with its sign flipped; zero stays unsigned.
    pub fn negated(&self) -> Numeric {
        let text = match self.text.strip_prefix('-') {
            Some(magnitude) => magnitude.into(),
            None if is_zero_text(&self.text) => self.text.clone(),
            None => format!("-{}", self.text).into_boxed_str(),
        };
        Numeric { text }
    }

    /// `(negative, integer digits, fraction digits)` of the output text.
    pub(crate) fn parts(&self) -> (bool, &str, &str) {
        split_decimal_text(&self.text)
    }
}

impl Display for Numeric {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl PartialEq for Numeric {
    fn eq(&self, other: &Self) -> bool {
        compare_decimal_text(&self.text, &other.text) == Ordering::Equal
    }
}

impl Eq for Numeric {}

/// `(negative, integer digits, fraction digits)` of a plain decimal spelling
/// `[-]digits[.digits]`.
pub(crate) fn split_decimal_text(text: &str) -> (bool, &str, &str) {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, text),
    };
    match unsigned.split_once('.') {
        Some((integer, fraction)) => (negative, integer, fraction),
        None => (negative, unsigned, ""),
    }
}

fn is_zero_text(text: &str) -> bool {
    text.bytes().all(|byte| matches!(byte, b'0' | b'.' | b'-'))
}

/// The numeric order of two plain decimal spellings `[-]digits[.digits]`:
/// by sign, then by magnitude, with leading integer zeros and trailing
/// fraction zeros not counting, so `1.50` equals `1.5` and `-0` equals `0`.
pub(crate) fn compare_decimal_text(left: &str, right: &str) -> Ordering {
    let (left_negative, left_integer, left_fraction) = split_decimal_text(left);
    let (right_negative, right_integer, right_fraction) = split_decimal_text(right);
    let sign = |negative: bool, text: &str| -> i8 {
        if is_zero_text(text) {
            0
        } else if negative {
            -1
        } else {
            1
        }
    };
    let left_sign = sign(left_negative, left);
    let right_sign = sign(right_negative, right);
    if left_sign != right_sign || left_sign == 0 {
        return left_sign.cmp(&right_sign);
    }
    let left_integer = left_integer.trim_start_matches('0');
    let right_integer = right_integer.trim_start_matches('0');
    let magnitude = left_integer
        .len()
        .cmp(&right_integer.len())
        .then_with(|| left_integer.cmp(right_integer))
        .then_with(|| {
            left_fraction
                .trim_end_matches('0')
                .cmp(right_fraction.trim_end_matches('0'))
        });
    if left_sign < 0 {
        magnitude.reverse()
    } else {
        magnitude
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(text: &str) -> String {
        Numeric::parse(text).unwrap().as_str().to_string()
    }

    #[test]
    fn spellings_read_as_numeric_in_reads_them() {
        assert_eq!(canonical("0"), "0");
        assert_eq!(canonical("-0"), "0");
        assert_eq!(canonical("-0.00"), "0.00");
        assert_eq!(canonical("1.50"), "1.50");
        assert_eq!(canonical("1.5e2"), "150");
        assert_eq!(canonical("1.50e1"), "15.0");
        assert_eq!(canonical("+007.5"), "7.5");
        assert_eq!(canonical(".5"), "0.5");
        assert_eq!(canonical("5."), "5");
        assert_eq!(canonical("1E-3"), "0.001");
        assert_eq!(canonical("-12.345e-2"), "-0.12345");
        assert_eq!(canonical("1e-400"), format!("0.{}1", "0".repeat(399)));
        assert_eq!(canonical("1e400"), format!("1{}", "0".repeat(400)));
        let ninety = "123456789".repeat(10);
        assert_eq!(canonical(&ninety), ninety);
        assert_eq!(
            canonical("4.94065645841247e-324"),
            format!("0.{}494065645841247", "0".repeat(323))
        );
        assert_eq!(
            canonical("1.79769313486232e+308"),
            format!("179769313486232{}", "0".repeat(294))
        );
    }

    #[test]
    fn non_numbers_and_out_of_range_magnitudes_are_refused() {
        for text in [
            "NaN", "nan", "Infinity", "-Infinity", "inf", "", "-", ".", "1e", "1e+", "1.2.3",
            "0x10", "1 ", " 1", "--1", "1e131073", "1e-16384",
        ] {
            assert!(Numeric::parse(text).is_err(), "{text:?}");
        }
        assert!(Numeric::parse("1e131071").is_ok());
        assert!(Numeric::parse("1e-16383").is_ok());
    }

    #[test]
    fn order_is_numeric_order() {
        let order = [
            "-1e400",
            "-90",
            "-1.5",
            "-1e-400",
            "0",
            "1e-400",
            "0.1",
            "1.5",
            "90",
            "123456789123456789123456789123456789123456789123456789123456789123456789123456789123456789",
            "1e400",
        ];
        for (index, left) in order.iter().enumerate() {
            for (other, right) in order.iter().enumerate() {
                let left = Numeric::parse(left).unwrap();
                let right = Numeric::parse(right).unwrap();
                assert_eq!(
                    compare_decimal_text(left.as_str(), right.as_str()),
                    index.cmp(&other),
                    "{left} vs {right}"
                );
            }
        }
        assert_eq!(compare_decimal_text("1.50", "1.5"), Ordering::Equal);
        assert_eq!(compare_decimal_text("-0.0", "0"), Ordering::Equal);
        assert_eq!(compare_decimal_text("0010", "10.000"), Ordering::Equal);
        assert_eq!(compare_decimal_text("0.05", "0.5"), Ordering::Less);
    }

    #[test]
    fn negation_flips_the_sign_of_a_nonzero_value() {
        assert_eq!(Numeric::parse("1e-400").unwrap().negated().as_str(), format!("-0.{}1", "0".repeat(399)));
        assert_eq!(Numeric::parse("-7").unwrap().negated().as_str(), "7");
        assert_eq!(Numeric::parse("0.00").unwrap().negated().as_str(), "0.00");
    }
}
