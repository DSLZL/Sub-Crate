use crate::{Result, require};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::Path;

pub fn read(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
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
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn check(actual: &Value, expected: &Value, subset: bool) -> Result {
    check_at(actual, expected, subset, "$")
}

fn check_at(actual: &Value, expected: &Value, subset: bool, path: &str) -> Result {
    match expected {
        Value::Object(wanted) => {
            let got = actual.as_object().ok_or_else(|| format!("{path}: expected object"))?;
            require(subset || got.len() == wanted.len(), format!("{path}: object keys differ"))?;
            for (key, value) in wanted {
                let actual = got.get(key).ok_or_else(|| format!("{path}: missing required field"))?;
                check_at(actual, value, subset, &format!("{path}.{key}"))?;
            }
        }
        Value::Array(wanted) => {
            let got = actual.as_array().ok_or_else(|| format!("{path}: expected array"))?;
            require(got.len() == wanted.len(), format!("{path}: array length differs"))?;
            for (index, (actual, expected)) in got.iter().zip(wanted).enumerate() {
                check_at(actual, expected, subset, &format!("{path}[{index}]"))?;
            }
        }
        Value::Number(wanted) => {
            let equal = actual.as_number().is_some_and(|got| {
                if got == wanted {
                    true
                } else {
                    let (float, integer) = if got.is_f64() { (got, wanted) } else { (wanted, got) };
                    float.is_f64() && !integer.is_f64() && float.as_f64().is_some_and(|value| {
                        value.fract() == 0.0 && if let Some(expected) = integer.as_u64() {
                            (0.0..18446744073709551616.0).contains(&value) && value as u64 == expected
                        } else if let Some(expected) = integer.as_i64() {
                            (-9223372036854775808.0..9223372036854775808.0).contains(&value) && value as i64 == expected
                        } else { false }
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
        require(!case["source"].as_str().unwrap_or("").is_empty(), "Fixture has no source")?;
        require(case["steps"].as_array().is_some_and(|steps| !steps.is_empty()), "Fixture has no steps")?;
        require(by_id.insert(id.to_owned(), case.clone()).is_none(), "Duplicate fixture")?;
    }
    required.iter().map(|id| {
        by_id.get(id.as_str().unwrap()).cloned().ok_or_else(|| "Required scenario has no executable fixture".into())
    }).collect()
}

pub fn full_match(pattern: &str, text: &str) -> Result<bool> {
    Ok(Regex::new(&format!(r"\A(?:{pattern})\z"))?.is_match(text))
}

pub fn captures(body: &Value, specifications: &Value) -> Result<Map<String, Value>> {
    let mut captured = Map::new();
    for (name, specification) in specifications.as_object().ok_or("Expected capture object")? {
        require(full_match("[A-Z][A-Z0-9_]*", name)?, "Invalid capture name")?;
        let pointer = specification["pointer"].as_str().ok_or("Missing JSON pointer")?;
        require(pointer.starts_with('/'), "Invalid JSON pointer")?;
        let value = body.pointer(pointer).ok_or("Missing capture path")?;
        match specification["type"].as_str() {
            Some("integer") => require(value.as_u64().is_some_and(|value| value > 0), "Expected positive captured integer")?,
            Some("string") => {
                let text = value.as_str().ok_or("Expected captured string")?;
                require(full_match(specification["pattern"].as_str().ok_or("Missing capture pattern")?, text)?, "Invalid captured string")?;
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
                    result.push_str(&replacement.as_str().map(str::to_owned).unwrap_or_else(|| replacement.to_string()));
                    offset = entire.end();
                }
                result.push_str(&text[offset..]);
                Value::String(result)
            }
            Value::Array(values) => Value::Array(values.iter().map(|value| visit(value, captured, pattern)).collect::<Result<_>>()?),
            Value::Object(values) => Value::Object(values.iter().map(|(key, value)| Ok((key.clone(), visit(value, captured, pattern)?))).collect::<Result<_>>()?),
            _ => value.clone(),
        })
    }
    visit(value, captured, &pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_and_subset_json_keep_types_arrays_missing_and_null() {
        crate::actions_only();
        assert!(check(&json!({"a":0,"b":false}), &json!({"b":false}), true).is_ok());
        for (actual, expected, subset) in [
            (json!({"a":0,"b":false}), json!({"b":false}), false),
            (json!({"a":false}), json!({"a":0}), true),
            (json!({}), json!({"a":null}), true),
            (json!([1,2]), json!([1]), true),
        ] {
            assert!(check(&actual, &expected, subset).is_err());
        }
        assert!(check(&json!(1.0), &json!(1), false).is_ok());
        assert!(check(&json!(9007199254740993_u64), &json!(9007199254740992_u64), false).is_err());
        assert!(check(&json!(9007199254740993_u64), &json!(9007199254740992.0), false).is_err());
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
        assert!(load_cases(&manifest, &json!([valid[0],valid[0],valid[1]]), "full").is_err());
        assert!(load_cases(&json!({"suites":{"full":["one","one"]}}), &valid, "full").is_err());
        assert!(load_cases(&json!({"suites":{"full":[]}}), &valid, "full").is_err());
        assert!(load_cases(&manifest, &json!([{"id":"one","steps":[]},valid[1]]), "full").is_err());
    }

    #[test]
    fn dynamic_values_are_validated_and_reused_without_type_coercion() {
        crate::actions_only();
        let spec = json!({"ID":{"pointer":"/rows/0/id","type":"integer"},"KEY":{"pointer":"/a~1b/~0x","type":"string","pattern":"[a-z]{3}"}});
        let values = captures(&json!({"rows":[{"id":42}],"a/b":{"~x":"abc"}}), &spec).unwrap();
        assert_eq!(interpolate(&json!({"id":"{{ID}}","path":"/row/{{KEY}}/{{ID}}"}), &values).unwrap(), json!({"id":42,"path":"/row/abc/42"}));
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
        assert_eq!(cases.iter().map(|case| case["steps"].as_array().unwrap().len()).sum::<usize>(), 1094);
        assert_eq!(manifest["suites"].as_object().unwrap().len(), 22);
        for suite in manifest["suites"].as_object().unwrap().keys() {
            if suite == "full" {
                assert!(load_cases(&manifest, &fixtures, suite).unwrap_err().to_string().contains("M1-complete-inventory"));
            } else {
                load_cases(&manifest, &fixtures, suite).unwrap();
            }
        }
    }
}
