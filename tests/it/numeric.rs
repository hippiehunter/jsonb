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

//! Numbers of `numeric`'s full range: read, stored, compared and printed
//! exactly, as PostgreSQL's `jsonb` keeps them.

use std::cmp::Ordering;

use jsonb::{parse_owned_jsonb, parse_value, Number, Numeric, OwnedJsonb, Value};

fn tiny() -> String {
    format!("0.{}1", "0".repeat(399))
}

fn huge() -> String {
    format!("1{}", "0".repeat(400))
}

fn ninety_digits() -> String {
    "123456789".repeat(10)
}

fn smallest_float8_expansion() -> String {
    format!("0.{}494065645841247", "0".repeat(323))
}

fn largest_float8_expansion() -> String {
    format!("179769313486232{}", "0".repeat(294))
}

fn jsonb(text: &str) -> OwnedJsonb {
    parse_owned_jsonb(text.as_bytes()).unwrap()
}

/// Each spelling with the `jsonb` output PostgreSQL 18 gives it.
fn extremes() -> Vec<(String, String)> {
    vec![
        ("1e-400".to_string(), tiny()),
        ("-1e-400".to_string(), format!("-{}", tiny())),
        ("1e400".to_string(), huge()),
        ("-1E+400".to_string(), format!("-{}", huge())),
        (ninety_digits(), ninety_digits()),
        (format!("-{}", ninety_digits()), format!("-{}", ninety_digits())),
        (
            format!("{}.5", ninety_digits()),
            format!("{}.5", ninety_digits()),
        ),
        (
            "4.94065645841247e-324".to_string(),
            smallest_float8_expansion(),
        ),
        (
            "1.79769313486232e+308".to_string(),
            largest_float8_expansion(),
        ),
        (
            "1.7976931348623157e308".to_string(),
            format!("17976931348623157{}", "0".repeat(292)),
        ),
        ("1.50e-1".to_string(), "0.150".to_string()),
        ("-0".to_string(), "0".to_string()),
        ("-0.000".to_string(), "0.000".to_string()),
        ("0e-400".to_string(), format!("0.{}", "0".repeat(400))),
    ]
}

#[test]
fn extreme_numbers_parse_print_and_round_trip_exactly() {
    for (input, output) in extremes() {
        let stored = jsonb(&input);
        assert_eq!(stored.as_raw().to_string(), output, "{input}");
        let reread = OwnedJsonb::new(stored.clone().to_vec());
        assert_eq!(reread.as_raw().to_string(), output, "{input}");
        // The printed text reads back to the same stored value.
        assert_eq!(
            jsonb(&output).as_raw().cmp(&stored.as_raw()),
            Ordering::Equal,
            "{input}"
        );
        let Value::Number(number) = parse_value(input.as_bytes()).unwrap() else {
            panic!("{input} is a number");
        };
        assert_eq!(number.to_string(), output, "{input}");
    }
}

#[test]
fn values_past_the_fixed_width_kinds_take_the_numeric_kind() {
    for input in [
        "1e-400".to_string(),
        "1e400".to_string(),
        ninety_digits(),
        "4.94065645841247e-324".to_string(),
        "1.79769313486232e+308".to_string(),
    ] {
        let Value::Number(number) = parse_value(input.as_bytes()).unwrap() else {
            panic!("{input} is a number");
        };
        assert!(matches!(number, Number::Numeric(_)), "{input}: {number:?}");
    }
    // Within 76 digits a value keeps its fixed-width kind, exponent or not.
    for (input, expected) in [
        ("1.5e2", Number::UInt64(150)),
        ("-2e1", Number::Int64(-20)),
        ("12.5e-1", Number::from_decimal_text("1.25").unwrap()),
    ] {
        let Value::Number(number) = parse_value(input.as_bytes()).unwrap() else {
            panic!("{input} is a number");
        };
        assert!(!matches!(number, Number::Numeric(_)), "{input}: {number:?}");
        assert_eq!(number, expected, "{input}");
    }
}

