use crate::{Result, json, notices, process, require};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        require(!kind.is_symlink(), "Unexpected delivery symlink")?;
        if kind.is_dir() {
            paths.extend(files(&entry.path())?);
        } else if kind.is_file() {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}
pub fn digest(path: impl AsRef<Path>) -> Result<String> {
    Ok(process::sha256(&std::fs::read(path)?))
}
fn hashes(directory: &Path) -> Result<BTreeMap<String, String>> {
    files(directory)?
        .into_iter()
        .filter(|path| path != &directory.join("manifest.json"))
        .map(|path| {
            Ok((
                path.strip_prefix(directory)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                digest(&path)?,
            ))
        })
        .collect()
}
pub fn asset_hashes(directory: &Path) -> Result<BTreeMap<String, String>> {
    files(directory)?
        .into_iter()
        .map(|path| {
            Ok((
                path.strip_prefix(directory)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                digest(&path)?,
            ))
        })
        .collect()
}
pub(crate) fn copy_tree(source: &Path, destination: &Path) -> Result {
    for path in files(source)? {
        let target = destination.join(path.strip_prefix(source)?);
        std::fs::create_dir_all(target.parent().ok_or("Missing copy parent")?)?;
        std::fs::copy(path, target)?;
    }
    Ok(())
}

fn native_notices(source: &Path, payload: &Path) -> Result {
    use std::io::Write;
    std::fs::copy(source.join("LICENSE"), payload.join("LICENSE"))?;
    let mut combined = std::fs::File::create_new(payload.join("THIRD_PARTY_NOTICES.txt"))?;
    for path in files(&source.join("licenses"))? {
        writeln!(
            combined,
            "\n===== {} =====\n",
            path.strip_prefix(source)?
                .to_string_lossy()
                .replace('\\', "/")
        )?;
        std::io::copy(&mut std::fs::File::open(path)?, &mut combined)?;
        writeln!(combined)?;
    }
    Ok(())
}

