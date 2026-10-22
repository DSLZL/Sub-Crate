use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut found=Vec::new();
    if !directory.is_dir() { return Ok(found); }
    for entry in std::fs::read_dir(directory)? {
        let entry=entry?; let kind=entry.file_type()?;
        if kind.is_dir() { found.extend(files(&entry.path())?); }
        else if kind.is_file() { found.push(entry.path()); }
    }
    found.sort(); Ok(found)
}

fn copy(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination.parent().ok_or("Missing notice destination parent")?)?;
    std::fs::copy(source,destination)?; Ok(())
}

pub fn collect(metadata: &Value, directory: &Path, sysroot: &Path, root: &Path, pinned_notice: &Path) -> Result<usize> {
    let notices=directory.join("licenses"); std::fs::create_dir(&notices)?;
    copy(&root.join("LICENSE"),&directory.join("LICENSE"))?;
    let utility_files=files(&root.join("rust/vendor"))?.into_iter().filter(|path| path.parent()==Some(root.join("rust/vendor").as_path()) && path.file_name().unwrap().to_string_lossy().contains("LICENSE") && path.extension().is_some_and(|extension| extension=="txt")).collect::<Vec<_>>();
    if utility_files.is_empty() { return Err("Missing embedded utility licenses".into()); }
    for path in &utility_files { copy(path,&notices.join("embedded-utilities").join(path.file_name().unwrap()))?; }
    let toolchain=sysroot.join("share/doc/rust");
    let rust_notices=files(&toolchain)?.into_iter().filter(|path| { let name=path.file_name().unwrap().to_string_lossy(); name.starts_with("LICENSE") || name.starts_with("COPYRIGHT") || path.strip_prefix(&toolchain).unwrap().components().any(|part| part.as_os_str()=="licenses") }).collect::<Vec<_>>();
    if rust_notices.is_empty() { return Err("Missing Rust toolchain notices".into()); }
    for path in rust_notices { copy(&path,&notices.join("rust-toolchain").join(path.strip_prefix(&toolchain)?))?; }
    let resolve=&metadata["resolve"];
    let root_id=resolve["root"].as_str().ok_or("Missing target-filtered Cargo resolve root")?;
    let graph=resolve["nodes"].as_array().ok_or("Missing Cargo resolve nodes")?.iter().map(|node| Ok((node["id"].as_str().ok_or("Missing Cargo node ID")?,node["dependencies"].as_array().ok_or("Missing Cargo dependencies")?))).collect::<Result<HashMap<_,_>>>()?;
    let mut reachable=HashSet::new(); let mut pending=vec![root_id];
    while let Some(id)=pending.pop() { if reachable.insert(id) { pending.extend(graph.get(id).ok_or("Incomplete Cargo resolve graph")?.iter().map(|id| id.as_str().ok_or("Invalid Cargo dependency ID")).collect::<std::result::Result<Vec<_>,_>>()?); } }
    let packages=metadata["packages"].as_array().ok_or("Missing Cargo package metadata")?;
    if !reachable.iter().all(|id| packages.iter().any(|package| package["id"]==*id)) { return Err("Incomplete Cargo package metadata".into()); }
    let mut crates=Vec::new(); let mut missing=Vec::new();
    for package in packages {
        let id=package["id"].as_str().ok_or("Missing Cargo package ID")?;
        if !reachable.contains(id) || id==root_id { continue; }
        let name=package["name"].as_str().ok_or("Missing Cargo package name")?;
        let version=package["version"].as_str().ok_or("Missing Cargo package version")?;
        let base=Path::new(package["manifest_path"].as_str().ok_or("Missing Cargo package manifest")?).parent().ok_or("Invalid Cargo package manifest")?;
        let paths=files(base)?.into_iter().filter(|path| { let name=path.file_name().unwrap().to_string_lossy().to_uppercase(); ["LICENSE","COPYING","NOTICE","COPYRIGHT"].iter().any(|prefix| name.starts_with(prefix)) }).collect::<Vec<_>>();
        let mut sources=paths.iter().map(|path| Ok((path.strip_prefix(base)?.to_owned(),path.clone()))).collect::<Result<BTreeMap<_,_>>>()?;
        if let Some(license)=package["license_file"].as_str() { let path=Path::new(license); if path.is_absolute() || path.components().any(|part| matches!(part,std::path::Component::ParentDir)) { return Err("Invalid Cargo license file path".into()); } sources.insert(path.to_owned(),base.join(path)); }
        let workspace_notice=["rquickjs-core","rquickjs-sys"].contains(&name) && version=="0.14.0";
        if workspace_notice {
            if package["license"]!="MIT" || package["repository"]!="https://github.com/DelSkayn/rquickjs.git" { return Err("Unexpected rquickjs license source".into()); }
            let digest=aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256,&std::fs::read(pinned_notice)?).as_ref().iter().map(|byte| format!("{byte:02x}")).collect::<String>();
            if digest!="976ad3d07927343ab99b31510625acba89eac8e0e517c712925620ddeda91b70" { return Err("Pinned rquickjs notice changed".into()); }
            sources.insert(PathBuf::from("workspace-LICENSE"),pinned_notice.to_owned());
        }
        if sources.is_empty() { missing.push(format!("{name} {version}")); continue; }
        let mut copied=Vec::new();
        for (relative,path) in sources {
            let destination=notices.join("crates").join(format!("{name}-{version}")).join(relative); copy(&path,&destination)?;
            copied.push(destination.strip_prefix(directory)?.to_string_lossy().replace('\\',"/"));
        }
        crates.push(json!({"name":name,"version":version,"license":package.get("license"),"source":package.get("source"),"notices":copied,"workspace_notice_source":if workspace_notice { Some("https://github.com/DelSkayn/rquickjs/blob/v0.14.0/LICENSE") } else { None }}));
    }
    if !missing.is_empty() { return Err(format!("Missing dependency notices: {}",missing.join(", ")).into()); }
    let report=json!({"scope":"Reachable locked dependencies in Cargo's target-filtered resolve graph","crates":crates,"embedded_notices":utility_files.iter().map(|path| path.file_name().unwrap().to_string_lossy()).collect::<Vec<_>>()});
    use std::io::Write;
    let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(directory.join("third-party.json"))?;
    file.write_all(serde_json::to_string_pretty(&report)?.as_bytes())?; file.write_all(b"\n")?;
    Ok(crates.len())
}