#[test]
fn not_a_number_and_infinite_spellings_are_refused() {
    for input in ["NaN", "-NaN", "Infinity", "-Infinity", "nan", "inf"] {
        assert!(Numeric::parse(input).is_err(), "{input}");
        assert!(Number::from_decimal_text(input).is_err(), "{input}");
    }
    assert!(parse_value(b"1e131072").is_err());
    assert!(parse_value(b"1e-16384").is_err());
}

#[test]
fn numeric_order_holds_across_every_kind() {
    let ascending = [
        format!("-{}", huge()),
        format!("-{}", ninety_digits()),
        "-9223372036854775809".to_string(),
        "-1.5".to_string(),
        format!("-{}", tiny()),
        "0".to_string(),
        tiny(),
        smallest_float8_expansion(),
        "0.5".to_string(),
        "1".to_string(),
        "18446744073709551616".to_string(),
        "123456789012345678901234567890123456789012345678901234567890123456789012345.5"
            .to_string(),
        ninety_digits(),
        largest_float8_expansion(),
        huge(),
    ];
    for (left_index, left) in ascending.iter().enumerate() {
        for (right_index, right) in ascending.iter().enumerate() {
            let expected = left_index.cmp(&right_index);
            let left_number = Number::from_decimal_text(left).unwrap();
            let right_number = Number::from_decimal_text(right).unwrap();
            assert_eq!(
                left_number.cmp(&right_number),
                expected,
                "{left} vs {right}"
            );
            assert_eq!(
                jsonb(left).as_raw().cmp(&jsonb(right).as_raw()),
                expected,
                "{left} vs {right}"
            );
        }
    }
    // Scale does not change a value.
    assert_eq!(
        Number::from_decimal_text(&format!("{}.000", ninety_digits())).unwrap(),
        Number::from_decimal_text(&ninety_digits()).unwrap()
    );
    assert_eq!(
        Number::from_decimal_text(&format!("{}0", tiny())).unwrap(),
        Number::from_decimal_text(&tiny()).unwrap()
    );
    assert_eq!(
        Number::from_decimal_text("1e-400").unwrap(),
        Number::from_decimal_text(&tiny()).unwrap()
    );
}

#[test]
fn containment_compares_numbers_by_value() {
    let document = jsonb(&format!(r#"{{"a": [{}, 1e-400, 2]}}"#, ninety_digits()));
    for (probe, expected) in [
        (format!(r#"{{"a": [{}]}}"#, ninety_digits()), true),
        (format!(r#"{{"a": [{}.00]}}"#, ninety_digits()), true),
        (r#"{"a": [1e-400]}"#.to_string(), true),
        (format!(r#"{{"a": [{}0]}}"#, tiny()), true),
        (r#"{"a": [2e-400]}"#.to_string(), false),
        (format!(r#"{{"a": [{}1]}}"#, ninety_digits()), false),
    ] {
        assert_eq!(
            document.as_raw().contains(&jsonb(&probe).as_raw()).unwrap(),
            expected,
            "{probe}"
        );
    }
}

#[test]
fn negation_is_exact_and_float_arithmetic_refuses_the_numeric_kind() {
    let number = Number::from_decimal_text("1e-400").unwrap();
    assert_eq!(number.neg().unwrap().to_string(), format!("-{}", tiny()));
    assert!(number.add(Number::UInt64(1)).is_err());
}

#[test]
fn a_zero_prints_one_unsigned_integer_digit_whatever_its_exponent() {
    for (input, expected) in [
        ("-0.00e4", "0"),
        ("0e5", "0"),
        ("0.000e2", "0.0"),
        ("-0e-3", "0.000"),
        ("00.0", "0.0"),
        ("0e1073741823", "0"),
    ] {
        assert_eq!(Numeric::parse(input).unwrap().as_str(), expected, "{input}");
    }
    for overflow in ["0e1073741824", "0e-1073741823", "0.0e-16383"] {
        assert!(Numeric::parse(overflow).is_err(), "{overflow}");
    }
}