pub fn frontend(output: &Path) -> Result<PathBuf> {
    let frontend =
        PathBuf::from(std::env::var_os("TEST_FRONTEND").ok_or("Missing frontend checkout")?)
            .canonicalize()?;
    let commit = crate::frontend::COMMIT;
    for (label, args, expected) in [
        ("frontend-commit", vec!["rev-parse", "HEAD"], commit),
        ("frontend-source", vec!["diff", "--exit-code", "HEAD"], ""),
    ] {
        let actual = process::checked(
            Command::new("git").args(args).current_dir(&frontend),
            output,
            label,
            Duration::from_secs(30),
        )?;
        require(actual.stdout.trim() == expected, "Frontend source changed")?;
    }
    require(
        frontend.join("dist/index.html").is_file(),
        "Build unchanged frontend first",
    )?;
    let mut source = process::provenance(output)?;
    source["source_commit"] = source["backend_commit"].clone();
    source["repository"] = json!(std::env::var("GITHUB_REPOSITORY")?);
    let payload = output.join(format!("sub-store-frontend-{}", &commit[..12]));
    std::fs::create_dir(&payload)?;
    copy_tree(&frontend.join("dist"), &payload.join("dist"))?;
    for file in ["LICENSE", "package.json", "pnpm-lock.yaml"] {
        std::fs::copy(frontend.join(file), payload.join(file))?;
    }
    for file in ["README.md", "ACCEPTANCE.md"] {
        std::fs::copy(crate::root().join("rust").join(file), payload.join(file))?;
    }
    let licenses = process::checked(
        Command::new("pnpm")
            .args(["licenses", "list", "--json"])
            .current_dir(&frontend),
        output,
        "frontend-dependency-licenses",
        Duration::from_secs(120),
    )?;
    json::write_new(
        payload.join("third-party.json"),
        &json::parse(&licenses.stdout)?,
    )?;
    let mut copied = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(frontend.join("node_modules/.pnpm"))? {
        let directory = entry?.path().join("node_modules");
        if !directory.is_dir() {
            continue;
        }
        let mut packages = Vec::new();
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                continue;
            }
            if entry.file_name().to_string_lossy().starts_with('@') {
                for package in std::fs::read_dir(entry.path())? {
                    let package = package?;
                    if !package.file_type()?.is_symlink() {
                        packages.push(package.path());
                    }
                }
            } else {
                packages.push(entry.path());
            }
        }
        for directory in packages {
            let file = directory.join("package.json");
            if !file.is_file() || file.is_symlink() {
                continue;
            }
            let package = json::read(file)?;
            let label = format!(
                "{}-{}",
                package["name"]
                    .as_str()
                    .ok_or("Missing dependency name")?
                    .replace('/', "__"),
                package["version"]
                    .as_str()
                    .ok_or("Missing dependency version")?
            );
            require(
                !label.contains("..") && !label.contains(['/', '\\']),
                "Invalid dependency notice path",
            )?;
            if !copied.insert(label.clone()) {
                continue;
            }
            for file in std::fs::read_dir(&directory)? {
                let file = file?;
                let name = file.file_name().to_string_lossy().to_uppercase();
                if file.file_type()?.is_file()
                    && ["LICENSE", "COPYING", "NOTICE", "COPYRIGHT"]
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
                {
                    let destination = payload.join("licenses").join(&label);
                    std::fs::create_dir_all(&destination)?;
                    std::fs::copy(file.path(), destination.join(file.file_name()))?;
                }
            }
        }
    }
    let assets = asset_hashes(&frontend.join("dist"))?;
    source["kind"] = json!("unchanged-frontend-distribution");
    source["frontend_commit"] = json!(commit);
    source["assets"] = json!(assets);
    source["frontend_artifact_sha256"] =
        json!(process::sha256(json::stringify(&json!(assets)).as_bytes()));
    source["installed_package_notice_scope"] = json!(
        "Original available notice files from every installed pnpm package; full dependency license report included"
    );
    let (unpacked, report) = package_payload(&payload, "linux-x64", &source)?;
    let extracted = unpacked.join("dist");
    json::check(&json!(asset_hashes(&extracted)?), &json!(assets), false)?;
    json::write_new(output.join("distribution.json"), &report)?;
    if let Some(path) = std::env::var_os("GITHUB_OUTPUT") {
        use std::io::Write;
        writeln!(
            std::fs::OpenOptions::new().append(true).open(path)?,
            "assets={}\npublish={}",
            extracted.display(),
            output.join("publish").display()
        )?;
    }
    Ok(extracted)
}
pub fn write_manifest(directory: &Path, metadata: &Value) -> Result<Value> {
    require(
        !directory.join("manifest.json").exists(),
        "Delivery manifest already exists",
    )?;
    let mut manifest = metadata.clone();
    manifest["files"] = json!(hashes(directory)?);
    if metadata["kind"] != "rust-native-distribution" {
        json::write_new(directory.join("manifest.json"), &manifest)?;
    }
    Ok(manifest)
}
pub fn verify_payload(directory: &Path, manifest: &Value) -> Result {
    if manifest["kind"] == "rust-native-distribution" {
        require(
            !directory.join("manifest.json").exists(),
            "Native package contains an internal manifest",
        )?;
    } else {
        json::check(
            &json::read(directory.join("manifest.json"))?,
            manifest,
            false,
        )?;
    }
    json::check(&json!(hashes(directory)?), &manifest["files"], false)
}

