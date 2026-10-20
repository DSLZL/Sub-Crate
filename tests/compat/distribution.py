"""Package the exact CI release binary and verify its extracted delivery."""
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
    crates = []
    for package in metadata["packages"]:
        if package["id"] not in reachable or package["id"] == metadata["resolve"]["root"]:
            continue
        base = Path(package["manifest_path"]).parent
        paths = {path for path in base.rglob("*") if path.is_file()
                 and path.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "COPYRIGHT"))}
        if package.get("license_file"):
            paths.add(base / package["license_file"])
        assert paths, f"Missing dependency notices: {package['name']} {package['version']}"
        label = f"{package['name']}-{package['version']}"
        copied = []
        for path in sorted(paths):
            relative = path.relative_to(base)
            destination = notices / "crates" / label / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, destination)
            copied.append(destination.relative_to(directory).as_posix())
        crates.append({"name": package["name"], "version": package["version"],
                       "license": package.get("license"), "source": package.get("source"), "notices": copied})
    (directory / "third-party.json").write_text(json.dumps({
        "scope": "Reachable locked dependencies in Cargo's target-filtered resolve graph", "crates": crates,
        "embedded_notices": [path.name for path in utility_files]}, indent=2) + "\n", encoding="utf-8")
    return len(crates)


def native(args, environment, evidence, report):
    expected_os, expected_machine, triple = TARGETS[args.target]
    assert platform.system() == expected_os and platform.machine() == expected_machine, "Wrong native distribution runner"
    publish = args.output / "publish"
    publish.mkdir(parents=True)

    def command(argv, label):
        result = execute(argv, ROOT, environment, evidence, label, timeout=120)
        assert result.returncode == 0, f"Distribution command failed: {label}"
        return result.stdout.strip()

    report.update(source_provenance(environment, evidence))
    head = command(["git", "rev-parse", "HEAD"], "distribution-head")
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
    report.update(source_commit=head, target_triple=triple, toolchain=toolchain,
                  rustflags=environment.get("RUSTFLAGS", ""), linkage=linkage,
                  toolchain_notice_directory=str(sysroot / "share/doc/rust"), python_version=platform.python_version())
    crate_count = collect_notices(metadata, payload, sysroot)
    inputs = command(["git", "ls-files", "--", "LICENSE", "rust", "tests/compat",
                      ".github/workflows/rust-compat-native.yml"], "distribution-inputs").splitlines()
    source_url = f"https://github.com/{environment['GITHUB_REPOSITORY']}/tree/{head}"
    (payload / "SOURCE.md").write_text(
        f"Source: {source_url}\n\nCorresponding source archive: https://github.com/{environment['GITHUB_REPOSITORY']}/archive/{head}.tar.gz\n"
        "\nBuild instructions: rust/README.md in that exact source. Cargo.lock and vendored utility manifests pin dependencies.\n",
        encoding="utf-8")
    manifest = write_manifest(payload, {"kind": "rust-native-distribution", "source_commit": head,
        "source_url": source_url, "rust_version": package["version"],
        "frontend_protocol_version": json.loads((BACKEND / "package.json").read_text(encoding="utf-8"))["version"],
        "target": args.target, "target_triple": triple, "os": platform.system(), "machine": platform.machine(),
        "toolchain": toolchain, "rustflags": environment.get("RUSTFLAGS", ""), "linkage": linkage,
        "build_input_sha256": {path: digest(ROOT / path) for path in inputs},
        "cargo_lock_sha256": digest(ROOT / "rust/Cargo.lock"), "dependency_notice_count": crate_count,
        "workflow_run": environment["GITHUB_RUN_ID"], "workflow_attempt": environment["GITHUB_RUN_ATTEMPT"]})
    archive = make_archive(payload, publish, args.target)
    extracted = args.output / "extracted"
    unpack_archive(archive, extracted)
    unpacked = extracted / payload.name
    verify_payload(unpacked, manifest)
    delivered_binary = unpacked / binary.name
    assert digest(delivered_binary) == digest(args.binary), "Delivered binary differs from the release build"
    checksum = digest(archive)
    (publish / "SHA256SUMS").write_text(f"{checksum}  {archive.name}\n", encoding="ascii")
    (publish / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    report.update(manifest=manifest, archive_name=archive.name, archive_bytes=archive.stat().st_size,
                  archive_sha256=checksum, extracted_binary=str(delivered_binary),
                  binary_sha256=digest(delivered_binary), extracted_payload_verified=True)
    with Path(environment["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        output.write(f"binary={delivered_binary}\npublish={publish}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Package and execute product deliveries in GitHub Actions only")
    evidence = HERE / "evidence" / "distribution-native"
    evidence.mkdir()
    report = {"kind": "native-distribution", "exit_code": 1, "target": args.target}
    try:
        native(args, os.environ.copy(), evidence, report)
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
