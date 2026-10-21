use base64::{Engine, engine::general_purpose};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

pub const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

pub fn uri_component(value: &str) -> Result<String, std::string::FromUtf16Error> {
    let value = String::from_utf16(&units(value))?;
    Ok(utf8_percent_encode(&value, COMPONENT).to_string())
}

pub fn decode_base64(text: &str) -> Result<Vec<u8>, base64::DecodeError> {
    // js-base64 removes padding/noise before Buffer decodes the remaining sextets.
    let mut encoded: String = text
        .chars()
        .filter_map(|c| match c {
            '-' => Some('+'),
            '_' => Some('/'),
            c if c.is_ascii_alphanumeric() || matches!(c, '+' | '/') => Some(c),
            _ => None,
        })
        .collect();
    if encoded.len() % 4 == 1 {
        encoded.pop();
    }
    general_purpose::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        general_purpose::NO_PAD.with_decode_allow_trailing_bits(true),
    )
    .decode(encoded)
}

// Escape this private marker too; the internal representation never reaches JSON.
const MARKER: char = '\u{e000}';

// User strings escape MARKER on ingress; these tokens only represent native numbers.
pub fn nonfinite(value: &Value) -> Option<f64> {
    match value.as_str()? {
        "\u{e000}number:NaN" => Some(f64::NAN),
        "\u{e000}number:Infinity" => Some(f64::INFINITY),
        "\u{e000}number:-Infinity" => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

pub fn numeric(value: f64) -> Value {
    if let Some(value) = serde_json::Number::from_f64(value) {
        Value::Number(value)
    } else {
        Value::String(format!(
            "{MARKER}number:{}",
            if value.is_nan() {
                "NaN"
            } else if value.is_sign_positive() {
                "Infinity"
            } else {
                "-Infinity"
            }
        ))
    }
}

pub fn trim(value: &str) -> &str {
    value.trim_matches(is_space)
}

pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'..='\u{000d}'
            | ' '
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

pub fn encode(text: &str) -> String {
    text.replace(MARKER, "\u{e000}\u{e000}")
}

pub fn from_units(words: &[u16]) -> String {
    let mut result = String::new();
    for character in char::decode_utf16(words.iter().copied()) {
        match character {
            Ok(MARKER) => result.push_str("\u{e000}\u{e000}"),
            Ok(character) => result.push(character),
            Err(error) => {
                result.push(MARKER);
                result.push_str(&format!("{:04x}", error.unpaired_surrogate()));
            }
        }
    }
    result
}

pub fn units(text: &str) -> Vec<u16> {
    let mut result = Vec::new();
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == MARKER {
            let next = characters.next().expect("escaped JSON marker");
            if next == MARKER {
                result.push(MARKER as u16);
            } else {
                let digits: String = std::iter::once(next)
                    .chain(characters.by_ref().take(3))
                    .collect();
                let word = u16::from_str_radix(&digits, 16).expect("encoded UTF-16 unit");
                assert!((0xd800..=0xdfff).contains(&word), "encoded surrogate");
                result.push(word);
            }
        } else {
            result.extend_from_slice(character.encode_utf16(&mut [0; 2]));
        }
    }
    result
}

pub fn text(encoded: &str) -> String {
    if !encoded.contains(MARKER) {
        return encoded.to_owned();
    }
    let mut result = String::new();
    for character in char::decode_utf16(units(encoded)) {
        match character {
            Ok(character) => result.push(character),
            Err(error) => result.push_str(&format!("\\u{:04x}", error.unpaired_surrogate())),
        }
    }
    result
}

fn escaped_unit(text: &str) -> Option<u16> {
    let digits = text.strip_prefix("\\u")?.get(..4)?;
    u16::from_str_radix(digits, 16).ok()
}

pub fn utf16_json(words: &[u16]) -> String {
    let mut text = String::new();
    let mut escaped = false;
    for character in char::decode_utf16(words.iter().copied()) {
        match character {
            Ok(character) => {
                text.push(character);
                escaped = character == '\\' && !escaped;
            }
            Err(error) => {
                // A raw surrogate cannot complete a JSON escape after a backslash.
                if escaped {
                    text.push('\0');
                } else {
                    text.push_str(&format!("\\u{:04x}", error.unpaired_surrogate()));
                }
                escaped = false;
            }
        }
    }
    text
}

pub fn parse(mut input: &str) -> serde_json::Result<Value> {
    if !input.contains("\\u") && !input.contains(MARKER) {
        return serde_json::from_str(input);
    }
    let mut output = String::with_capacity(input.len());
    let mut quoted = false;
    while let Some(character) = input.chars().next() {
        if quoted && character == '\\' {
            if let Some(word) = escaped_unit(input) {
                if (0xd800..=0xdbff).contains(&word)
                    && let Some(low @ 0xdc00..=0xdfff) = escaped_unit(&input[6..])
                {
                    output.push_str(&from_units(&[word, low]));
                    input = &input[12..];
                } else {
                    if (0xd800..=0xdfff).contains(&word) || word == MARKER as u16 {
                        output.push_str(&from_units(&[word]));
                    } else {
                        output.push_str(&input[..6]);
                    }
                    input = &input[6..];
                }
            } else {
                let length = 1 + input[1..].chars().next().map_or(0, char::len_utf8);
                output.push_str(&input[..length]);
                input = &input[length..];
            }
            continue;
        }
        if character == '"' {
            quoted = !quoted;
        } else if quoted && character == MARKER {
            output.push(MARKER);
        }
        output.push(character);
        input = &input[character.len_utf8()..];
    }
    serde_json::from_str(&output)
}

pub fn stringify(value: &Value) -> String {
    serialized(&value.to_string())
}

pub fn same_json(left: &Value, right: &Value) -> bool {
    fn identical(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Object(left), Value::Object(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right).all(|((key, left), (other, right))| {
                        key == other && !key.contains(MARKER) && identical(left, right)
                    })
            }
            (Value::Array(left), Value::Array(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right).all(|(left, right)| identical(left, right))
            }
            (Value::String(left), Value::String(right)) => left == right && !left.contains(MARKER),
            _ => left == right,
        }
    }
    // Value equality ignores object order. Keep that order and fall back to the
    // existing serializer for numeric equivalence and checked private markers.
    identical(left, right) || stringify(left) == stringify(right)
}