pub fn package_payload(payload: &Path, target: &str, metadata: &Value) -> Result<(PathBuf, Value)> {
    let parent = payload.parent().ok_or("Missing payload parent")?;
    let publish = parent.join("publish");
    std::fs::create_dir(&publish)?;
    let source = metadata["source_commit"]
        .as_str()
        .ok_or("Missing source commit")?;
    let repository = metadata["repository"]
        .as_str()
        .ok_or("Missing source repository")?;
    let mut text = format!(
        "Source: https://github.com/{repository}/tree/{source}\n\nCorresponding source archive: https://github.com/{repository}/archive/{source}.tar.gz\n\nBuild instructions: rust/README.md in that exact source. Cargo.lock and vendored utility manifests pin dependencies.\n"
    );
    if let Some(frontend) = metadata["frontend_commit"].as_str() {
        text.push_str(&format!("\nUnmodified frontend source: https://github.com/sub-store-org/Sub-Store-Front-End/tree/{frontend}\n"));
    }
    std::fs::write(payload.join("SOURCE.md"), text)?;
    let manifest = write_manifest(payload, metadata)?;
    verify_payload(payload, &manifest)?;
    let name = payload.file_name().ok_or("Missing payload name")?;
    let archive = publish.join(format!(
        "{}.{}",
        name.to_string_lossy(),
        if target.starts_with("windows-") {
            "zip"
        } else {
            "tar.gz"
        }
    ));
    require(!archive.exists(), "Delivery archive already exists")?;
    let mut command = Command::new("cmake");
    command
        .args([
            "-E",
            "tar",
            if target.starts_with("windows-") {
                "cf"
            } else {
                "czf"
            },
        ])
        .arg(&archive)
        .arg(if target.starts_with("windows-") {
            "--format=zip"
        } else {
            "--format=gnutar"
        })
        .arg(name)
        .current_dir(parent);
    process::checked(
        &mut command,
        parent,
        "archive-create",
        Duration::from_secs(120),
    )?;
    let extracted = parent.join("extracted");
    std::fs::create_dir(&extracted)?;
    // Only archives just created from the verified, symlink-free payload are extracted here.
    process::checked(
        Command::new("cmake")
            .args(["-E", "tar", "xf"])
            .arg(&archive)
            .current_dir(&extracted),
        parent,
        "archive-extract",
        Duration::from_secs(120),
    )?;
    let unpacked = extracted.join(name);
    verify_payload(&unpacked, &manifest)?;
    let checksum = digest(&archive)?;
    let archive_name = archive.file_name().unwrap().to_string_lossy();
    std::fs::write(
        publish.join("SHA256SUMS"),
        format!("{checksum}  {archive_name}\n"),
    )?;
    json::write_new(publish.join("manifest.json"), &manifest)?;
    Ok((
        unpacked,
        json!({"manifest":manifest,"archive_name":archive_name,"archive_bytes":archive.metadata()?.len(),"archive_sha256":checksum,"extracted_payload_verified":true}),
    ))
}

pub fn binary_imports(binary: &Path, target: &str) -> Result<Value> {
    let data = std::fs::read(binary)?;
    let u16_at = |offset: usize| -> Result<u16> {
        Ok(u16::from_le_bytes(
            data.get(offset..offset + 2)
                .ok_or("Truncated executable")?
                .try_into()?,
        ))
    };
    let u32_at = |offset: usize| -> Result<usize> {
        Ok(u32::from_le_bytes(
            data.get(offset..offset + 4)
                .ok_or("Truncated executable")?
                .try_into()?,
        ) as usize)
    };
    if target.starts_with("linux-") {
        require(
            data.get(..6) == Some(b"\x7fELF\x02\x01"),
            "Expected 64-bit little-endian ELF",
        )?;
        let machine = u16_at(18)?;
        require(
            machine
                == match target {
                    "linux-x64" => 62,
                    "linux-arm64" => 183,
                    _ => return Err("Unknown ELF target".into()),
                },
            "Wrong ELF architecture",
        )?;
        return Ok(json!({"format":"ELF64","machine":machine}));
    }
    require(
        target == "windows-x64" && data.get(..2) == Some(b"MZ"),
        "Expected Windows PE executable",
    )?;
    let pe = u32_at(60)?;
    require(
        data.get(pe..pe + 4) == Some(b"PE\0\0"),
        "Invalid PE signature",
    )?;
    let machine = u16_at(pe + 4)?;
    require(machine == 0x8664, "Wrong PE architecture")?;
    let count = usize::from(u16_at(pe + 6)?);
    let optional = pe + 24;
    let size = usize::from(u16_at(pe + 20)?);
    require(u16_at(optional)? == 0x20b, "Expected PE32+")?;
    let mut sections = Vec::new();
    for index in 0..count {
        let offset = optional + size + index * 40 + 8;
        sections.push((
            u32_at(offset)?,
            u32_at(offset + 4)?,
            u32_at(offset + 8)?,
            u32_at(offset + 12)?,
        ));
    }
    let offset = |rva: usize| -> Result<usize> {
        sections
            .iter()
            .find_map(|(virtual_size, address, raw_size, raw_offset)| {
                (rva >= *address && rva < address + virtual_size.max(raw_size))
                    .then(|| raw_offset + rva - address)
            })
            .ok_or_else(|| "Unmapped PE import RVA".into())
    };
    let imports_rva = u32_at(optional + 120)?;
    let mut imports = Vec::new();
    if imports_rva != 0 {
        let mut position = offset(imports_rva)?;
        while data
            .get(position..position + 20)
            .ok_or("Truncated PE import table")?
            .iter()
            .any(|byte| *byte != 0)
        {
            let start = offset(u32_at(position + 12)?)?;
            let length = data
                .get(start..)
                .ok_or("Truncated PE import name")?
                .iter()
                .position(|byte| *byte == 0)
                .ok_or("Unterminated PE import name")?;
            let name = std::str::from_utf8(&data[start..start + length])?;
            require(name.is_ascii(), "Invalid PE import name")?;
            imports.push(name.to_owned());
            position += 20;
        }
    }
    require(!imports.is_empty(), "No PE import table found")?;
    let redist = regex::Regex::new(r"(?i)\A(?:vcruntime|msvcp|msvcr)\d")?;
    require(
        !imports.iter().any(|name| redist.is_match(name)),
        "Delivery requires a VC redistributable DLL",
    )?;
    imports.sort();
    Ok(json!({"format":"PE32+","machine":machine,"imports":imports,"vc_redist_required":false}))
}

