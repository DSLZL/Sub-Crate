use crate::{Result, require};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::Path;

pub fn read(path: impl AsRef<Path>) -> Result<Value> {
    parse(&std::fs::read_to_string(path)?)
}

// serde_json strings are Unicode; JS fixtures also contain isolated UTF-16 units.
const UTF16_MARKER: char = '\u{e000}';

pub fn literal(text: &str) -> String {
    text.replace(UTF16_MARKER, "\u{e000}\u{e000}")
}

pub fn text(value: &Value) -> Result<String> {
    Ok(serde_json::from_str(&quote(
        value.as_str().ok_or("Expected string value")?,
    ))?)
}

pub fn parse(raw: &str) -> Result<Value> {
    fn unit(raw: &str) -> Option<u16> {
        raw.strip_prefix("\\u")
            .and_then(|tail| tail.get(..4))
            .and_then(|digits| u16::from_str_radix(digits, 16).ok())
    }
    let mut escaped = String::with_capacity(raw.len());
    let mut offset = 0;
    let mut string = false;
    while offset < raw.len() {
        let rest = &raw[offset..];
        let character = rest.chars().next().unwrap();
        if character == '"' {
            string = !string;
        }
        if string && character == '\\' {
            if let Some(word) = unit(rest) {
                if (0xd800..=0xdbff).contains(&word)
                    && unit(&rest[6..]).is_some_and(|low| (0xdc00..=0xdfff).contains(&low))
                {
                    escaped.push_str(&rest[..12]);
                    offset += 12;
                    continue;
                }
                if (0xd800..=0xdfff).contains(&word) {
                    escaped.push(UTF16_MARKER);
                    escaped.push_str(&format!("{word:04x}"));
                    offset += 6;
                    continue;
                }
                if word == UTF16_MARKER as u16 {
                    escaped.push(UTF16_MARKER);
                    escaped.push(UTF16_MARKER);
                    offset += 6;
                    continue;
                }
                escaped.push_str(&rest[..6]);
                offset += 6;
                continue;
            }
            if let Some(next) = rest.chars().nth(1) {
                escaped.push(character);
                escaped.push(next);
                offset += 1 + next.len_utf8();
                continue;
            }
        }
        escaped.push(character);
        if string && character == UTF16_MARKER {
            escaped.push(character);
        }
        offset += character.len_utf8();
    }
    Ok(serde_json::from_str(&escaped)?)
}

pub fn quote(text: &str) -> String {
    let mut result = String::from("\"");
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == UTF16_MARKER {
            if characters.as_str().starts_with(UTF16_MARKER) {
                characters.next();
            } else {
                let digits: String = characters.by_ref().take(4).collect();
                assert!(
                    u16::from_str_radix(&digits, 16)
                        .is_ok_and(|unit| (0xd800..=0xdfff).contains(&unit)),
                    "Invalid internal UTF-16 encoding"
                );
                result.push_str("\\u");
                result.push_str(&digits);
                continue;
            }
        }
        let encoded = serde_json::to_string(&character.to_string()).unwrap();
        result.push_str(&encoded[1..encoded.len() - 1]);
    }
    result.push('"');
    result
}

