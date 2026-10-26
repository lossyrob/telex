"""Bounded GitHub reads and exact retained-artifact acquisition; no import-time effects."""

import datetime
import hashlib
import http.client
import ipaddress
import json
from pathlib import Path
import re
import stat
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile


API = "https://api.github.com/repos/lossyrob/telex/"
JSON_BYTES = 4 * 1024 * 1024


def require(value, message):
    if not value:
        raise RuntimeError(message)


def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=lambda value: require(False, "invalid JSON constant"))


def instant(value):
    require(isinstance(value, str) and re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", value),
            "invalid UTC timestamp")
    return datetime.datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc).timestamp()


def unexpired(pin):
    require(time.time() < instant(pin["expires_at"]), "derived artifact retention expired; return to design")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def opener():
    return urllib.request.build_opener(NoRedirect(), urllib.request.ProxyHandler({}))


def close_error(response):
    try:
        response.close()
    except (OSError, http.client.HTTPException):
        raise RuntimeError("HTTP response close failed; no retry") from None


def request_api(endpoint, token, deadline):
    require(isinstance(token, str) and token and "\r" not in token and "\n" not in token, "missing/invalid API token")
    require(isinstance(endpoint, str) and re.fullmatch(r"[A-Za-z0-9_/?=&.-]+", endpoint) and
            ".." not in endpoint and not endpoint.startswith("/"), "noncanonical API endpoint")
    require(time.monotonic() < deadline, "GitHub read deadline")
    request = urllib.request.Request(API + endpoint, headers={
        "Authorization": "Bearer " + token, "Accept": "application/vnd.github+json",
        "Accept-Encoding": "identity", "X-GitHub-Api-Version": "2022-11-28"})
    return opener().open(request, timeout=min(30, deadline - time.monotonic()))


def read_bounded(response, maximum, deadline, sink=None, progress=None):
    require(response.headers.get("Content-Encoding", "identity") == "identity", "encoded response prohibited")
    length = response.headers.get("Content-Length")
    if length is not None:
        require(re.fullmatch(r"[0-9]+", length) and int(length) <= maximum, "response length bound")
    count, digest, chunks = 0, hashlib.sha256(), []
    while True:
        require(time.monotonic() < deadline, "bounded read deadline")
        raw = response.read(min(65536, maximum + 1 - count))
        if not raw:
            break
        count += len(raw)
        digest.update(raw)
        if progress is not None:
            progress.update(observed_body_bytes=count, observed_body_sha256=digest.hexdigest(),
                            byte_precision=("OVERFLOW FAILURE EVIDENCE: cap+1 response body; not ZIP identity or file persistence."
                                            if count > maximum else "Received response body, not file persistence."))
        require(count <= maximum, "response byte bound")
        if sink is None:
            chunks.append(raw)
        else:
            sink.write(raw)
    if length is not None:
        require(count == int(length), "truncated response")
    return {"bytes": count, "sha256": digest.hexdigest()}, b"".join(chunks)


def fetch_json(endpoint, token, deadline):
    try:
        response = request_api(endpoint, token, deadline)
    except urllib.error.HTTPError as error:
        close_error(error)
        raise RuntimeError("GitHub metadata request rejected; no redirects or retry") from None
    except (OSError, urllib.error.URLError):
        raise RuntimeError("GitHub metadata transport failed; no retry") from None
    with response:
        require(response.status == 200 and response.geturl() == API + endpoint, "noncanonical GitHub metadata response")
        _, raw = read_bounded(response, JSON_BYTES, deadline)
    return strict_json(raw)