pub fn native(output: &Path, binary: &Path) -> Result<PathBuf> {
    let (target, triple) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("linux-x64", "x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => ("linux-arm64", "aarch64-unknown-linux-gnu"),
        ("windows", "x86_64") => ("windows-x64", "x86_64-pc-windows-msvc"),
        _ => return Err("Unsupported native distribution runner".into()),
    };
    let command = |program: &str, args: &[&str], label: &str| -> Result<String> {
        Ok(process::checked(
            Command::new(program).args(args).current_dir(crate::root()),
            output,
            label,
            Duration::from_secs(120),
        )?
        .stdout
        .trim()
        .to_owned())
    };
    let mut source = process::provenance(output)?;
    let commit = source["backend_commit"].clone();
    source["source_commit"] = commit;
    source["repository"] = json!(std::env::var("GITHUB_REPOSITORY")?);
    let inputs = command(
        "git",
        &[
            "ls-files",
            "--",
            "LICENSE",
            ".dockerignore",
            "rust",
            "tests/compat",
            "tests/src",
            "tests/Cargo.toml",
            "tests/Cargo.lock",
            ".config/nextest.toml",
            ".github/actions/node-reference",
            ".github/workflows/rust-compat-native.yml",
            ".github/workflows/rust-tests.yml",
        ],
        "distribution-inputs",
    )?;
    source["build_input_sha256"] = json!(
        inputs
            .lines()
            .map(|path| Ok((path.to_owned(), digest(crate::root().join(path))?)))
            .collect::<Result<BTreeMap<_, _>>>()?
    );
    source["cargo_lock_sha256"] = json!(digest(crate::root().join("rust/Cargo.lock"))?);
    source["workflow_run"] = json!(std::env::var("GITHUB_RUN_ID")?);
    source["workflow_attempt"] = json!(std::env::var("GITHUB_RUN_ATTEMPT")?);
    source["frontend_protocol_version"] =
        json::read(crate::root().join("backend/package.json"))?["version"].clone();
    let toolchain = command("rustc", &["-vV"], "rustc-version")?;
    require(
        toolchain.contains(&format!("host: {triple}")),
        "Wrong native compiler host",
    )?;
    let metadata = json::parse(&command(
        "cargo",
        &[
            "metadata",
            "--manifest-path",
            "rust/Cargo.toml",
            "--locked",
            "--format-version",
            "1",
            "--filter-platform",
            triple,
        ],
        "cargo-metadata",
    )?)?;
    let package = metadata["packages"]
        .as_array()
        .ok_or("Missing Cargo packages")?
        .iter()
        .find(|package| package["id"] == metadata["resolve"]["root"])
        .ok_or("Missing application Cargo metadata")?;
    source["rust_version"] = package["version"].clone();
    let payload = output.join(format!(
        "sub-store-rust-{}-{target}",
        package["version"].as_str().ok_or("Missing version")?
    ));
    std::fs::create_dir(&payload)?;
    let filename = if cfg!(windows) {
        "sub-store-rust.exe"
    } else {
        "sub-store-rust"
    };
    std::fs::copy(binary, payload.join(filename))?;
    std::fs::copy(
        crate::root().join("rust/RELEASE-README.md"),
        payload.join("README.md"),
    )?;
    let mut linkage = binary_imports(&payload.join(filename), target)?;
    let flags = std::env::var("RUSTFLAGS").unwrap_or_default();
    if cfg!(windows) {
        require(
            flags.contains("+crt-static"),
            "Static CRT build flag missing",
        )?;
    } else {
        let dependencies = process::checked(
            Command::new("ldd").arg(payload.join(filename)),
            output,
            "dynamic-dependencies",
            Duration::from_secs(30),
        )?
        .stdout;
        require(
            !dependencies.contains("not found"),
            "Unresolved native library",
        )?;
        let versions = process::checked(
            Command::new("readelf")
                .arg("--version-info")
                .arg(payload.join(filename)),
            output,
            "elf-version-requirements",
            Duration::from_secs(30),
        )?
        .stdout;
        linkage["dynamic_libraries"] = json!(dependencies.lines().collect::<Vec<_>>());
        linkage["glibc_versions"] = json!(
            regex::Regex::new(r"\bGLIBC_[0-9.]+")?
                .find_iter(&versions)
                .map(|found| found.as_str())
                .collect::<std::collections::BTreeSet<_>>()
        );
    }
    let sysroot = PathBuf::from(command("rustc", &["--print", "sysroot"], "rust-sysroot")?);
    let notice_source = output.join("dependency-notices");
    std::fs::create_dir(&notice_source)?;
    let count = notices::collect(
        &metadata,
        &notice_source,
        &sysroot,
        &crate::root(),
        &crate::compat().join("fixtures/licenses/rquickjs-v0.14.0-LICENSE"),
    )?;
    native_notices(&notice_source, &payload)?;
    source["kind"] = json!("rust-native-distribution");
    source["target"] = json!(target);
    source["target_triple"] = json!(triple);
    source["os"] = json!(std::env::consts::OS);
    source["machine"] = json!(std::env::consts::ARCH);
    source["toolchain"] = json!(toolchain);
    source["rustflags"] = json!(flags);
    source["linkage"] = linkage;
    source["dependency_notice_count"] = json!(count);
    let (unpacked, report) = package_payload(&payload, target, &source)?;
    require(
        report["manifest"]["files"]
            .as_object()
            .is_some_and(|files| {
                files.len() == 5
                    && [
                        filename,
                        "README.md",
                        "LICENSE",
                        "SOURCE.md",
                        "THIRD_PARTY_NOTICES.txt",
                    ]
                    .iter()
                    .all(|name| files.contains_key(*name))
            }),
        "Unexpected native delivery files",
    )?;
    let delivered = unpacked.join(filename);
    require(
        digest(&delivered)? == digest(binary)?,
        "Delivered binary differs from release build",
    )?;
    json::write_new(output.join("distribution.json"), &report)?;
    if let Some(path) = std::env::var_os("GITHUB_OUTPUT") {
        use std::io::Write;
        writeln!(
            std::fs::OpenOptions::new().append(true).open(path)?,
            "binary={}\npublish={}",
            delivered.display(),
            output.join("publish").display()
        )?;
    }
    Ok(delivered)
}

