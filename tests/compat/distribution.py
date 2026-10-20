"""Package CI deliveries and verify their actual extracted payloads."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import tarfile
import tomllib
import traceback
import zipfile

from reference import BACKEND, HERE, ROOT, execute, source_provenance


TARGETS = {
    "linux-x64": ("Linux", "x86_64", "x86_64-unknown-linux-gnu"),
    "linux-arm64": ("Linux", "aarch64", "aarch64-unknown-linux-gnu"),
    "windows-x64": ("Windows", "AMD64", "x86_64-pc-windows-msvc"),
}


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write_manifest(directory, metadata):
    files = {path.relative_to(directory).as_posix(): digest(path)
             for path in sorted(directory.rglob("*")) if path.is_file()}
    assert "manifest.json" not in files, "Delivery manifest already exists"
    manifest = {**metadata, "files": files}
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest


def verify_payload(directory, manifest):
    assert json.loads((directory / "manifest.json").read_text(encoding="utf-8")) == manifest, "Delivery manifest changed"
    assert not any(path.is_symlink() for path in directory.rglob("*")), "Unexpected delivery symlink"
    files = {path.relative_to(directory).as_posix(): digest(path)
             for path in sorted(directory.rglob("*")) if path.is_file() and path != directory / "manifest.json"}
    assert files == manifest["files"], "Delivery payload is missing, modified or has extra files"


def make_archive(directory, publish, target):
    if target.startswith("windows-"):
        archive = publish / (directory.name + ".zip")
        with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED) as output:
            for path in sorted(directory.rglob("*")):
                if path.is_file():
                    output.write(path, path.relative_to(directory.parent).as_posix())
    else:
        archive = publish / (directory.name + ".tar.gz")
        with tarfile.open(archive, "x:gz") as output:
            output.add(directory, arcname=directory.name)
    return archive


def unpack_archive(archive, destination):
    destination.mkdir()
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as source:
            source.extractall(destination)
    else:
        with tarfile.open(archive) as source:
            source.extractall(destination, filter="data")


def package_payload(payload, target, metadata):
    publish = payload.parent / "publish"
    publish.mkdir()
    source = metadata["source_commit"]
    repository = metadata["repository"]
    (payload / "SOURCE.md").write_text(
        f"Source: https://github.com/{repository}/tree/{source}\n\n"
        f"Corresponding source archive: https://github.com/{repository}/archive/{source}.tar.gz\n\n"
        "Build instructions: rust/README.md in that exact source. Cargo.lock and vendored utility manifests pin dependencies.\n"
        + (f"\nUnmodified frontend source: https://github.com/sub-store-org/Sub-Store-Front-End/tree/{metadata['frontend_commit']}\n"
           if metadata.get("frontend_commit") else ""), encoding="utf-8")
    manifest = write_manifest(payload, metadata)
    verify_payload(payload, manifest)
    archive = make_archive(payload, publish, target)
    extracted = payload.parent / "extracted"
    unpack_archive(archive, extracted)
    unpacked = extracted / payload.name
    verify_payload(unpacked, manifest)
    checksum = digest(archive)
    (publish / "SHA256SUMS").write_text(f"{checksum}  {archive.name}\n", encoding="ascii")
    (publish / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return unpacked, {"manifest": manifest, "archive_name": archive.name, "archive_bytes": archive.stat().st_size,
                      "archive_sha256": checksum, "extracted_payload_verified": True}


def delivery_source(environment, evidence):
    source = source_provenance(environment, evidence)
    result = execute(["git", "ls-files", "--", "LICENSE", "rust", "tests/compat",
                      ".github/workflows/rust-compat-native.yml"], ROOT, environment, evidence, "distribution-inputs")
    assert result.returncode == 0, "Could not identify delivery build inputs"
    return {**source, "source_commit": source["backend_commit"], "repository": environment["GITHUB_REPOSITORY"],
            "rust_version": tomllib.loads((ROOT / "rust/Cargo.toml").read_text(encoding="utf-8"))["package"]["version"],
            "frontend_protocol_version": json.loads((BACKEND / "package.json").read_text(encoding="utf-8"))["version"],
            "build_input_sha256": {path: digest(ROOT / path) for path in result.stdout.splitlines()},
            "cargo_lock_sha256": digest(ROOT / "rust/Cargo.lock"),
            "workflow_run": environment["GITHUB_RUN_ID"], "workflow_attempt": environment["GITHUB_RUN_ATTEMPT"]}


def binary_imports(binary, target):
    content = binary.read_bytes()
    if target.startswith("linux-"):
        assert content[:6] == b"\x7fELF\x02\x01", "Expected a 64-bit little-endian ELF executable"
        machine = struct.unpack_from("<H", content, 18)[0]
        assert machine == {"linux-x64": 62, "linux-arm64": 183}[target], "Wrong ELF architecture"
        return {"format": "ELF64", "machine": machine}
    assert content[:2] == b"MZ", "Expected a PE executable"
    pe = struct.unpack_from("<I", content, 60)[0]
    assert content[pe:pe + 4] == b"PE\0\0", "Invalid PE signature"
    machine, section_count = struct.unpack_from("<HH", content, pe + 4)
    assert machine == 0x8664, "Wrong PE architecture"
    optional_size = struct.unpack_from("<H", content, pe + 20)[0]
    optional = pe + 24
    assert struct.unpack_from("<H", content, optional)[0] == 0x20B, "Expected PE32+"
    sections = [struct.unpack_from("<IIII", content, optional + optional_size + i * 40 + 8)
                for i in range(section_count)]

    def offset(rva):
        for virtual_size, address, raw_size, raw_offset in sections:
            if address <= rva < address + max(virtual_size, raw_size):
                return raw_offset + rva - address
        raise AssertionError(f"Unmapped PE import RVA: {rva}")

    imports_rva = struct.unpack_from("<I", content, optional + 120)[0]
    imports = []
    if imports_rva:
        position = offset(imports_rva)
        while any(content[position:position + 20]):
            name_rva = struct.unpack_from("<I", content, position + 12)[0]
            start = offset(name_rva)
            imports.append(content[start:content.index(0, start)].decode("ascii"))
            position += 20
    assert imports, "No PE import table found"
    assert not any(re.match(r"(?:vcruntime|msvcp|msvcr)\d", name, re.I) for name in imports), f"Delivery needs a VC redistributable DLL: {imports}"
    return {"format": "PE32+", "machine": machine, "imports": sorted(imports), "vc_redist_required": False}


def collect_notices(metadata, directory, sysroot):
    notices = directory / "licenses"
    notices.mkdir()
    shutil.copyfile(ROOT / "LICENSE", directory / "LICENSE")
    vendor = notices / "embedded-utilities"
    vendor.mkdir()
    utility_files = sorted((ROOT / "rust/vendor").glob("*LICENSE*.txt"))
    assert utility_files, "Missing embedded utility licenses"
    for path in utility_files:
        shutil.copyfile(path, vendor / path.name)
    toolchain = sysroot / "share/doc/rust"
    rust_notices = [path for path in toolchain.rglob("*") if path.is_file()
                    and (path.name.startswith(("LICENSE", "COPYRIGHT")) or "licenses" in path.relative_to(toolchain).parts)]
    assert rust_notices, f"Missing Rust toolchain notices: {toolchain}"
    for path in rust_notices:
        destination = notices / "rust-toolchain" / path.relative_to(toolchain)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, destination)
    graph = {node["id"]: node["dependencies"] for node in metadata["resolve"]["nodes"]}
    reachable, pending = set(), [metadata["resolve"]["root"]]
    while pending:
        package_id = pending.pop()
        if package_id not in reachable:
            reachable.add(package_id)
            pending.extend(graph[package_id])
    assert reachable <= {package["id"] for package in metadata["packages"]}, "Incomplete Cargo package metadata"
    crates, missing = [], []
    for package in metadata["packages"]:
        if package["id"] not in reachable or package["id"] == metadata["resolve"]["root"]:
            continue
        base = Path(package["manifest_path"]).parent
        paths = {path for path in base.rglob("*") if path.is_file()
                 and path.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "COPYRIGHT"))}
        if package.get("license_file"):
            paths.add(base / package["license_file"])
        files = {path.relative_to(base): path for path in paths}
        workspace_notice = None
        # These two published crates omit their shared upstream workspace license.
        if package["name"] in ("rquickjs-core", "rquickjs-sys") and package["version"] == "0.14.0":
            assert package["license"] == "MIT" and package["repository"] == "https://github.com/DelSkayn/rquickjs.git", "Unexpected rquickjs license source"
            workspace_notice = HERE / "fixtures/licenses/rquickjs-v0.14.0-LICENSE"
            assert digest(workspace_notice) == "976ad3d07927343ab99b31510625acba89eac8e0e517c712925620ddeda91b70", "Pinned rquickjs notice changed"
            files[Path("workspace-LICENSE")] = workspace_notice
        if not files:
            missing.append(f"{package['name']} {package['version']}")
            continue
        label = f"{package['name']}-{package['version']}"
        copied = []
        for relative, path in sorted(files.items()):
            destination = notices / "crates" / label / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, destination)
            copied.append(destination.relative_to(directory).as_posix())
        crates.append({"name": package["name"], "version": package["version"],
                       "license": package.get("license"), "source": package.get("source"), "notices": copied,
                       "workspace_notice_source": "https://github.com/DelSkayn/rquickjs/blob/v0.14.0/LICENSE" if workspace_notice else None})
    assert not missing, "Missing dependency notices: " + ", ".join(missing)
    (directory / "third-party.json").write_text(json.dumps({
        "scope": "Reachable locked dependencies in Cargo's target-filtered resolve graph", "crates": crates,
        "embedded_notices": [path.name for path in utility_files]}, indent=2) + "\n", encoding="utf-8")
    return len(crates)


def native(args, environment, evidence, report):
    expected_os, expected_machine, triple = TARGETS[args.target]
    assert platform.system() == expected_os and platform.machine() == expected_machine, "Wrong native distribution runner"
    args.output.mkdir(parents=True)

    def command(argv, label):
        result = execute(argv, ROOT, environment, evidence, label, timeout=120)
        assert result.returncode == 0, f"Distribution command failed: {label}"
        return result.stdout.strip()

    source = delivery_source(environment, evidence)
    report.update(source)
    toolchain = command(["rustc", "-vV"], "rustc-version")
    assert f"host: {triple}" in toolchain, "Wrong compiler host"
    metadata = json.loads(command(["cargo", "metadata", "--manifest-path", "rust/Cargo.toml", "--locked",
                                   "--format-version", "1", "--filter-platform", triple], "cargo-metadata"))
    package = next(item for item in metadata["packages"] if item["id"] == metadata["resolve"]["root"])
    payload = args.output / (f"sub-store-rust-{package['version']}-{args.target}")
    payload.mkdir()
    binary = payload / ("sub-store-rust.exe" if expected_os == "Windows" else "sub-store-rust")
    shutil.copy2(args.binary, binary)
    shutil.copyfile(ROOT / "rust/README.md", payload / "README.md")
    shutil.copyfile(ROOT / "rust/Cargo.lock", payload / "Cargo.lock")
    linkage = binary_imports(binary, args.target)
    if expected_os == "Windows":
        assert "+crt-static" in environment.get("RUSTFLAGS", ""), "Static CRT build flag missing"
    else:
        dependencies = command(["ldd", str(binary)], "dynamic-dependencies")
        assert "not found" not in dependencies, "Unresolved native library"
        versions = command(["readelf", "--version-info", str(binary)], "elf-version-requirements")
        linkage.update(dynamic_libraries=dependencies.splitlines(),
                       glibc_versions=sorted(set(re.findall(r"\bGLIBC_[0-9.]+", versions))))
    sysroot = Path(command(["rustc", "--print", "sysroot"], "rust-sysroot"))
    report.update(target_triple=triple, toolchain=toolchain,
                  rustflags=environment.get("RUSTFLAGS", ""), linkage=linkage,
                  toolchain_notice_directory=str(sysroot / "share/doc/rust"), python_version=platform.python_version())
    crate_count = collect_notices(metadata, payload, sysroot)
    unpacked, delivery = package_payload(payload, args.target, {**source, "kind": "rust-native-distribution",
        "target": args.target, "target_triple": triple, "os": platform.system(), "machine": platform.machine(),
        "toolchain": toolchain, "rustflags": environment.get("RUSTFLAGS", ""), "linkage": linkage,
        "dependency_notice_count": crate_count})
    delivered_binary = unpacked / binary.name
    assert digest(delivered_binary) == digest(args.binary), "Delivered binary differs from the release build"
    report.update(**delivery, extracted_binary=str(delivered_binary),
                  binary_sha256=digest(delivered_binary))
    with Path(environment["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        output.write(f"binary={delivered_binary}\npublish={args.output / 'publish'}\n")


def docker_delivery(image, environment):
    target = {"x86_64": "linux-x64", "aarch64": "linux-arm64"}[platform.machine()]
    evidence = HERE / "evidence/distribution-docker"
    evidence.mkdir()
    report = {"kind": "docker-distribution", "exit_code": 1, "target": target}

    def command(argv, label, timeout=120):
        result = execute(argv, ROOT, environment, evidence, label, timeout=timeout)
        assert result.returncode == 0, f"Image delivery command failed: {label}"
        return result.stdout.strip()

    try:
        source = delivery_source(environment, evidence)
        inspected = json.loads(command(["docker", "image", "inspect", image], "image-before"))[0]
        expected_arch = {"linux-x64": "amd64", "linux-arm64": "arm64"}[target]
        assert inspected["Os"] == "linux" and inspected["Architecture"] == expected_arch, "Wrong delivered image platform"
        report.update(source, image_id=inspected["Id"], architecture=expected_arch)
        root = ROOT / ".compat-delivery" / ("docker-" + target)
        root.mkdir(parents=True)
        payload = root / f"sub-store-rust-{source['rust_version']}-docker-{target}"
        payload.mkdir()
        shutil.copyfile(ROOT / "LICENSE", payload / "LICENSE")
        shutil.copyfile(ROOT / "rust/README.md", payload / "README.md")
        shutil.copyfile(ROOT / "rust/Cargo.lock", payload / "Cargo.lock")
        container = command(["docker", "create", image], "notice-container")
        try:
            command(["docker", "cp", container + ":/usr/share/licenses/sub-store", str(payload / "licenses")], "image-notices")
        finally:
            command(["docker", "rm", container], "remove-notice-container")
        notices = json.loads((payload / "licenses/third-party.json").read_text(encoding="utf-8"))
        assert notices["crates"] and (payload / "licenses/licenses/embedded-utilities/CRON-LICENSE.txt").is_file(), "Incomplete image notices"
        toolchain = (payload / "licenses/toolchain.txt").read_text(encoding="utf-8")
        triple = (payload / "licenses/target.txt").read_text(encoding="utf-8").strip()
        assert triple == TARGETS[target][2], "Wrong image compiler target"
        command(["docker", "save", "--output", str(payload / "image.tar"), image], "image-save", timeout=300)
        unpacked, delivery = package_payload(payload, target, {**source, "kind": "rust-docker-distribution",
            "target": target, "target_triple": triple, "os": "linux", "architecture": expected_arch,
            "image_tag": image, "image_id": inspected["Id"], "toolchain": toolchain,
            "docker_version": command(["docker", "version", "--format", "{{json .}}"], "docker-version"),
            "dependency_notice_count": len(notices["crates"])})
        report.update(delivery)
        command(["docker", "image", "rm", image], "remove-built-image")
        command(["docker", "load", "--input", str(unpacked / "image.tar")], "image-load", timeout=300)
        loaded = json.loads(command(["docker", "image", "inspect", image], "image-after"))[0]
        assert loaded["Id"] == inspected["Id"] and loaded["Architecture"] == expected_arch and loaded["Os"] == "linux", "Reloaded delivery changed image identity/platform"
        report.update(reloaded_image_id=loaded["Id"], saved_image_reloaded=True, exit_code=0)
        with Path(environment["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
            output.write(f"publish={root / 'publish'}\n")
        return report
    except Exception as error:
        report.update(error=f"{type(error).__name__}: {error}", traceback=traceback.format_exc())
        raise
    finally:
        (evidence / "summary.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


def frontend_delivery(args, environment, evidence, report):
    frontend = args.frontend.resolve(strict=True)
    revision = execute(["git", "rev-parse", "HEAD"], frontend, environment, evidence, "frontend-commit")
    clean = execute(["git", "diff", "--exit-code", "HEAD"], frontend, environment, evidence, "frontend-source")
    commit = json.loads((HERE / "manifest.json").read_text(encoding="utf-8"))["frontend_commit"]
    assert revision.returncode == 0 and revision.stdout.strip() == commit and clean.returncode == 0, "Frontend source changed"
    assert (frontend / "dist/index.html").is_file(), "Build the unchanged frontend first"
    source = delivery_source(environment, evidence)
    args.output.mkdir(parents=True)
    payload = args.output / ("sub-store-frontend-" + commit[:12])
    payload.mkdir()
    shutil.copytree(frontend / "dist", payload / "dist")
    for name in ("LICENSE", "package.json", "pnpm-lock.yaml"):
        shutil.copyfile(frontend / name, payload / name)
    shutil.copyfile(ROOT / "rust/README.md", payload / "README.md")
    licenses = execute(["pnpm", "licenses", "list", "--json"], frontend, environment, evidence, "frontend-dependency-licenses", timeout=120)
    assert licenses.returncode == 0, "Could not collect frontend dependency licenses"
    dependencies = json.loads(licenses.stdout)
    (payload / "third-party.json").write_text(json.dumps(dependencies, indent=2) + "\n", encoding="utf-8")
    package_files = [*frontend.glob("node_modules/.pnpm/*/node_modules/*/package.json"),
                     *frontend.glob("node_modules/.pnpm/*/node_modules/@*/*/package.json")]
    copied = set()
    for package_file in sorted(package_files):
        if package_file.is_symlink():
            continue
        package = json.loads(package_file.read_text(encoding="utf-8"))
        label = package["name"].replace("/", "__") + "-" + package["version"]
        if label in copied:
            continue
        copied.add(label)
        for path in package_file.parent.iterdir():
            if path.is_file() and path.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "COPYRIGHT")):
                destination = payload / "licenses" / label / path.name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, destination)
    assets = {path.relative_to(frontend / "dist").as_posix(): digest(path)
              for path in sorted((frontend / "dist").rglob("*")) if path.is_file()}
    asset_hash = hashlib.sha256(json.dumps(assets, sort_keys=True).encode("utf-8")).hexdigest()
    unpacked, delivery = package_payload(payload, "linux-x64", {**source, "kind": "unchanged-frontend-distribution",
        "frontend_commit": commit, "frontend_artifact_sha256": asset_hash, "assets": assets,
        "installed_package_notice_scope": "Original available notice files from every installed pnpm package; full dependency license report included"})
    extracted = {path.relative_to(unpacked / "dist").as_posix(): digest(path)
                 for path in sorted((unpacked / "dist").rglob("*")) if path.is_file()}
    assert extracted == assets, "Delivered frontend differs from the unchanged build"
    report.update(source, **delivery, frontend_commit=commit, frontend_artifact_sha256=asset_hash, extracted_assets=str(unpacked / "dist"))
    with Path(environment["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        output.write(f"assets={unpacked / 'dist'}\npublish={args.output / 'publish'}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    kind = parser.add_mutually_exclusive_group(required=True)
    kind.add_argument("--target", choices=TARGETS)
    kind.add_argument("--frontend", type=Path)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.target and not args.binary:
        parser.error("--target requires --binary")
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Package and execute product deliveries in GitHub Actions only")
    label = "native" if args.target else "frontend"
    evidence = HERE / "evidence" / ("distribution-" + label)
    evidence.mkdir()
    report = {"kind": label + "-distribution", "exit_code": 1, "target": args.target}
    try:
        if args.target:
            native(args, os.environ.copy(), evidence, report)
        else:
            frontend_delivery(args, os.environ.copy(), evidence, report)
        report["exit_code"] = 0
    except Exception as error:
        report.update(error=f"{type(error).__name__}: {error}", traceback=traceback.format_exc())
    finally:
        (evidence / "summary.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"kind": report["kind"], "exit_code": report["exit_code"], "error": report.get("error")}))
    return report["exit_code"]


if __name__ == "__main__":
    if not __debug__:
        raise SystemExit("Distribution verification requires enabled assertions")
    raise SystemExit(main())
