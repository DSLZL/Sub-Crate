use crate::json as codec;
use base64::{Engine, engine::general_purpose};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Store {
    path: PathBuf,
    pub data: Value,
    live_buffers: HashSet<&'static str>,
}

pub fn decode(text: &str) -> Result<Value, String> {
    let value: Value = codec::parse(text).or_else(|_| {
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
        let decoder = general_purpose::GeneralPurpose::new(
            &base64::alphabet::STANDARD,
            general_purpose::NO_PAD.with_decode_allow_trailing_bits(true),
        );
        let bytes = decoder
            .decode(encoded)
            .map_err(|_| "Invalid JSON or Base64 backup".to_owned())?;
        codec::parse(&String::from_utf8_lossy(&bytes)).map_err(|_| "Invalid backup JSON".to_owned())
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
        recover_restore(&path)?;
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
            let root = codec::parse(
                std::str::from_utf8(&fs::read(&root_path)?).map_err(io::Error::other)?,
            )?;
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
            if !data[key].is_array() && !stored_buffer(key, &data[key]) {
                return Err(io::Error::other(format!(
                    "{key} must be an array after migration"
                )));
            }
        }
        if !truthy(&data["settings"]) {
            data["settings"] = json!({});
        }
        if !valid_records(&data) {
            return Err(io::Error::other("Invalid state records"));
        }
        let store = Self {
            path,
            data,
            live_buffers: HashSet::new(),
        };
        atomic_write(&store.path, &codec::pretty(&store.data))?;
        Ok(store)
    }

    pub fn commit(&mut self, next: Value) -> io::Result<()> {
        recover_restore(&self.path)?;
        atomic_write(&self.path, &codec::pretty(&next))?;
        self.data = next;
        Ok(())
    }

    pub fn list(&self, key: &str) -> Option<&[Value]> {
        let value = if self.live_buffers.contains(key) {
            &self.data[key]["data"]
        } else {
            &self.data[key]
        };
        value.as_array().map(Vec::as_slice)
    }

    pub fn commit_list(&mut self, next: Value, key: &'static str, raw: bool) -> io::Result<()> {
        self.commit(next)?;
        if raw {
            self.live_buffers.insert(key);
        } else {
            self.live_buffers.remove(key);
        }
        Ok(())
    }

    pub fn restore(&mut self, next: Value, original: Value) -> io::Result<()> {
        recover_restore(&self.path)?;
        let root_path = self.path.with_file_name("root.json");
        let previous_data = fs::read(&self.path)?;
        let previous_root = fs::read(&root_path)?;
        let mut root =
            codec::parse(std::str::from_utf8(&previous_root).map_err(io::Error::other)?)?;
        let original = String::from_utf8(codec::pretty(&legacy_object_order(original)))
            .expect("serialized JSON is UTF-8");
        let root_fields = root
            .as_object_mut()
            .ok_or_else(|| io::Error::other("root.json must be an object"))?;
        root_fields.insert("sub-store".to_owned(), json!(codec::encode(&original)));
        let pending = self.path.with_file_name(".sub-store-restore.pending.json");
        let previous = json!({
            "data": general_purpose::STANDARD.encode(previous_data),
            "root": general_purpose::STANDARD.encode(previous_root)
        });
        atomic_write(&pending, &serde_json::to_vec(&previous)?)?;
        let result = (|| {
            atomic_write(&root_path, &codec::pretty(&root))?;
            atomic_write(&self.path, &codec::pretty(&next))?;
            fs::remove_file(&pending)
        })();
        if let Err(error) = result {
            recover_restore(&self.path).map_err(|recovery| {
                io::Error::other(format!("{error}; restore recovery failed: {recovery}"))
            })?;
            return Err(error);
        }
        self.data = next;
        self.live_buffers.clear();
        Ok(())
    }

    pub fn clear_resource_cache(&mut self) -> io::Result<()> {
        recover_restore(&self.path)?;
        let path = self.path.with_file_name("root.json");
        let mut root =
            codec::parse(std::str::from_utf8(&fs::read(&path)?).map_err(io::Error::other)?)?;
        let root_fields = root
            .as_object_mut()
            .ok_or_else(|| io::Error::other("root.json must be an object"))?;
        for key in [
            "sub-store-cached-resource",
            "sub-store-cached-headers-resource",
            "sub-store-cached-script-resource",
        ] {
            root_fields.insert(key.to_owned(), json!("{}"));
        }
        atomic_write(&path, &codec::pretty(&root))
    }
}

