"""Small independent DNS wire fixtures; not a general resolver/parser."""
import base64
import ipaddress
import struct


def name_bytes(domain):
    return b"".join(bytes([len(label)]) + label.encode("ascii") for label in domain.split(".")) + b"\0"


def query_bytes(domain, kind=1, edns=None):
    additional = b""
    if edns:
        address = ipaddress.ip_address(edns)
        prefix = 24 if address.version == 4 else 56
        subnet = struct.pack("!HBB", 1 if address.version == 4 else 2, prefix, 0) + address.packed[:prefix // 8]
        option = struct.pack("!HH", 8, len(subnet)) + subnet
        additional = b"\0" + struct.pack("!HHIH", 41, 4096, 0, len(option)) + option
    return struct.pack("!6H", 0, 0x100, 1, 0, 0, bool(additional)) + name_bytes(domain) + struct.pack("!HH", kind, 1) + additional


def query_path(mode, domain, kind=1, edns=None):
    encoded = base64.urlsafe_b64encode(query_bytes(domain, kind, edns)).decode("ascii").rstrip("=")
    return f"/host/dns/{mode}?dns={encoded}"


def response_bytes(packet, mode):
    offset = 12
    while packet[offset]:
        length = packet[offset]
        if length > 63:
            raise ValueError("Fixture accepts only uncompressed question labels")
        offset += length + 1
    end = offset + 5
    kind, klass = struct.unpack("!HH", packet[offset + 1:end])
    if packet[:12] != struct.pack("!6H", 0, 0x100, 1, 0, 0, struct.unpack("!H", packet[10:12])[0]) or klass != 1:
        raise ValueError("Unexpected fixture DNS query header")
    if mode == "malformed":
        return b"\0\1"
    records = []
    if mode not in ("empty", "serial-empty", "nxdomain"):
        records.append((5, name_bytes("alias.fixture")))
        answer_kind = 28 if mode == "wrong" else kind
        addresses = ["2001:db8::7"] if answer_kind == 28 else ["198.51.100.7", "198.51.100.9"]
        records.extend((answer_kind, ipaddress.ip_address(address).packed) for address in addresses)
    flags = 0x8183 if mode in ("nxdomain", "rcode") else 0x8180
    header = struct.pack("!6H", 0, flags, 1, len(records), 0, 0)
    answers = b"".join(b"\xc0\x0c" + struct.pack("!HHIH", kind, 1, 60, len(data)) + data for kind, data in records)
    return header + packet[12:end] + answers
