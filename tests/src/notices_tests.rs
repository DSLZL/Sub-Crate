use crate::{json, notices, process};
use serde_json::json;
use std::path::Path;

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}
fn seed(root: &Path) {
    write(&root.join("LICENSE"), "application license");
    write(&root.join("rust/vendor/LICENSES.txt"), "embedded license");
    write(
        &root.join("toolchain/share/doc/rust/LICENSE-MIT"),
        "toolchain license",
    );
}

#[test]
fn target_filtered_notices_include_transitive_dependencies_and_reject_missing_licenses() {
    let root = process::scratch("notices-transitive").unwrap();
    seed(&root);
    write(&root.join("dependency/Cargo.toml"), "");
    write(&root.join("dependency/LICENSE"), "dependency license");
    std::fs::create_dir(root.join("transitive")).unwrap();
    let metadata = json!({"packages":[{"id":"root"},{"id":"dependency","name":"dependency","version":"1.0.0","manifest_path":root.join("dependency/Cargo.toml"),"license":"MIT"},{"id":"transitive","name":"transitive","version":"1.0.0","manifest_path":root.join("transitive/Cargo.toml"),"license":"MIT"},{"id":"android-only","name":"android-only","version":"1.0.0","manifest_path":root.join("not-downloaded/Cargo.toml")}],"resolve":{"root":"root","nodes":[{"id":"root","dependencies":["dependency"]},{"id":"dependency","dependencies":["transitive"]},{"id":"transitive","dependencies":[]},{"id":"android-only","dependencies":[]}]}});
    let pinned = crate::compat().join("fixtures/licenses/rquickjs-v0.14.0-LICENSE");
    let failed = root.join("failed");
    std::fs::create_dir(&failed).unwrap();
    assert!(
        notices::collect(&metadata, &failed, &root.join("toolchain"), &root, &pinned)
            .unwrap_err()
            .to_string()
            .contains("Missing dependency notices")
    );
    write(&root.join("transitive/LICENSE"), "transitive license");
    let output = root.join("licensed");
    std::fs::create_dir(&output).unwrap();
    assert_eq!(
        notices::collect(&metadata, &output, &root.join("toolchain"), &root, &pinned).unwrap(),
        2
    );
    let report = json::read(output.join("third-party.json")).unwrap();
    assert_eq!(
        report["crates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["dependency", "transitive"]
    );
    let mut invalid = metadata;
    invalid["resolve"]["nodes"][1]["dependencies"] = json!(["absent"]);
    let output = root.join("incomplete");
    std::fs::create_dir(&output).unwrap();
    assert!(notices::collect(&invalid, &output, &root.join("toolchain"), &root, &pinned).is_err());
}

#[test]
fn omitted_rquickjs_workspace_license_is_pinned_to_version_repository_and_hash() {
    let root = process::scratch("notices-rquickjs").unwrap();
    seed(&root);
    std::fs::create_dir(root.join("core")).unwrap();
    let pinned = crate::compat().join("fixtures/licenses/rquickjs-v0.14.0-LICENSE");
    let metadata = json!({"packages":[{"id":"app"},{"id":"core","name":"rquickjs-core","version":"0.14.0","license":"MIT","repository":"https://github.com/DelSkayn/rquickjs.git","manifest_path":root.join("core/Cargo.toml")}],"resolve":{"root":"app","nodes":[{"id":"app","dependencies":["core"]},{"id":"core","dependencies":[]}]}});
    let output = root.join("valid");
    std::fs::create_dir(&output).unwrap();
    assert_eq!(
        notices::collect(&metadata, &output, &root.join("toolchain"), &root, &pinned).unwrap(),
        1
    );
    assert_eq!(
        process::sha256(
            &std::fs::read(output.join("licenses/crates/rquickjs-core-0.14.0/workspace-LICENSE"))
                .unwrap()
        ),
        "976ad3d07927343ab99b31510625acba89eac8e0e517c712925620ddeda91b70"
    );
    let bad = root.join("changed-LICENSE");
    std::fs::write(&bad, "changed").unwrap();
    for mutation in ["version", "repository", "license", "hash"] {
        let mut changed = metadata.clone();
        match mutation {
            "version" => changed["packages"][1]["version"] = json!("0.14.1"),
            "repository" => {
                changed["packages"][1]["repository"] = json!("https://example.invalid/wrong")
            }
            "license" => changed["packages"][1]["license"] = json!("Apache-2.0"),
            _ => (),
        }
        let output = root.join(format!("mutation-{mutation}"));
        std::fs::create_dir(&output).unwrap();
        assert!(
            notices::collect(
                &changed,
                &output,
                &root.join("toolchain"),
                &root,
                if mutation == "hash" { &bad } else { &pinned }
            )
            .is_err()
        );
    }
}