def validate_metadata(metadata, run, job, pin):
    unexpired(pin)
    require(isinstance(metadata, dict) and set(metadata) == {
        "id", "node_id", "name", "size_in_bytes", "url", "archive_download_url", "expired", "digest",
        "created_at", "updated_at", "expires_at", "workflow_run"}, "unknown/missing artifact metadata schema")
    expected = {"id": pin["artifact_id"], "name": pin["artifact_name"], "size_in_bytes": pin["archive_bytes"],
                "url": API + f"actions/artifacts/{pin['artifact_id']}",
                "archive_download_url": API + f"actions/artifacts/{pin['artifact_id']}/zip",
                "digest": "sha256:" + pin["archive_sha256"], "expires_at": pin["expires_at"],
                "created_at": pin["created_at"], "updated_at": pin["created_at"]}
    require(all(type(metadata[k]) is type(v) and metadata[k] == v for k, v in expected.items()) and
            metadata["expired"] is False and isinstance(metadata["node_id"], str) and metadata["node_id"],
            "artifact identity/retention mismatch")
    require(metadata["workflow_run"] == {
        "id": pin["run_id"], "repository_id": pin["repository_id"], "head_repository_id": pin["repository_id"],
        "head_branch": pin["branch"], "head_sha": pin["source_sha"]}, "artifact originating run mismatch")
    for data, expected in (
        (run, {"id": pin["run_id"], "run_attempt": 1, "head_sha": pin["source_sha"],
               "head_branch": pin["branch"], "path": pin["workflow"], "event": "push",
               "status": "completed", "conclusion": "success"}),
        (job, {"id": pin["job_id"], "run_id": pin["run_id"], "run_attempt": 1, "head_sha": pin["source_sha"],
               "name": "Build-derived-ipsw-arm64", "status": "completed", "conclusion": "success",
               "labels": ["macos-26"]}),
    ):
        require(isinstance(data, dict) and all(type(data.get(k)) is type(v) and data[k] == v for k, v in expected.items()),
                "artifact BUILD run/job identity mismatch")
    return {"artifact_id": pin["artifact_id"], "run_id": pin["run_id"], "job_id": pin["job_id"],
            "source_sha": pin["source_sha"], "expires_at": pin["expires_at"], "retained": True}


def download(pin, target, token, deadline, receipt=None):
    receipt = {} if receipt is None else receipt
    endpoint = f"actions/artifacts/{pin['artifact_id']}/zip"
    try:
        response = request_api(endpoint, token, deadline)
    except urllib.error.HTTPError as error:
        try:
            receipt["api_status"] = error.code
            require(error.code == 302 and error.geturl() == API + endpoint, "artifact API must supply one canonical302")
            locations = error.headers.get_all("Location", [])
            require(len(locations) == 1, "artifact storage location missing/ambiguous")
            location = locations[0]
            require(isinstance(location, str) and 0 < len(location) <= 8192 and
                    all(32 < ord(ch) < 127 for ch in location), "malformed artifact storage URL")
        finally:
            close_error(error)
    except (OSError, urllib.error.URLError):
        raise RuntimeError("artifact API transport failed; no retry") from None
    else:
        close_error(response)
        raise RuntimeError("artifact API did not supply the required302")
    try:
        parsed = urllib.parse.urlsplit(location)
        hostname, port = parsed.hostname, parsed.port
        authority = "[" + str(ipaddress.IPv6Address(hostname)) + "]" if hostname and ":" in hostname else hostname
    except ValueError:
        raise RuntimeError("malformed artifact storage URL") from None
    require(parsed.scheme == "https" and hostname and hostname.isascii() and
            re.fullmatch(r"[A-Za-z0-9._:-]+", hostname) and
            not parsed.username and not parsed.password and "@" not in parsed.netloc and
            port in (None, 443) and parsed.netloc.lower() in (authority, authority + ":443") and
            not parsed.fragment, "invalid artifact storage HTTPS authority")
    receipt["redirect_hostname"] = hostname
    unexpired(pin)
    require(time.monotonic() < deadline, "artifact download deadline")
    # A new opener/request has no API Authorization or cookies. Never retain signed URL/headers.
    request = urllib.request.Request(location, headers={"Accept-Encoding": "identity"})
    try:
        response = opener().open(request, timeout=min(30, deadline - time.monotonic()))
        with response, target.open("xb") as sink:
            receipt["storage_status"] = response.status
            require(response.status == 200 and response.geturl() == location, "artifact storage redirect/status refused")
            observed, _ = read_bounded(response, pin["archive_bytes"], deadline, sink, receipt)
    except urllib.error.HTTPError as error:
        close_error(error)
        raise RuntimeError("artifact storage HTTP failure; no redirects or retry") from None
    except (OSError, urllib.error.URLError, http.client.HTTPException, ValueError):
        raise RuntimeError("artifact storage transport/write failure; no retry") from None
    receipt.update(observed)
    require(observed == {"bytes": pin["archive_bytes"], "sha256": pin["archive_sha256"]}, "artifact ZIP identity mismatch")
    unexpired(pin)
    return observed


def file_identity(file, maximum, deadline):
    info = file.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_size <= maximum, "candidate regular-file/size bound")
    digest, count = hashlib.sha256(), 0
    with file.open("rb") as source:
        while True:
            require(time.monotonic() < deadline, "candidate hash deadline")
            raw = source.read(min(65536, maximum + 1 - count))
            if not raw:
                break
            count += len(raw)
            require(count <= maximum, "candidate hash byte bound")
            digest.update(raw)
    require(count == info.st_size, "candidate changed while reading")
    return {"bytes": count, "sha256": digest.hexdigest()}