pub fn docker(output: &Path, image: &str) -> Result {
    let (target, triple, architecture) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("linux-x64", "x86_64-unknown-linux-gnu", "amd64"),
        ("linux", "aarch64") => ("linux-arm64", "aarch64-unknown-linux-gnu", "arm64"),
        _ => return Err("Docker delivery requires a native Linux runner".into()),
    };
    require(
        image.starts_with("sub-store-nextest:"),
        "Refusing to replace an image not owned by this acceptance run",
    )?;
    let command = |args: &[&str], label: &str, timeout: u64| -> Result<String> {
        Ok(process::checked(
            Command::new("docker").args(args).current_dir(crate::root()),
            output,
            label,
            Duration::from_secs(timeout),
        )?
        .stdout
        .trim()
        .to_owned())
    };
    let inspected =
        json::parse(&command(&["image", "inspect", image], "image-before", 120)?)?[0].clone();
    require(
        inspected["Os"] == "linux" && inspected["Architecture"] == architecture,
        "Wrong delivered image platform",
    )?;
    let mut source = process::provenance(output)?;
    source["source_commit"] = source["backend_commit"].clone();
    source["repository"] = json!(std::env::var("GITHUB_REPOSITORY")?);
    let inputs = process::checked(
        Command::new("git")
            .args([
                "ls-files",
                "--",
                "LICENSE",
                "rust",
                "tests/compat",
                "tests/src",
                "tests/Cargo.toml",
                "tests/Cargo.lock",
                ".config/nextest.toml",
                ".github/actions/node-reference",
                ".github/workflows/rust-compat-native.yml",
            ])
            .current_dir(crate::root()),
        output,
        "docker-build-inputs",
        Duration::from_secs(30),
    )?
    .stdout;
    source["build_input_sha256"] = json!(
        inputs
            .lines()
            .map(|path| Ok((path.to_owned(), digest(crate::root().join(path))?)))
            .collect::<Result<BTreeMap<_, _>>>()?
    );
    source["cargo_lock_sha256"] = json!(digest(crate::root().join("rust/Cargo.lock"))?);
    source["workflow_run"] = json!(std::env::var("GITHUB_RUN_ID")?);
    source["workflow_attempt"] = json!(std::env::var("GITHUB_RUN_ATTEMPT")?);
    let metadata = process::checked(
        Command::new("cargo")
            .args([
                "metadata",
                "--manifest-path",
                "rust/Cargo.toml",
                "--locked",
                "--no-deps",
                "--format-version",
                "1",
            ])
            .current_dir(crate::root()),
        output,
        "docker-package-version",
        Duration::from_secs(120),
    )?
    .stdout;
    let version = json::parse(&metadata)?["packages"][0]["version"]
        .as_str()
        .ok_or("Missing image application version")?
        .to_owned();
    source["rust_version"] = json!(version);
    let payload = output.join(format!("sub-store-rust-{version}-docker-{target}"));
    std::fs::create_dir(&payload)?;
    std::fs::copy(crate::root().join("LICENSE"), payload.join("LICENSE"))?;
    for file in ["README.md", "ACCEPTANCE.md", "Cargo.lock"] {
        std::fs::copy(crate::root().join("rust").join(file), payload.join(file))?;
    }
    let container = command(&["create", image], "notice-container", 120)?;
    require(
        !container.is_empty()
            && container
                .chars()
                .all(|character| character.is_ascii_hexdigit()),
        "Invalid created notice container identity",
    )?;
    let copied = command(
        &[
            "cp",
            &format!("{container}:/usr/share/licenses/sub-store"),
            payload
                .join("licenses")
                .to_str()
                .ok_or("Invalid notice destination")?,
        ],
        "image-notices",
        120,
    );
    let cleaned = command(&["rm", &container], "remove-notice-container", 120);
    copied?;
    cleaned?;
    let notices = json::read(payload.join("licenses/third-party.json"))?;
    let count = notices["crates"]
        .as_array()
        .ok_or("Missing image dependency notices")?
        .len();
    require(
        count > 0
            && payload
                .join("licenses/licenses/embedded-utilities/CRON-LICENSE.txt")
                .is_file(),
        "Incomplete image notices",
    )?;
    let toolchain = std::fs::read_to_string(payload.join("licenses/toolchain.txt"))?;
    require(
        std::fs::read_to_string(payload.join("licenses/target.txt"))?.trim() == triple,
        "Wrong image compiler target",
    )?;
    command(
        &[
            "save",
            "--output",
            payload
                .join("image.tar")
                .to_str()
                .ok_or("Invalid image archive")?,
            image,
        ],
        "image-save",
        300,
    )?;
    source["kind"] = json!("rust-docker-distribution");
    source["target"] = json!(target);
    source["target_triple"] = json!(triple);
    source["os"] = json!("linux");
    source["architecture"] = json!(architecture);
    source["image_tag"] = json!(image);
    source["image_id"] = inspected["Id"].clone();
    source["toolchain"] = json!(toolchain);
    source["docker_version"] = json!(command(
        &["version", "--format", "{{json .}}"],
        "docker-version",
        120
    )?);
    source["dependency_notice_count"] = json!(count);
    let (unpacked, mut delivery) = package_payload(&payload, target, &source)?;
    command(&["image", "rm", image], "remove-built-image", 120)?;
    command(
        &[
            "load",
            "--input",
            unpacked
                .join("image.tar")
                .to_str()
                .ok_or("Invalid extracted image archive")?,
        ],
        "image-load",
        300,
    )?;
    let loaded =
        json::parse(&command(&["image", "inspect", image], "image-after", 120)?)?[0].clone();
    require(
        loaded["Id"] == inspected["Id"]
            && loaded["Architecture"] == architecture
            && loaded["Os"] == "linux",
        "Reloaded image changed identity/platform",
    )?;
    delivery["saved_image_reloaded"] = json!(true);
    delivery["reloaded_image_id"] = loaded["Id"].clone();
    json::write_new(output.join("docker-distribution.json"), &delivery)?;
    if let Some(path) = std::env::var_os("GITHUB_OUTPUT") {
        use std::io::Write;
        writeln!(
            std::fs::OpenOptions::new().append(true).open(path)?,
            "publish={}",
            output.join("publish").display()
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_notices_preserve_every_notice_without_shipping_development_files() {
        let root = process::scratch("native-notices").unwrap();
        let source = root.join("source");
        let payload = root.join("payload");
        std::fs::create_dir_all(source.join("licenses/crates/dependency-1.0")).unwrap();
        std::fs::create_dir_all(source.join("licenses/rust-toolchain")).unwrap();
        std::fs::create_dir(&payload).unwrap();
        std::fs::write(source.join("LICENSE"), "application license").unwrap();
        std::fs::write(source.join("third-party.json"), "development metadata").unwrap();
        let notices = [
            (
                "licenses/crates/dependency-1.0/LICENSE",
                b"dependency license\r\n\xff".as_slice(),
            ),
            (
                "licenses/rust-toolchain/COPYRIGHT",
                b"toolchain copyright".as_slice(),
            ),
        ];
        for (name, bytes) in notices {
            std::fs::write(source.join(name), bytes).unwrap();
        }
        native_notices(&source, &payload).unwrap();
        assert_eq!(
            std::fs::read(payload.join("LICENSE")).unwrap(),
            b"application license"
        );
        let combined = std::fs::read(payload.join("THIRD_PARTY_NOTICES.txt")).unwrap();
        for (name, bytes) in notices {
            assert!(
                combined
                    .windows(name.len())
                    .any(|part| part == name.as_bytes())
            );
            assert!(combined.windows(bytes.len()).any(|part| part == bytes));
        }
        assert_eq!(files(&payload).unwrap().len(), 2);
        assert!(source.join("third-party.json").is_file());
    }

    #[test]
    fn archives_reject_modified_missing_extra_payload_files_and_changed_manifest() {
        for (target, kind) in [
            ("linux-x64", "rust-native-distribution"),
            ("windows-x64", "rust-native-distribution"),
            ("linux-x64", "other-distribution"),
            ("windows-x64", "other-distribution"),
        ] {
            let root = process::scratch("archive-integrity").unwrap();
            let payload = root.join("sub-store-rust");
            std::fs::create_dir(&payload).unwrap();
            let original = b"verified executable bytes\0";
            std::fs::write(payload.join("sub-store-rust"), original).unwrap();
            std::fs::write(payload.join("LICENSE"), "license text").unwrap();
            let (unpacked,report)=package_payload(&payload,target,&json!({"source_commit":"ffffffffffffffffffffffffffffffffffffffff","repository":"example/sub-store","target":target,"kind":kind})).unwrap();
            let manifest = &report["manifest"];
            verify_payload(&unpacked, manifest).unwrap();
            assert_eq!(
                unpacked.join("manifest.json").exists(),
                kind != "rust-native-distribution"
            );
            assert_eq!(
                json::read(root.join("publish/manifest.json")).unwrap(),
                *manifest
            );
            let binary = unpacked.join("sub-store-rust");
            std::fs::write(&binary, b"tampered").unwrap();
            assert!(verify_payload(&unpacked, manifest).is_err());
            std::fs::remove_file(&binary).unwrap();
            assert!(verify_payload(&unpacked, manifest).is_err());
            std::fs::write(&binary, original).unwrap();
            let extra = unpacked.join("unexpected");
            std::fs::write(&extra, b"extra").unwrap();
            assert!(verify_payload(&unpacked, manifest).is_err());
            std::fs::remove_file(extra).unwrap();
            let mut changed = manifest.clone();
            changed["source_commit"] = json!("0000000000000000000000000000000000000000");
            std::fs::write(unpacked.join("manifest.json"), json::stringify(&changed)).unwrap();
            assert!(verify_payload(&unpacked, manifest).is_err());
        }
    }
    #[test]
    fn native_binary_rejects_the_other_elf_architecture() {
        let root = process::scratch("binary-architecture").unwrap();
        let binary = root.join("service");
        let mut data = vec![0; 64];
        data[..6].copy_from_slice(b"\x7fELF\x02\x01");
        data[18..20].copy_from_slice(&183_u16.to_le_bytes());
        std::fs::write(&binary, &data).unwrap();
        assert!(
            binary_imports(&binary, "linux-x64")
                .unwrap_err()
                .to_string()
                .contains("Wrong ELF architecture")
        );
        assert!(binary_imports(&binary, "linux-arm64").is_ok());
    }
}