pub fn js_string(value: &Value) -> String {
    if let Some(number) = nonfinite(value) {
        return if number.is_nan() {
            "NaN".to_owned()
        } else if number.is_sign_positive() {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        };
    }
    match value {
        Value::String(value) => value.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    js_string(item)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Number(value) => number(value),
        _ => value.to_string(),
    }
}

pub fn pretty(value: &Value) -> Vec<u8> {
    serialized(&serde_json::to_string_pretty(value).expect("JSON value serialization")).into_bytes()
}

pub fn to_number(value: &Value) -> f64 {
    if let Some(number) = nonfinite(value) {
        return number;
    }
    match value {
        Value::Null => 0.0,
        Value::Bool(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        Value::Number(value) => value.as_f64().unwrap_or(f64::NAN),
        Value::String(value) => {
            let value = trim(value);
            if value.is_empty() {
                return 0.0;
            }
            for (prefix, radix) in [
                ("0x", 16),
                ("0X", 16),
                ("0b", 2),
                ("0B", 2),
                ("0o", 8),
                ("0O", 8),
            ] {
                if let Some(digits) = value.strip_prefix(prefix) {
                    if digits.is_empty() {
                        return f64::NAN;
                    }
                    return digits
                        .chars()
                        .try_fold(0.0, |total, digit| {
                            digit
                                .to_digit(radix)
                                .map(|digit| total * f64::from(radix) + f64::from(digit))
                        })
                        .unwrap_or(f64::NAN);
                }
            }
            value.parse().unwrap_or(f64::NAN)
        }
        Value::Array(_) => to_number(&Value::String(js_string(value))),
        _ => f64::NAN,
    }
}

pub fn number(value: &serde_json::Number) -> String {
    let number = value.as_f64().unwrap();
    if number == 0.0 {
        "0".to_owned()
    } else if (1e-6..1e21).contains(&number.abs()) {
        number.to_string()
    } else {
        let text = serde_json::json!(number).to_string();
        if let Some((mantissa, exponent)) = text.split_once('e') {
            let exponent = exponent.parse::<i32>().unwrap();
            format!("{mantissa}e{exponent:+}")
        } else {
            text
        }
    }
}

fn serialized(input: &str) -> String {
    let input = ["NaN", "Infinity", "-Infinity"]
        .into_iter()
        .fold(input.to_owned(), |input, number| {
            input.replace(&format!("\"{MARKER}number:{number}\""), "null")
        });
    let mut input = input.as_str();
    let mut output = String::with_capacity(input.len());
    let mut quoted = false;
    while let Some(character) = input.chars().next() {
        let length = if quoted && character == '\\' {
            1 + input[1..].chars().next().unwrap().len_utf8()
        } else if !quoted && (character == '-' || character.is_ascii_digit()) {
            let length = input
                .find(|c: char| !matches!(c, '0'..='9' | '-' | '+' | '.' | 'e' | 'E'))
                .unwrap_or(input.len());
            let value = serde_json::from_str(&input[..length]).expect("serialized JSON number");
            output.push_str(&number(&value));
            input = &input[length..];
            continue;
        } else {
            if character == '"' {
                quoted = !quoted;
            }
            character.len_utf8()
        };
        output.push_str(&input[..length]);
        input = &input[length..];
    }
    text(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn state_comparison_matches_serialized_order_numbers_and_checked_markers() {
        let mut values = vec![
            Value::Null,
            json!(false),
            json!(0),
            json!(-0.0),
            json!(1),
            json!(1.0),
            json!(9007199254740992_u64),
            json!(9007199254740993_u64),
            numeric(f64::NAN),
            numeric(f64::INFINITY),
            json!("1"),
            json!("中🙂\\\""),
            json!(from_units(&[0xd800, 0xe000, 0xdfff])),
            parse(r#""\ue000number:NaN""#).unwrap(),
            parse(r#"{"a":1,"b":2}"#).unwrap(),
            parse(r#"{"b":2,"a":1}"#).unwrap(),
            parse(r#"{"nested":[{"a":1,"b":2}]}"#).unwrap(),
            parse(r#"{"nested":[{"b":2,"a":1}]}"#).unwrap(),
            parse(r#"{"\ud800":{"\ue000":"\udfff"}}"#).unwrap(),
        ];
        values.push(Value::Array(values.clone()));
        for left in &values {
            for right in &values {
                assert_eq!(same_json(left, right), stringify(left) == stringify(right));
            }
        }
        assert!(!same_json(&values[14], &values[15]));
        assert!(!same_json(&values[16], &values[17]));
        assert!(same_json(&values[6], &values[7]));
        for invalid in [json!("\u{e000}"), json!({"\u{e000}": 1})] {
            assert!(std::panic::catch_unwind(|| stringify(&invalid)).is_err());
            assert!(std::panic::catch_unwind(|| same_json(&invalid, &invalid)).is_err());
        }
    }

    #[test]
    fn preserves_surrogates_keys_and_literal_marker_sequences() {
        let source = r#"{"\ud800":["\udfff","\ud83d\ude42","🙂","\ue000d800","\ue000\ue000","\\ud800","\"\\"]}"#;
        let value = parse(source).unwrap();
        let items = value[&from_units(&[0xd800])].as_array().unwrap();
        assert_eq!(items[1], items[2]);
        assert_eq!(text(items[3].as_str().unwrap()), "\u{e000}d800");
        assert_eq!(items[5], "\\ud800");
        let written = stringify(&value);
        assert!(written.contains(r#""\ud800""#));
        assert!(written.contains(r#""\udfff""#));
        assert_eq!(parse(&written).unwrap(), value);
        assert_eq!(
            parse(std::str::from_utf8(&pretty(&value)).unwrap()).unwrap(),
            value
        );
    }

    #[test]
    fn every_utf16_unit_survives_json_and_settings_projection() {
        for unit in 0..=u16::MAX {
            let encoded = from_units(&[unit]);
            let value = json!({(encoded.clone()): encoded});
            let written = stringify(&value);
            assert_eq!(parse(&written).unwrap(), value, "{unit:x}");
            assert_eq!(
                units(value.as_object().unwrap().keys().next().unwrap()),
                [unit]
            );
        }
        assert_eq!(units(&encode("中🙂")), [0x4e2d, 0xd83d, 0xde42]);
    }

    #[test]
    fn malformed_json_is_still_rejected() {
        for source in [
            r#""\ud80""#,
            r#""\ud80g""#,
            r#""\u+041""#,
            r#""\x00""#,
            r#"{"x":"a"b"}"#,
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
        assert_eq!(
            parse(&utf16_json(&[0x22, 0xd800, 0x22])).unwrap(),
            json!(from_units(&[0xd800]))
        );
        assert!(parse(&utf16_json(&[0x22, 0x5c, 0xd800, 0x22])).is_err());
        assert!(parse(&utf16_json(&[0xd800])).is_err());
        assert_eq!(
            parse(&utf16_json(&[0x22, 0x5c, 0x5c, 0xd800, 0x22])).unwrap(),
            json!(format!("\\{}", from_units(&[0xd800])))
        );
    }

    #[test]
    fn native_nonfinite_numbers_do_not_collide_with_user_strings() {
        let literal = parse(r#""\ue000number:NaN""#).unwrap();
        assert!(nonfinite(&literal).is_none());
        assert_eq!(parse(&stringify(&literal)).unwrap(), literal);
        let numbers = json!([
            numeric(f64::NAN),
            numeric(f64::INFINITY),
            numeric(f64::NEG_INFINITY)
        ]);
        assert_eq!(stringify(&numbers), "[null,null,null]");
        assert_eq!(
            String::from_utf8(pretty(&numbers)).unwrap(),
            "[\n  null,\n  null,\n  null\n]"
        );
        assert!(to_number(&numbers[0]).is_nan());
        assert_eq!(js_string(&numbers[1]), "Infinity");
        assert!(!crate::state::truthy(&numbers[0]));
        assert!(crate::state::truthy(&numbers[1]));
    }

    #[test]
    fn numbers_use_the_source_backend_value_and_wire_format() {
        let value = parse(r#"{"values":[1.0,-0.0,1e-6,1e-7,1e20,1e21,9007199254740993],"literal":"1e+21","key123":2}"#).unwrap();
        assert_eq!(
            stringify(&value),
            r#"{"values":[1,0,0.000001,1e-7,100000000000000000000,1e+21,9007199254740992],"literal":"1e+21","key123":2}"#
        );
    }
}