pub fn stringify(value: &Value) -> String {
    match value {
        Value::String(text) => quote(text),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(stringify).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(key, value)| format!("{}: {}", quote(key), stringify(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => value.to_string(),
    }
}

pub fn write_new(path: impl AsRef<Path>, value: &Value) -> Result {
    use std::io::Write;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(stringify(value).as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn check(actual: &Value, expected: &Value, subset: bool) -> Result {
    check_at(actual, expected, subset, "$")
}

fn check_at(actual: &Value, expected: &Value, subset: bool, path: &str) -> Result {
    match expected {
        Value::Object(wanted) => {
            let got = actual
                .as_object()
                .ok_or_else(|| format!("{path}: expected object"))?;
            require(
                subset || got.len() == wanted.len(),
                format!("{path}: object keys differ"),
            )?;
            for (key, value) in wanted {
                let actual = got
                    .get(key)
                    .ok_or_else(|| format!("{path}: missing required field"))?;
                check_at(actual, value, subset, &format!("{path}.{key}"))?;
            }
        }
        Value::Array(wanted) => {
            let got = actual
                .as_array()
                .ok_or_else(|| format!("{path}: expected array"))?;
            require(
                got.len() == wanted.len(),
                format!("{path}: array length differs"),
            )?;
            for (index, (actual, expected)) in got.iter().zip(wanted).enumerate() {
                check_at(actual, expected, subset, &format!("{path}[{index}]"))?;
            }
        }
        Value::Number(wanted) => {
            let equal = actual.as_number().is_some_and(|got| {
                if got == wanted {
                    true
                } else {
                    let (float, integer) = if got.is_f64() {
                        (got, wanted)
                    } else {
                        (wanted, got)
                    };
                    float.is_f64()
                        && !integer.is_f64()
                        && float.as_f64().is_some_and(|value| {
                            value.fract() == 0.0
                                && if let Some(expected) = integer.as_u64() {
                                    (0.0..18446744073709551616.0).contains(&value)
                                        && value as u64 == expected
                                } else if let Some(expected) = integer.as_i64() {
                                    (-9223372036854775808.0..9223372036854775808.0).contains(&value)
                                        && value as i64 == expected
                                } else {
                                    false
                                }
                        })
                }
            });
            require(equal, format!("{path}: number differs"))?;
        }
        _ => require(actual == expected, format!("{path}: scalar differs"))?,
    }
    Ok(())
}

pub fn load_cases(manifest: &Value, fixtures: &Value, suite: &str) -> Result<Vec<Value>> {
    let required = manifest["suites"][suite]
        .as_array()
        .ok_or("Unknown required suite")?;
    require(!required.is_empty(), "Empty required suite")?;
    let mut ids = HashSet::new();
    for id in required {
        let id = id.as_str().ok_or("Invalid required scenario")?;
        require(ids.insert(id), "Duplicate required scenario")?;
    }
    let mut by_id = Map::new();
    for case in fixtures.as_array().ok_or("Expected fixture array")? {
        let id = case["id"].as_str().ok_or("Missing fixture identifier")?;
        require(
            !case["source"].as_str().unwrap_or("").is_empty(),
            "Fixture has no source",
        )?;
        require(
            case["steps"]
                .as_array()
                .is_some_and(|steps| !steps.is_empty()),
            "Fixture has no steps",
        )?;
        require(
            by_id.insert(id.to_owned(), case.clone()).is_none(),
            "Duplicate fixture",
        )?;
    }
    required
        .iter()
        .map(|id| {
            by_id.get(id.as_str().unwrap()).cloned().ok_or_else(|| {
                format!(
                    "Required scenario has no executable fixture: {}",
                    id.as_str().unwrap()
                )
                .into()
            })
        })
        .collect()
}

pub fn full_match(pattern: &str, text: &str) -> Result<bool> {
    Ok(Regex::new(&format!(r"\A(?:{pattern})\z"))?.is_match(text))
}

pub fn captures(body: &Value, specifications: &Value) -> Result<Map<String, Value>> {
    let mut captured = Map::new();
    for (name, specification) in specifications
        .as_object()
        .ok_or("Expected capture object")?
    {
        require(full_match("[A-Z][A-Z0-9_]*", name)?, "Invalid capture name")?;
        let pointer = specification["pointer"]
            .as_str()
            .ok_or("Missing JSON pointer")?;
        require(pointer.starts_with('/'), "Invalid JSON pointer")?;
        let value = body.pointer(pointer).ok_or("Missing capture path")?;
        match specification["type"].as_str() {
            Some("integer") => require(
                value.as_u64().is_some_and(|value| value > 0),
                "Expected positive captured integer",
            )?,
            Some("string") => {
                let text = value.as_str().ok_or("Expected captured string")?;
                require(
                    full_match(
                        specification["pattern"]
                            .as_str()
                            .ok_or("Missing capture pattern")?,
                        text,
                    )?,
                    "Invalid captured string",
                )?;
            }
            _ => return Err("Unknown capture type".into()),
        }
        captured.insert(name.clone(), value.clone());
    }
    Ok(captured)
}

pub fn interpolate(value: &Value, captured: &Map<String, Value>) -> Result<Value> {
    let pattern = Regex::new(r"\{\{([A-Z][A-Z0-9_]*)\}\}")?;
    fn visit(value: &Value, captured: &Map<String, Value>, pattern: &Regex) -> Result<Value> {
        Ok(match value {
            Value::String(text) => {
                let mut result = String::new();
                let mut offset = 0;
                for found in pattern.captures_iter(text) {
                    let entire = found.get(0).unwrap();
                    let replacement = captured.get(&found[1]).ok_or("Unknown capture reference")?;
                    if entire.start() == 0 && entire.end() == text.len() {
                        return Ok(replacement.clone());
                    }
                    result.push_str(&text[offset..entire.start()]);
                    result.push_str(
                        &replacement
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| replacement.to_string()),
                    );
                    offset = entire.end();
                }
                result.push_str(&text[offset..]);
                Value::String(result)
            }
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|value| visit(value, captured, pattern))
                    .collect::<Result<_>>()?,
            ),
            Value::Object(values) => Value::Object(
                values
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), visit(value, captured, pattern)?)))
                    .collect::<Result<_>>()?,
            ),
            _ => value.clone(),
        })
    }
    visit(value, captured, &pattern)
}

