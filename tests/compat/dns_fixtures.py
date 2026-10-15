"""Small independent DNS wire fixtures; not a general resolver/parser."""
import base64
import ipaddress
import struct
from pathlib import Path
import socket
import socketserver
import ssl
import threading
import time


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


def question_name(packet):
    labels, offset = [], 12
    while packet[offset]:
        length = packet[offset]
        labels.append(packet[offset + 1:offset + 1 + length].decode("ascii"))
        offset += length + 1
    return ".".join(labels)


class ControlledDns:
    """Owned loopback listeners; independently record packet bytes and peer closure."""

    def __init__(self):
        self.requests, self.connections, self.errors, self.servers = [], [], [], []
        self.urls = {}
        self.condition, self.active = threading.Condition(), 0

    def wait_for_idle(self):
        with self.condition:
            if not self.condition.wait_for(lambda: self.active == 0, timeout=3):
                raise AssertionError("Owned DNS fixture still has active handlers")

    def __enter__(self):
        owner = self
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        fixtures = Path(__file__).parent / "fixtures"
        context.load_cert_chain(fixtures / "dns-cert.pem", fixtures / "dns-key.pem")
        context.set_servername_callback(lambda sock, name, _: setattr(sock, "fixture_server_name", name))

        def receive(sock, size):
            result = b""
            while len(result) < size:
                part = sock.recv(size - len(result))
                if not part:
                    raise EOFError("Fixture received an incomplete DNS query")
                result += part
            return result

        class Handler(socketserver.BaseRequestHandler):
            def handle(self):
                with owner.condition:
                    owner.active += 1
                transport = self.server.transport
                stream = transport not in ("udp", "udp6")
                connection = None
                sock = self.request if stream else self.request[1]
                try:
                    if stream:
                        connection = {"transport": transport, "handshake": None, "server_name": None, "closed": False}
                        owner.connections.append(connection)
                        sock.settimeout(3)
                        if transport == "tls":
                            sock = context.wrap_socket(sock, server_side=True, do_handshake_on_connect=False)
                            try:
                                sock.do_handshake()
                                connection["handshake"] = True
                            except ssl.SSLError:
                                connection["handshake"] = False
                                return
                            finally:
                                connection["server_name"] = getattr(sock, "fixture_server_name", None)
                        length = receive(sock, 2)
                        packet = receive(sock, int.from_bytes(length, "big"))
                    else:
                        length, packet = b"", self.request[0]
                    domain = question_name(packet)
                    mode = next((name for name in ("end-header", "end-body", "malformed", "timeout", "fragment", "extra", "empty")
                                 if domain.startswith(name)), "good")
                    if domain == "fallback.fixture" and transport == "udp":
                        mode = "empty"
                    record = {"transport": transport, "packet": list(packet), "frame": list(length), "domain": domain}
                    owner.requests.append(record)
                    response = response_bytes(packet, mode)
                    if mode == "timeout":
                        time.sleep(0.18)
                    if stream:
                        framed = len(response).to_bytes(2, "big") + response
                        if mode == "end-header":
                            framed = framed[:1]
                        elif mode == "end-body":
                            framed = framed[:11]
                        elif mode == "extra":
                            framed += b"\0\2\0\1"  # An invalid second frame must be ignored.
                        sizes = [1, 1, 3, 3, 3, len(framed) - 11] if mode == "fragment" else [len(framed)]
                        record["response_chunks"] = sizes
                        start, offset = time.monotonic(), 0
                        for index, size in enumerate(sizes):
                            if index:
                                time.sleep(0.06)
                            try:
                                sock.sendall(framed[offset:offset + size])
                            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError, ssl.SSLEOFError):
                                if mode != "timeout":
                                    raise
                                connection["closed"] = True
                            offset += size
                        record["response_elapsed"] = time.monotonic() - start
                        if mode.startswith("end-"):
                            sock.shutdown(socket.SHUT_WR)
                        try:
                            connection["closed"] = sock.recv(1) == b""
                        except (ConnectionResetError, ConnectionAbortedError, ssl.SSLEOFError):
                            connection["closed"] = True
                    else:
                        sock.sendto(response, self.client_address)
                except Exception as error:
                    owner.errors.append(f"{transport}: {type(error).__name__}: {error}")
                finally:
                    if stream:
                        sock.close()
                    with owner.condition:
                        owner.active -= 1
                        owner.condition.notify_all()

        try:
            for transport, family, protocol in (("udp", socket.AF_INET, "udp"), ("udp6", socket.AF_INET6, "udp"),
                                               ("tcp", socket.AF_INET, "tcp"), ("tcp6", socket.AF_INET6, "tcp"),
                                               ("tls", socket.AF_INET, "tls")):
                base = socketserver.ThreadingUDPServer if protocol == "udp" else socketserver.ThreadingTCPServer
                server_type = type("DnsFixtureServer", (base,), {"address_family": family, "daemon_threads": True,
                                                                "transport": transport})
                host = "::1" if family == socket.AF_INET6 else "127.0.0.1"
                server = server_type((host, 0), Handler)
                thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
                thread.start()
                self.servers.append((server, thread))
                self.urls[transport] = f"{protocol}://{'[' + host + ']' if ':' in host else host}:{server.server_address[1]}"
            self.urls["tlsName"] = self.urls["tls"].replace("127.0.0.1", "localhost")
            self.urls["udpName"] = self.urls["udp"].replace("127.0.0.1", "localhost")
            return self
        except Exception:
            self.__exit__()
            raise

    def __exit__(self, *args):
        for server, thread in self.servers:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