fn recover_restore(path: &Path) -> io::Result<()> {
    let pending = path.with_file_name(".sub-store-restore.pending.json");
    let bytes = match fs::read(&pending) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let previous: Value = serde_json::from_slice(&bytes)?;
    let saved = |field| {
        let encoded = previous[field]
            .as_str()
            .ok_or_else(|| io::Error::other("Invalid restore recovery record"))?;
        general_purpose::STANDARD
            .decode(encoded)
            .map_err(io::Error::other)
    };
    let data = saved("data")?;
    let root = saved("root")?;
    decode(std::str::from_utf8(&data).map_err(io::Error::other)?).map_err(io::Error::other)?;
    if !codec::parse(std::str::from_utf8(&root).map_err(io::Error::other)?)?.is_object() {
        return Err(io::Error::other("Invalid root restore recovery record"));
    }
    atomic_write(&path.with_file_name("root.json"), &root)?;
    atomic_write(path, &data)?;
    fs::remove_file(pending)
}

fn array_index(key: &str) -> u32 {
    key.parse::<u32>()
        .ok()
        .filter(|index| index.to_string() == key)
        .unwrap_or(u32::MAX)
}

pub fn legacy_object_order(value: Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut fields: Vec<_> = fields.into_iter().collect();
            fields.sort_by_key(|(key, _)| array_index(key));
            Value::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key, legacy_object_order(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(legacy_object_order).collect()),
        value => value,
    }
}

fn stored_buffer(key: &str, value: &Value) -> bool {
    matches!(key, "subs" | "collections")
        && value.as_object().is_some_and(|object| object.len() == 2)
        && value["type"] == "Buffer"
        && value["data"].as_array().is_some_and(|bytes| {
            bytes
                .iter()
                .all(|byte| byte.as_u64().is_some_and(|byte| byte <= 255))
        })
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
            stored_buffer(key, items)
                || items.as_array().is_some_and(|items| {
                    // Versioned CRUD accepts mixed entries in these two lists.
                    matches!(*key, "subs" | "collections") || items.iter().all(Value::is_object)
                })
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
                value if !truthy(&value) => Vec::new(),
                Value::Object(items) => {
                    let mut entries: Vec<_> = items.into_iter().collect();
                    // Object.values visits canonical array indices before other keys.
                    entries.sort_by_key(|(key, _)| array_index(key));
                    entries.into_iter().map(|(_, value)| value).collect()
                }
                Value::Array(items) => items,
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
                    if let Some(processes) = object.get_mut("process")
                        && truthy(processes)
                        && !processes.as_array().is_some_and(Vec::is_empty)
                        && processes["length"].as_f64() != Some(0.0)
                    {
                        let items = match std::mem::take(processes) {
                            Value::Array(items) => items,
                            Value::String(_) => Vec::new(),
                            _ => return Err(io::Error::other("Invalid legacy processes")),
                        };
                        let mut quick = json!({"type":"Quick Setting Operator", "args":{
                            "udp":"DEFAULT", "tfo":"DEFAULT", "scert":"DEFAULT",
                            "vmess aead":"DEFAULT", "useless":"DEFAULT"
                        }});
                        let mut next = Vec::new();
                        for mut process in items {
                            if process.is_null() {
                                return Err(io::Error::other("Invalid legacy process"));
                            }
                            if !truthy(&process["type"]) {
                                continue;
                            }
                            if let Some(kinds) = process["type"].as_array() {
                                if !kinds.iter().any(|kind| kind == "Keyword") {
                                    next.push(process);
                                }
                                continue;
                            }
                            let Some(kind) = process["type"].as_str() else {
                                return Err(io::Error::other("Invalid legacy process type"));
                            };
                            if kind == "Useless Filter" {
                                quick["args"]["useless"] = json!("ENABLED");
                            } else if kind == "Set Property Operator" {
                                if process["args"].is_null() {
                                    return Err(io::Error::other("Missing legacy operator args"));
                                }
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
                        *processes = Value::Array(next);
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