def extract_candidate(archive, target, pin, deadline):
    require(file_identity(archive, pin["archive_bytes"], deadline) ==
            {"bytes": pin["archive_bytes"], "sha256": pin["archive_sha256"]}, "unverified artifact ZIP")
    with zipfile.ZipFile(archive) as source:
        members = source.infolist()
        require(len(members) == pin["files"] and len({m.filename for m in members}) == len(members),
                "artifact file count/duplicate mismatch")
        require(sum(m.file_size for m in members) == pin["uncompressed_bytes"], "artifact expanded size mismatch")
        for member in members:
            require(re.fullmatch(r"[A-Za-z0-9_.-]+", member.filename) and member.filename not in (".", "..") and
                    not member.is_dir() and not member.flag_bits & 1 and
                    (member.external_attr >> 16) & 0xF000 in (0, stat.S_IFREG) and
                    member.compress_type in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED),
                    "artifact member type/path/encoding mismatch")
            maximum = pin["binary_bytes"] if member.filename == "ipsw" else 4 * 1024 * 1024
            require(member.file_size <= maximum, "artifact member oversize")
        by_name = {m.filename: m for m in members}
        require("sha256.json" in by_name and "ipsw" in by_name, "missing artifact manifest/candidate")
        manifest = strict_json(source.read(by_name["sha256.json"]))
        require(isinstance(manifest, dict) and set(manifest) == set(by_name) - {"sha256.json"} and
                len(manifest) == pin["manifest_entries"] and all(
                    isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) for value in manifest.values()),
                "artifact manifest schema mismatch")
        require(by_name["ipsw"].file_size == pin["binary_bytes"] and manifest["ipsw"] == pin["binary_sha256"],
                "artifact candidate manifest mismatch")
        # Verify every member/CRC and all hashes without extracting paths from the archive.
        for name, expected in manifest.items():
            with source.open(by_name[name]) as member:
                digest, count = hashlib.sha256(), 0
                while True:
                    require(time.monotonic() < deadline, "artifact verification deadline")
                    raw = member.read(min(65536, by_name[name].file_size + 1 - count))
                    if not raw:
                        break
                    count += len(raw)
                    require(count <= by_name[name].file_size, "artifact expanded member overflow")
                    digest.update(raw)
                require(count == by_name[name].file_size and digest.hexdigest() == expected, "artifact member hash mismatch")
        unexpired(pin)
        with source.open(by_name["ipsw"]) as member, target.open("xb") as sink:
            remaining = pin["binary_bytes"]
            while remaining:
                require(time.monotonic() < deadline, "candidate copy deadline")
                raw = member.read(min(65536, remaining))
                require(raw, "truncated candidate")
                sink.write(raw)
                remaining -= len(raw)
            require(not member.read(1), "candidate copy overflow")
    expected = {"bytes": pin["binary_bytes"], "sha256": pin["binary_sha256"]}
    require(file_identity(target, pin["binary_bytes"], deadline) == expected, "extracted candidate mismatch")
    target.chmod(0o700)
    return expected


def acquire(work, pin, token, deadline, record=None):
    record = {} if record is None else record
    record.update(artifact_id=pin["artifact_id"], verified=False, phase="retention-and-metadata")
    unexpired(pin)
    metadata = fetch_json(f"actions/artifacts/{pin['artifact_id']}", token, deadline)
    run = fetch_json(f"actions/runs/{pin['run_id']}/attempts/1", token, deadline)
    job = fetch_json(f"actions/jobs/{pin['job_id']}", token, deadline)
    record.update(validate_metadata(metadata, run, job, pin))
    archive, target = work / "ipsw-artifact.zip", work / "ipsw"
    require(not archive.exists() and not target.exists() and not target.is_symlink(), "substituted artifact/candidate path")
    record["phase"] = "exact-artifact-download"
    record["transport"] = {}
    record["archive"] = download(pin, archive, token, deadline, record["transport"])
    record["phase"] = "ZIP-manifest-candidate-verification"
    record["candidate"] = extract_candidate(archive, target, pin, deadline)
    record["verification"] = "Exact artifact API identity, ZIP, full manifest and post-sign candidate before any use."
    archive.unlink()
    record.update(verified=True, phase="verified-before-use")
    return target, record


def before_use(work, pin, record, deadline):
    unexpired(pin)
    require(record is not None and record.get("verified") is True and record.get("artifact_id") == pin["artifact_id"] and
            record.get("source_sha") == pin["source_sha"] and record.get("candidate") ==
            {"bytes": pin["binary_bytes"], "sha256": pin["binary_sha256"]}, "candidate not acquired from exact artifact")
    require(file_identity(work / "ipsw", pin["binary_bytes"], deadline) == record["candidate"],
            "derived candidate changed before invocation")