pub fn project_captured_scalars(
    response: &Value,
    expected: &Value,
    captured: &Map<String, Value>,
) -> Result<Option<Value>> {
    use base64::Engine;
    let pattern = Regex::new(r"\{\{[A-Z][A-Z0-9_]*\}\}")?;
    let mut targets: Vec<(Vec<Value>, String)> = Vec::new();
    fn declared(
        value: &Value,
        path: &mut Vec<Value>,
        pattern: &Regex,
        targets: &mut Vec<(Vec<Value>, String)>,
    ) -> Result {
        match value {
            Value::String(template) if pattern.is_match(template) => {
                if let Some((_, old)) = targets.iter().find(|(key, _)| key == path) {
                    require(old == template, "Conflicting capture paths")?;
                } else {
                    targets.push((path.clone(), template.clone()));
                }
            }
            Value::Array(values) => {
                for (index, item) in values.iter().enumerate() {
                    path.push(Value::from(index));
                    declared(item, path, pattern, targets)?;
                    path.pop();
                }
            }
            Value::Object(values) => {
                for (key, item) in values {
                    path.push(Value::from(key.clone()));
                    declared(item, path, pattern, targets)?;
                    path.pop();
                }
            }
            _ => (),
        }
        Ok(())
    }
    for key in ["json", "json_contains"] {
        if let Some(value) = expected.get(key) {
            declared(value, &mut Vec::new(), &pattern, &mut targets)?;
        }
    }
    if targets.is_empty() {
        return Ok(None);
    }
    let body = response["body"].as_str().ok_or("Missing captured body")?;
    let _ = parse(body)?;
    let length = body.len().to_string();
    require(
        response["headers"]["content-length"].as_str() == Some(length.as_str()),
        "Captured body length mismatch",
    )?;
    let digest = base64::engine::general_purpose::STANDARD_NO_PAD.encode(
        aws_lc_rs::digest::digest(
            &aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY,
            body.as_bytes(),
        )
        .as_ref(),
    );
    require(
        response["headers"]["etag"] == format!("W/\"{:x}-{digest}\"", body.len()),
        "Captured body ETag mismatch",
    )?;

    struct Scanner<'a> {
        raw: &'a str,
        targets: &'a [(Vec<Value>, String)],
        captured: &'a Map<String, Value>,
        visited: HashSet<usize>,
        replacements: Vec<(usize, usize, String)>,
    }
    impl Scanner<'_> {
        fn whitespace(&self, mut offset: usize) -> usize {
            while self
                .raw
                .as_bytes()
                .get(offset)
                .is_some_and(u8::is_ascii_whitespace)
            {
                offset += 1;
            }
            offset
        }
        fn value(&self, offset: usize) -> Result<(Value, usize)> {
            let bytes = self.raw.as_bytes();
            let mut end = offset;
            if bytes.get(offset) == Some(&b'"') {
                end += 1;
                loop {
                    match bytes.get(end).ok_or("Unterminated JSON string")? {
                        b'\\' => end += 2,
                        b'"' => {
                            end += 1;
                            break;
                        }
                        _ => end += 1,
                    }
                }
            } else {
                while bytes
                    .get(end)
                    .is_some_and(|byte| !b",]} \r\n\t".contains(byte))
                {
                    end += 1;
                }
            }
            Ok((parse(&self.raw[offset..end])?, end))
        }
        fn scan(&mut self, offset: usize, path: &mut Vec<Value>) -> Result<usize> {
            let offset = self.whitespace(offset);
            if let Some(index) = self.targets.iter().position(|(key, _)| key == path) {
                require(
                    !matches!(self.raw.as_bytes().get(offset), Some(b'{' | b'[')),
                    "Capture projection must be a scalar",
                )?;
                let (actual, end) = self.value(offset)?;
                let expected =
                    interpolate(&Value::from(self.targets[index].1.clone()), self.captured)?;
                require(
                    !actual.is_object() && !actual.is_array(),
                    "Capture projection must be a scalar",
                )?;
                require(
                    std::mem::discriminant(&actual) == std::mem::discriminant(&expected)
                        && actual.is_f64() == expected.is_f64()
                        && actual == expected,
                    "Capture projection value mismatch",
                )?;
                self.replacements.push((
                    offset,
                    end,
                    serde_json::to_string(&self.targets[index].1)?,
                ));
                self.visited.insert(index);
                return Ok(end);
            }
            let mapping = match self.raw.as_bytes().get(offset) {
                Some(b'{') => true,
                Some(b'[') => false,
                _ => return Ok(self.value(offset)?.1),
            };
            let close = if mapping { b'}' } else { b']' };
            let mut cursor = self.whitespace(offset + 1);
            let mut index = 0;
            while self.raw.as_bytes().get(cursor) != Some(&close) {
                let key = if mapping {
                    let (key, end) = self.value(cursor)?;
                    require(key.is_string(), "Invalid JSON object key")?;
                    cursor = self.whitespace(end);
                    require(
                        self.raw.as_bytes().get(cursor) == Some(&b':'),
                        "Invalid JSON object",
                    )?;
                    cursor += 1;
                    key
                } else {
                    Value::from(index)
                };
                path.push(key);
                cursor = self.scan(cursor, path)?;
                path.pop();
                cursor = self.whitespace(cursor);
                index += 1;
                if self.raw.as_bytes().get(cursor) == Some(&b',') {
                    cursor = self.whitespace(cursor + 1);
                } else {
                    require(
                        self.raw.as_bytes().get(cursor) == Some(&close),
                        "Invalid JSON container",
                    )?;
                }
            }
            Ok(cursor + 1)
        }
    }
    let mut scanner = Scanner {
        raw: body,
        targets: &targets,
        captured,
        visited: HashSet::new(),
        replacements: Vec::new(),
    };
    let end = scanner.scan(0, &mut Vec::new())?;
    require(
        scanner.whitespace(end) == body.len(),
        "Trailing JSON content",
    )?;
    require(
        scanner.visited.len() == targets.len(),
        "Missing capture projection path",
    )?;
    let mut projected = body.to_owned();
    for (start, end, replacement) in scanner.replacements.into_iter().rev() {
        projected.replace_range(start..end, &replacement);
    }
    Ok(Some(
        serde_json::json!({"body":projected,"paths":targets.iter().map(|(path,template)| serde_json::json!({"path":path,"template":template})).collect::<Vec<_>>()}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn external_json_text_and_nested_backups_preserve_literal_markers_and_surrogate_escapes()
    -> Result {
        crate::actions_only();
        let raw =
            "{\"marker\":\"\u{e000}d800\",\"markers\":\"\u{e000}\u{e000}\",\"high\":\"\\ud800\"}";
        let document = parse(raw)?;
        let envelope = json!({"body":literal(raw),"content":literal(&stringify(&document))});
        let decoded = parse(&stringify(&envelope))?;
        assert_eq!(text(&decoded["body"])?, raw);
        check(&parse(&text(&decoded["content"])?)?, &document, false)
    }

    #[test]
    fn exact_and_subset_json_keep_types_arrays_missing_and_null() {
        crate::actions_only();
        assert!(check(&json!({"a":0,"b":false}), &json!({"b":false}), true).is_ok());
        for (actual, expected, subset) in [
            (json!({"a":0,"b":false}), json!({"b":false}), false),
            (json!({"a":false}), json!({"a":0}), true),
            (json!({}), json!({"a":null}), true),
            (json!([1, 2]), json!([1]), true),
        ] {
            assert!(check(&actual, &expected, subset).is_err());
        }
        assert!(check(&json!(1.0), &json!(1), false).is_ok());
        assert!(
            check(
                &json!(9007199254740993_u64),
                &json!(9007199254740992_u64),
                false
            )
            .is_err()
        );
        assert!(
            check(
                &json!(9007199254740993_u64),
                &json!(9007199254740992.0),
                false
            )
            .is_err()
        );
    }

    #[test]
    fn js_strings_keep_surrogates_pairs_keys_and_literal_marker_sequences() {
        crate::actions_only();
        let raw = r#"{"\ud800":["\udfff","\ud83d\ude42","🙂","\ue000d800","\ue000\ue000","\\ud800","\"\\"]}"#;
        let value = parse(raw).unwrap();
        let roundtrip = stringify(&value);
        assert!(roundtrip.contains(r#""\ud800""#));
        assert!(roundtrip.contains(r#""\udfff""#));
        assert!(roundtrip.contains(r#""\\ud800""#));
        assert!(roundtrip.contains("🙂"));
        assert_eq!(parse(&roundtrip).unwrap(), value);
        assert_ne!(
            parse(r#""\ud800""#).unwrap(),
            parse(r#""\ue000d800""#).unwrap()
        );
        assert!(parse(r#""\uD80x""#).is_err());
        assert!(parse(r#""\ud800" trailing"#).is_err());
    }

    #[test]
    fn required_suites_reject_missing_duplicate_empty_and_invalid_fixtures() {
        crate::actions_only();
        let manifest = json!({"suites":{"full":["one","two"]}});
        let valid = json!([
            {"id":"one","source":"fixture:1","steps":[{}]},
            {"id":"two","source":"fixture:2","steps":[{}]}
        ]);
        assert_eq!(load_cases(&manifest, &valid, "full").unwrap().len(), 2);
        assert!(load_cases(&manifest, &valid, "absent").is_err());
        assert!(load_cases(&manifest, &json!([valid[0]]), "full").is_err());
        assert!(load_cases(&manifest, &json!([valid[0], valid[0], valid[1]]), "full").is_err());
        assert!(load_cases(&json!({"suites":{"full":["one","one"]}}), &valid, "full").is_err());
        assert!(load_cases(&json!({"suites":{"full":[]}}), &valid, "full").is_err());
        assert!(
            load_cases(
                &manifest,
                &json!([{"id":"one","steps":[]},valid[1]]),
                "full"
            )
            .is_err()
        );
    }

    #[test]
    fn dynamic_values_are_validated_and_reused_without_type_coercion() {
        crate::actions_only();
        let spec = json!({"ID":{"pointer":"/rows/0/id","type":"integer"},"KEY":{"pointer":"/a~1b/~0x","type":"string","pattern":"[a-z]{3}"}});
        let values = captures(&json!({"rows":[{"id":42}],"a/b":{"~x":"abc"}}), &spec).unwrap();
        assert_eq!(
            interpolate(
                &json!({"id":"{{ID}}","path":"/row/{{KEY}}/{{ID}}"}),
                &values
            )
            .unwrap(),
            json!({"id":42,"path":"/row/abc/42"})
        );
        for value in [json!(false), json!(0), json!(1.5), json!("42")] {
            assert!(captures(&json!({"rows":[{"id":value}],"a/b":{"~x":"abc"}}), &spec).is_err());
        }
        assert!(interpolate(&json!("{{UNKNOWN}}"), &values).is_err());
        assert!(captures(&json!({"rows":[{"id":42}],"a/b":{"~x":"abcd"}}), &spec).is_err());
    }

    #[test]
    fn checked_in_http_inputs_retain_the_complete_source_inventory() {
        crate::actions_only();
        let manifest = read(crate::compat().join("manifest.json")).unwrap();
        let fixtures = read(crate::compat().join("fixtures/http-cases.json")).unwrap();
        let cases = fixtures.as_array().unwrap();
        assert_eq!(cases.len(), 235);
        assert_eq!(
            cases
                .iter()
                .map(|case| case["steps"].as_array().unwrap().len())
                .sum::<usize>(),
            1094
        );
        assert_eq!(manifest["suites"].as_object().unwrap().len(), 22);
        for suite in manifest["suites"].as_object().unwrap().keys() {
            if suite == "full" {
                assert!(
                    load_cases(&manifest, &fixtures, suite)
                        .unwrap_err()
                        .to_string()
                        .contains("M1-complete-inventory")
                );
            } else {
                load_cases(&manifest, &fixtures, suite).unwrap();
            }
        }
    }

    #[test]
    fn capture_projection_preserves_every_undeclared_byte() {
        use base64::Engine;
        crate::actions_only();
        let response = |body: &str| {
            let digest = base64::engine::general_purpose::STANDARD_NO_PAD.encode(
                aws_lc_rs::digest::digest(
                    &aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY,
                    body.as_bytes(),
                )
                .as_ref(),
            );
            json!({"body":body,"headers":{"content-length":body.len().to_string(),"etag":format!("W/\"{:x}-{digest}\"",body.len())}})
        };
        let expected = json!({"json_contains":{"rows":[{"at":"{{AT}}","a/b":"{{ID}}"}]}});
        let first = response(
            r#"{"rows":[{"at":17,"a/b":"id\"x","other":17}],"keep":"id\"x","unicode":"🙂"}"#,
        );
        let second = response(
            r#"{"rows":[{"at":29,"a/b":"id-y","other":17}],"keep":"id\"x","unicode":"🙂"}"#,
        );
        let captured = json!({"AT":17,"ID":"id\"x"}).as_object().unwrap().clone();
        let a = project_captured_scalars(&first, &expected, &captured)
            .unwrap()
            .unwrap();
        let b = project_captured_scalars(
            &second,
            &expected,
            json!({"AT":29,"ID":"id-y"}).as_object().unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(a, b);
        assert!(
            a["body"]
                .as_str()
                .unwrap()
                .contains(r#""other":17}],"keep":"id\"x","unicode":"🙂""#)
        );
        let mut bad = first.clone();
        bad["headers"]["etag"] = json!("wrong");
        assert!(project_captured_scalars(&bad, &expected, &captured).is_err());
        assert!(
            project_captured_scalars(
                &first,
                &expected,
                json!({"AT":29,"ID":"id\"x"}).as_object().unwrap()
            )
            .is_err()
        );
        assert!(
            project_captured_scalars(&first, &json!({"json":{"missing":"{{AT}}"}}), &captured)
                .is_err()
        );
        assert!(project_captured_scalars(&first, &expected, &Map::new()).is_err());
        assert!(
            project_captured_scalars(&first, &json!({"json":{}}), &captured)
                .unwrap()
                .is_none()
        );
    }
}
