use crate::{Result, json, process::sha256, require, server::Environment};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

const DATABASES: [(&str, &str); 2] = [
    ("country", "GeoIP2-Country-Test.mmdb"),
    ("asn", "GeoLite2-ASN-Test.mmdb"),
];
fn fixtures() -> PathBuf {
    crate::compat().join("fixtures/mmdb")
}

pub fn verify(directory: &Path) -> Result<Value> {
    let manifest = json::read(directory.join("provenance.json"))?;
    let required: BTreeSet<_> = ["LICENSE-MIT".to_owned()]
        .into_iter()
        .chain(
            DATABASES
                .iter()
                .flat_map(|(_, name)| [name.to_string(), name.replace(".mmdb", ".json")]),
        )
        .collect();
    let files = manifest["files"]
        .as_array()
        .ok_or("Missing MMDB fixture provenance")?;
    let names = files
        .iter()
        .map(|item| item["file"].as_str().ok_or("Missing MMDB filename"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    require(
        manifest["commit"] == "c3ce82bec7835e41d91f1757d5ac50815328010c"
            && files.len() == required.len()
            && names == required.iter().map(String::as_str).collect(),
        "MMDB provenance must include pinned databases, source JSON and license",
    )?;
    for item in files {
        let raw = std::fs::read(directory.join(item["file"].as_str().unwrap()))?;
        require(
            item["bytes"].as_u64() == Some(raw.len() as u64) && item["sha256"] == sha256(&raw),
            "MMDB fixture differs from pinned source",
        )?;
    }
    Ok(manifest)
}

fn prefix(network: &str, ip: IpAddr) -> Result<(bool, String)> {
    let (address, bits) = network
        .split_once('/')
        .ok_or("Missing MMDB network prefix")?;
    let address = address.parse::<IpAddr>()?;
    let prefix = bits.parse::<u32>()?;
    let (base, candidate, width) = match (address, ip) {
        (IpAddr::V4(base), IpAddr::V4(ip)) => {
            (u128::from(u32::from(base)), u128::from(u32::from(ip)), 32)
        }
        (IpAddr::V6(base), IpAddr::V6(ip)) => (u128::from(base), u128::from(ip), 128),
        _ => return Ok((false, String::new())),
    };
    require(prefix <= width, "Invalid MMDB network prefix")?;
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (width - prefix)
    };
    let canonical = base & mask;
    let address = if width == 32 {
        IpAddr::V4(Ipv4Addr::from(canonical as u32))
    } else {
        IpAddr::V6(Ipv6Addr::from(canonical))
    };
    Ok((
        canonical == (candidate & mask),
        format!("{address}/{prefix}"),
    ))
}

pub fn vectors() -> Result<Value> {
    let mut result = json!({});
    for (kind, addresses) in [
        (
            "country",
            ["81.2.69.160", "175.16.199.1", "2001:218::1", "2a02:d500::1"],
        ),
        (
            "asn",
            ["1.0.0.1", "1.128.0.1", "2001:1700::1", "12.81.96.1"],
        ),
    ] {
        let name = DATABASES
            .iter()
            .find(|(key, _)| *key == kind)
            .unwrap()
            .1
            .replace(".mmdb", ".json");
        let source = json::read(fixtures().join(name))?;
        let mut rows = Vec::new();
        for raw in addresses {
            let ip = raw.parse::<IpAddr>()?;
            let mut candidates = vec![ip];
            if let IpAddr::V4(ip) = ip {
                candidates.push(IpAddr::V6(Ipv6Addr::from(u128::from(u32::from(ip)))));
            }
            let mut found = Vec::new();
            for item in source.as_array().ok_or("Invalid MMDB source vectors")? {
                for (network, record) in item.as_object().ok_or("Invalid MMDB source record")? {
                    for candidate in &candidates {
                        let (matches, network) = prefix(network, *candidate)?;
                        if matches {
                            let fields = record
                                .as_object()
                                .ok_or("Invalid MMDB source fields")?
                                .iter()
                                .filter(|(key, _)| {
                                    [
                                        "country",
                                        "autonomous_system_number",
                                        "autonomous_system_organization",
                                    ]
                                    .contains(&key.as_str())
                                })
                                .map(|(key, value)| (key.clone(), value.clone()))
                                .collect::<serde_json::Map<_, _>>();
                            found.push((network, fields));
                            break;
                        }
                    }
                }
            }
            require(
                found.len() == 1,
                "Expected exactly one independent MMDB source prefix",
            )?;
            let (network, record) = found.pop().unwrap();
            rows.push(json!({"ip":raw,"network":network,"record":record}));
        }
        result[kind] = json!(rows);
    }
    Ok(result)
}

pub fn prepare(state: &Path) -> Result<Environment> {
    verify(&fixtures())?;
    let mut environment = Environment::new();
    for (kind, name) in DATABASES {
        let path = state.join(format!("mmdb {kind}🙂.mmdb"));
        std::fs::copy(fixtures().join(name), &path)?;
        environment.insert(
            format!("SUB_STORE_MMDB_{}_PATH", kind.to_uppercase()).into(),
            path.into_os_string(),
        );
    }
    std::fs::copy(
        fixtures().join(DATABASES[0].1),
        state.join("mmdb transient🙂.mmdb"),
    )?;
    std::fs::write(state.join("mmdb corrupt.mmdb"), b"not-mmdb")?;
    json::write_new(state.join("mmdb-vectors.json"), &vectors()?)?;
    Ok(environment)
}

pub fn check_state(state: &Path) -> Result<Value> {
    for (kind, name) in DATABASES {
        require(
            std::fs::read(state.join(format!("mmdb {kind}🙂.mmdb")))?
                == std::fs::read(fixtures().join(name))?,
            "MMDB input changed",
        )?;
    }
    json::check(
        &json::read(state.join("mmdb-vectors.json"))?,
        &vectors()?,
        false,
    )?;
    require(
        std::fs::read(state.join("mmdb corrupt.mmdb"))? == b"not-mmdb",
        "Corrupt MMDB fixture changed",
    )?;
    require(
        !state.join("mmdb transient🙂.mmdb").exists(),
        "Eager-read fixture was not removed",
    )?;
    let mut hashes = serde_json::Map::new();
    let mut paths = std::fs::read_dir(state)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .ok_or("Missing MMDB state filename")?
            .to_string_lossy();
        if name.starts_with("mmdb") {
            hashes.insert(name.into_owned(), json!(sha256(&std::fs::read(path)?)));
        }
    }
    Ok(Value::Object(hashes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmdb_inputs_reject_corruption_omission_and_changed_provenance() {
        for mutation in [
            "bytes",
            "missing-file",
            "missing-entry",
            "duplicate-entry",
            "commit",
        ] {
            let target = crate::process::scratch("mmdb-mutation").unwrap();
            for entry in std::fs::read_dir(fixtures()).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_file() {
                    std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
                }
            }
            let mut manifest = verify(&target).unwrap();
            match mutation {
                "bytes" => {
                    let path = target.join(DATABASES[0].1);
                    let mut data = std::fs::read(&path).unwrap();
                    data[0] ^= 1;
                    std::fs::write(path, data).unwrap();
                }
                "missing-file" => std::fs::remove_file(target.join(DATABASES[1].1)).unwrap(),
                _ => {
                    match mutation {
                        "missing-entry" => {
                            manifest["files"].as_array_mut().unwrap().pop();
                        }
                        "duplicate-entry" => {
                            let entry = manifest["files"][0].clone();
                            manifest["files"].as_array_mut().unwrap().push(entry);
                        }
                        _ => manifest["commit"] = json!("wrong"),
                    }
                    std::fs::write(target.join("provenance.json"), json::stringify(&manifest))
                        .unwrap();
                }
            }
            assert!(
                verify(&target).is_err(),
                "MMDB mutation {mutation} accepted"
            );
        }
    }

    #[test]
    fn mmdb_vectors_use_independent_sources_and_preserve_owned_copies() {
        let state = crate::process::scratch("mmdb-owned").unwrap();
        let expected = vectors().unwrap();
        assert_eq!(
            expected["country"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["record"]["country"]["iso_code"].clone())
                .collect::<Vec<_>>(),
            vec![json!("GB"), json!("CN"), json!("JP"), Value::Null]
        );
        assert_eq!(
            expected["asn"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["record"]["autonomous_system_number"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![15169, 1221, 6730, 7018]
        );
        assert!(
            expected["asn"][3]["record"]
                .get("autonomous_system_organization")
                .is_none()
        );
        let environment = prepare(&state).unwrap();
        assert_eq!(environment.len(), 2);
        std::fs::remove_file(state.join("mmdb transient🙂.mmdb")).unwrap();
        check_state(&state).unwrap();
        std::fs::write(state.join("mmdb country🙂.mmdb"), b"changed").unwrap();
        assert!(check_state(&state).is_err());
    }
}
