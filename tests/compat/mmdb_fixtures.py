"""Pinned public MMDB inputs and independently sourced lookup expectations."""
import hashlib
import ipaddress
import json
from pathlib import Path
import shutil
from runner import check_json

FIXTURES = Path(__file__).resolve().parent / "fixtures/mmdb"
COMMIT = "c3ce82bec7835e41d91f1757d5ac50815328010c"
DATABASES = {"country": "GeoIP2-Country-Test.mmdb", "asn": "GeoLite2-ASN-Test.mmdb"}


def verify_fixtures(directory=FIXTURES):
    manifest = json.loads((directory / "provenance.json").read_text(encoding="utf-8"))
    required = {"LICENSE-MIT", *DATABASES.values(), *(name.replace(".mmdb", ".json") for name in DATABASES.values())}
    names = [item["file"] for item in manifest["files"]]
    if manifest["commit"] != COMMIT or set(names) != required or len(names) != len(required):
        raise AssertionError("MMDB fixture provenance must include the pinned databases, source JSON and license")
    for item in manifest["files"]:
        data = (directory / item["file"]).read_bytes()
        if len(data) != item["bytes"] or hashlib.sha256(data).hexdigest() != item["sha256"]:
            raise AssertionError("MMDB fixture differs from pinned source: " + item["file"])
    return manifest


def vectors():
    result = {}
    for kind, addresses in {
        "country": ["81.2.69.160", "175.16.199.1", "2001:218::1", "2a02:d500::1"],
        "asn": ["1.0.0.1", "1.128.0.1", "2001:1700::1", "12.81.96.1"],
    }.items():
        source = json.loads((FIXTURES / DATABASES[kind].replace(".mmdb", ".json")).read_text(encoding="utf-8"))
        records = [(ipaddress.ip_network(network, strict=False), record)
                   for item in source for network, record in item.items()]
        rows = []
        for raw in addresses:
            ip = ipaddress.ip_address(raw)
            # The official country source expresses IPv4 prefixes in IPv6's low 32 bits.
            candidates = [ip] + ([ipaddress.IPv6Address(int(ip))] if ip.version == 4 else [])
            found = [(network, record) for network, record in records
                     if any(candidate.version == network.version and candidate in network for candidate in candidates)]
            assert len(found) == 1, (kind, raw, found)
            network, record = found[0]
            fields = {key: value for key, value in record.items()
                      if key in ("country", "autonomous_system_number", "autonomous_system_organization")}
            rows.append({"ip": raw, "network": str(network), "record": fields})
        result[kind] = rows
    return result


def prepare(state):
    verify_fixtures()
    for kind, name in DATABASES.items():
        shutil.copyfile(FIXTURES / name, state / f"mmdb {kind}🙂.mmdb")
    shutil.copyfile(FIXTURES / DATABASES["country"], state / "mmdb transient🙂.mmdb")
    (state / "mmdb corrupt.mmdb").write_bytes(b"not-mmdb")
    (state / "mmdb-vectors.json").write_text(json.dumps(vectors(), ensure_ascii=False), encoding="utf-8")
    return {f"SUB_STORE_MMDB_{kind.upper()}_PATH": str(state / f"mmdb {kind}🙂.mmdb") for kind in DATABASES}


def check_state(state):
    for kind, name in DATABASES.items():
        assert (state / f"mmdb {kind}🙂.mmdb").read_bytes() == (FIXTURES / name).read_bytes(), "MMDB input changed"
    check_json(json.loads((state / "mmdb-vectors.json").read_text(encoding="utf-8")), vectors())
    assert (state / "mmdb corrupt.mmdb").read_bytes() == b"not-mmdb"
    assert not (state / "mmdb transient🙂.mmdb").exists(), "Eager-read fixture was not removed"
    return {file.name: hashlib.sha256(file.read_bytes()).hexdigest() for file in sorted(state.glob("mmdb*"))}
