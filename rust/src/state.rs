use base64::{Engine, engine::general_purpose};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Store {
    path: PathBuf,
    pub data: Value,
}

pub fn decode(text: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text).or_else(|_| {
        let encoded: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = general_purpose::STANDARD
            .decode(&encoded)
            .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(&encoded))
            .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(encoded))
            .map_err(|_| "Invalid JSON or Base64 backup".to_owned())?;
        serde_json::from_slice(&bytes).map_err(|_| "Invalid backup JSON".to_owned())
    })?;
    if !value.is_object() {
        return Err("State must be a JSON object".to_owned());
    }
    Ok(value)
}

impl Store {
    pub fn open(directory: &Path) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        let path = directory.join("sub-store.json");
        let original = fs::read(&path).or_else(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Ok(b"{}".to_vec())
            } else {
                Err(e)
            }
        })?;
        let text = std::str::from_utf8(&original).map_err(io::Error::other)?;
        let mut data = decode(text).map_err(io::Error::other)?;
        let root_path = directory.join("root.json");
        if root_path.exists() {
            let root: Value = serde_json::from_slice(&fs::read(&root_path)?)?;
            if !root.is_object() {
                return Err(io::Error::other("root.json must be a JSON object"));
            }
            snapshot(&root_path, &fs::read(&root_path)?)?;
        } else {
            atomic_write(&root_path, b"{}")?;
        }
        if path.exists() {
            snapshot(&path, &original)?;
        }
        migrate(&mut data)?;
        for key in [
            "subs",
            "collections",
            "artifacts",
            "files",
            "tokens",
            "modules",
            "archives",
        ] {
            if !truthy(&data[key]) {
                data[key] = json!([]);
            }
            if !data[key].is_array() {
                return Err(io::Error::other(format!(
                    "{key} must be an array after migration"
                )));
            }
        }
        if !truthy(&data["settings"]) {
            data["settings"] = json!({});
        }
        if !data["settings"].is_object() {
            return Err(io::Error::other("settings must be an object"));
        }
        if !valid_records(&data) {
            return Err(io::Error::other("Invalid state records"));
        }
        let store = Self { path, data };
        atomic_write(&store.path, &serde_json::to_vec_pretty(&store.data)?)?;
        Ok(store)
    }

    pub fn commit(&mut self, next: Value) -> io::Result<()> {
        atomic_write(&self.path, &serde_json::to_vec_pretty(&next)?)?;
        self.data = next;
        Ok(())
    }
}

pub fn valid_records(data: &Value) -> bool {
    [
        "subs",
        "collections",
        "artifacts",
        "files",
        "tokens",
        "modules",
        "archives",
    ]
    .iter()
    .all(|key| {
        data.get(key).is_none_or(|items| {
            items
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_object))
        })
    })
}

fn snapshot(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let backup = path.with_extension("pre-rust.json");
    match private_file(&backup).open(&backup) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

fn private_file(_path: &Path) -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = private_file(&temporary).open(&temporary)?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temporary, metadata.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64() != Some(0.0),
        Value::String(value) => !value.is_empty(),
        _ => true,
    }
}

pub fn migrate(data: &mut Value) -> io::Result<()> {
    if !data.is_object() {
        return Err(io::Error::other("State must be an object"));
    }
    if !truthy(&data["schemaVersion"]) {
        for key in [
            "subs",
            "collections",
            "artifacts",
            "rules",
            "files",
            "tokens",
        ] {
            let previous = data
                .as_object_mut()
                .unwrap()
                .remove(key)
                .unwrap_or(Value::Null);
            let mut items: Vec<Value> = match previous {
                Value::Object(items) => {
                    let mut entries: Vec<_> = items.into_iter().collect();
                    // Object.values visits canonical array indices before other keys.
                    entries.sort_by_key(|(key, _)| {
                        key.parse::<u32>()
                            .ok()
                            .filter(|index| index.to_string() == *key)
                            .unwrap_or(u32::MAX)
                    });
                    entries.into_iter().map(|(_, value)| value).collect()
                }
                Value::Array(items) => items,
                Value::Null => Vec::new(),
                _ => return Err(io::Error::other(format!("Invalid legacy {key}"))),
            };
            if key == "subs" || key == "collections" {
                for item in &mut items {
                    let object = item
                        .as_object_mut()
                        .ok_or_else(|| io::Error::other("Invalid legacy item"))?;
                    if key == "subs" && !truthy(object.get("source").unwrap_or(&Value::Null)) {
                        object.insert("source".to_owned(), json!("remote"));
                    }
                    if key == "collections" {
                        object.remove("ua");
                    }
                    if object.get("display-name").is_some_and(truthy) {
                        let name = object.remove("display-name").unwrap();
                        object.insert("displayName".to_owned(), name);
                    }
                    if let Some(processes) = object.get_mut("process").and_then(Value::as_array_mut)
                        && !processes.is_empty()
                    {
                        let mut quick = json!({"type":"Quick Setting Operator", "args":{
                            "udp":"DEFAULT", "tfo":"DEFAULT", "scert":"DEFAULT",
                            "vmess aead":"DEFAULT", "useless":"DEFAULT"
                        }});
                        let mut next = Vec::new();
                        for mut process in std::mem::take(processes) {
                            let Some(kind) = process["type"].as_str() else {
                                continue;
                            };
                            if kind == "Useless Filter" {
                                quick["args"]["useless"] = json!("ENABLED");
                            } else if kind == "Set Property Operator" {
                                let field = match process["args"]["key"].as_str() {
                                    Some("udp") => "udp",
                                    Some("tfo") => "tfo",
                                    Some("skip-cert-verify") => "scert",
                                    Some("aead") => "vmess aead",
                                    _ => continue,
                                };
                                quick["args"][field] =
                                    json!(if truthy(&process["args"]["value"]) {
                                        "ENABLED"
                                    } else {
                                        "DISABLED"
                                    });
                            } else if kind.contains("Keyword") {
                                continue;
                            } else {
                                if kind == "Flag Operator" {
                                    let add = process.get("args").is_none_or(truthy);
                                    process["args"] =
                                        json!({"mode": if add { "add" } else { "remove" }});
                                }
                                next.push(process);
                            }
                        }
                        next.insert(0, quick);
                        *processes = next;
                    }
                }
            }
            data[key] = Value::Array(items);
        }
        data.as_object_mut().unwrap().remove("builtin");
    }
    data["schemaVersion"] = json!("2.0");
    Ok(())
}
