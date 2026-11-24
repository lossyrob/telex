"""Exercise the built CLI's platform TLS verifier without installing roots or running PostgreSQL."""
import concurrent.futures
import os
from pathlib import Path
import socket
import ssl
import struct
import subprocess
import sys
import tempfile


def openssl(directory, *args):
    subprocess.run(["openssl", *args], cwd=directory, capture_output=True, check=True)


def certificates(directory):
    for name in ("trusted", "unrelated"):
        openssl(directory, "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
                "-subj", f"/CN=Telex test {name}", "-addext", "basicConstraints=critical,CA:TRUE",
                "-keyout", name + ".key", "-out", name + ".pem")
    openssl(directory, "req", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost",
            "-keyout", "server.key", "-out", "server.csr")
    (directory / "server.ext").write_text(
        "subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n"
        "basicConstraints=critical,CA:FALSE\n")
    openssl(directory, "x509", "-req", "-in", "server.csr", "-CA", "trusted.pem",
            "-CAkey", "trusted.key", "-CAcreateserial", "-days", "2",
            "-extfile", "server.ext", "-out", "server.pem")
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(directory / "server.pem", directory / "server.key")
    return ctx


def serve(listener, context):
    # Observe the PostgreSQL StartupMessage after TLS verification, before authentication.
    with listener.accept()[0] as conn:
        conn.settimeout(5)
        with conn.makefile("rb") as raw:
            assert raw.read(8) == struct.pack("!II", 8, 80877103)
        if context is None:
            conn.sendall(b"N")
            assert conn.recv(1) == b""
            return False
        conn.sendall(b"S")
        try:
            with context.wrap_socket(conn, server_side=True) as tls, tls.makefile("rb") as stream:
                length = stream.read(4)
                if not length:
                    return False
                packet_length = struct.unpack("!I", length)[0]
                assert 8 <= packet_length < 10000
                startup = stream.read(packet_length - 4)
                assert startup.startswith(struct.pack("!I", 196608))
                assert b"user\0synthetic\0" in startup
                return True
        except (ssl.SSLError, ConnectionResetError, ConnectionAbortedError):
            return False


def reject_bad_profiles(binary, directory):
    env = {**os.environ, "TELEX_HOME": str(directory),
           "TELEX_CONFIG": str(directory / "invalid.toml")}
    (directory / "bad.pem").write_text("not a certificate")
    for name, mode, expected in [
        ("missing.pem", "require", "resolving TLS CA file"),
        ("bad.pem", "require", "parsing TLS CA certificate"),
        ("trusted.pem", "prefer", "requires sslmode=require"),
        ("trusted.pem", "disable", "requires sslmode=require"),
    ]:
        result = subprocess.run(
            [binary, "backend", "add", "bad", "--postgres", f"host=localhost sslmode={mode}",
             "--tls-ca-file", name], cwd=directory, env=env, capture_output=True, text=True)
        assert result.returncode and expected in result.stderr, result.stderr
        assert not (directory / "invalid.toml").exists()


def run(binary, directory, context, hostname, ca, expect_startup):
    with socket.socket() as listener, concurrent.futures.ThreadPoolExecutor(1) as pool:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        listener.settimeout(8)
        port = listener.getsockname()[1]
        dsn = (f"host={hostname} hostaddr=127.0.0.1 port={port} user=synthetic "
               "dbname=fixture sslmode=require connect_timeout=3")
        env = {**os.environ, "TELEX_HOME": str(directory), "TELEX_BACKEND": "test",
               "TELEX_CONFIG": str(directory / "config.toml"), "TEST_PG_PASSWORD": "fixture"}
        args = [binary, "backend", "add", "test", "--postgres", dsn,
                "--password-env", "TEST_PG_PASSWORD"]
        if ca:
            args.extend(["--tls-ca-file", str(directory / ca)])
        subprocess.run(args, env=env, check=True, capture_output=True, timeout=10)
        server = pool.submit(serve, listener, context)
        result = subprocess.run([binary, "init"], env=env, capture_output=True, timeout=12)
        # Even the trusted endpoint intentionally closes after receiving StartupMessage.
        assert result.returncode != 0
        assert server.result() == expect_startup, (hostname, ca, result.stderr)
        print(f"{hostname} ca={ca} tls={bool(context)} startup={expect_startup}", flush=True)


if __name__ == "__main__":
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        ctx = certificates(directory)
        reject_bad_profiles(binary, directory)
        run(binary, directory, ctx, "127.0.0.1", "trusted.pem", True)
        run(binary, directory, ctx, "localhost", "trusted.pem", True)
        run(binary, directory, ctx, "wrong.invalid", "trusted.pem", False)
        run(binary, directory, ctx, "localhost", "unrelated.pem", False)
        run(binary, directory, ctx, "localhost", None, False)
        run(binary, directory, None, "localhost", "trusted.pem", False)
