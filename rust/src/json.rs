use serde_json::Value;

// Escape this private marker too; the internal representation never reaches JSON.
const MARKER: char = '\u{e000}';

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
    text(&value.to_string())
}

pub fn pretty(value: &Value) -> Vec<u8> {
    text(&serde_json::to_string_pretty(value).expect("JSON value serialization")).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
