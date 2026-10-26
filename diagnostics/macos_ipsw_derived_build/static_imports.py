"""Conservative source-only import closure; never invokes Go or imports upstream code."""

import hashlib
import re
import stat
import time


MODULE = "github.com/blacktop/ipsw"
GOOS = frozenset("aix android darwin dragonfly freebsd hurd illumos ios js linux nacl netbsd openbsd plan9 solaris wasip1 windows zos".split())
GOARCH = frozenset("386 amd64 amd64p32 arm armbe arm64 arm64be loong64 mips mipsle mips64 mips64le mips64p32 mips64p32le ppc ppc64 ppc64le riscv riscv64 s390 s390x sparc sparc64 wasm".split())
# Every name below is present in the source directories of the pinned dd3908cf Go archive.
STANDARD = frozenset("""
archive/tar archive/zip arena bufio bytes cmp compress/bzip2 compress/flate compress/gzip
compress/lzw compress/zlib container/heap container/list container/ring context
crypto crypto/aes crypto/boring crypto/cipher crypto/des crypto/dsa crypto/ecdh crypto/ecdsa
crypto/ed25519 crypto/elliptic crypto/fips140 crypto/hkdf crypto/hmac crypto/hpke crypto/md5 crypto/mldsa crypto/mlkem crypto/mlkem/mlkemtest
crypto/pbkdf2 crypto/rand crypto/rc4 crypto/rsa crypto/sha1 crypto/sha256 crypto/sha3
crypto/sha512 crypto/subtle crypto/tls crypto/tls/fipsonly crypto/x509 crypto/x509/pkix database/sql
database/sql/driver debug/buildinfo debug/dwarf debug/elf debug/gosym debug/macho
debug/pe debug/plan9obj embed encoding encoding/ascii85 encoding/asn1
encoding/base32 encoding/base64 encoding/binary encoding/csv encoding/gob
encoding/hex encoding/json encoding/json/jsontext encoding/json/v2 encoding/pem encoding/xml errors expvar flag fmt
go/ast go/build go/build/constraint go/constant go/doc go/doc/comment go/format
go/importer go/parser go/printer go/scanner go/token go/types go/version hash
hash/adler32 hash/crc32 hash/crc64 hash/fnv hash/maphash html html/template image
image/color image/color/palette image/draw image/gif image/jpeg image/png index/suffixarray
io io/fs io/ioutil iter log log/slog log/syslog maps math math/big math/bits
math/cmplx math/rand math/rand/v2 mime mime/multipart mime/quotedprintable net
net/http net/http/cgi net/http/cookiejar net/http/fcgi net/http/httptest
net/http/httptrace net/http/httputil net/http/pprof net/mail net/netip net/rpc
net/rpc/jsonrpc net/smtp net/textproto net/url os os/exec os/signal os/user path
path/filepath plugin reflect regexp regexp/syntax runtime runtime/asan runtime/cgo runtime/coverage runtime/debug
runtime/metrics runtime/msan runtime/pprof runtime/race runtime/secret runtime/trace simd simd/archsimd slices sort strconv
strings structs sync sync/atomic syscall syscall/js testing testing/cryptotest testing/fstest testing/iotest
testing/quick testing/slogtest testing/synctest text/scanner text/tabwriter
text/template text/template/parse time time/tzdata unicode unicode/utf16 unicode/utf8 unique unsafe uuid weak
""".split())
RESIDUALS = [
    "Only main-module import syntax/constraints and external module-prefix membership are checked; not compilation, import-name binding, type checking or linking.",
    "External module source/transitive imports and selected MVS versions are not evaluated; all candidate matching plan versions are reported.",
    "The explicit standard-library name allowlist classifies imports; installed Go package availability/implementation is not attested.",
    "Cgo preambles/C libraries, assembly, embed resources, generated code and linkname effects are not analyzed.",
    "Fixed darwin/arm64, cgo, gc, go1.1 through go1.27 and arm64.v8.0 tags only; experiment/tool tag uncertainty fails closed.",
]


class Error(ValueError):
    pass


def require(ok, message):
    if not ok:
        raise Error(message)


def tag_value(name, custom):
    require(not name.startswith("goexperiment.") and name != "boringcrypto" and not name.startswith("go1.27."),
            "unsupported experiment/tool tag: " + name)
    if name.startswith("go1."):
        require(re.fullmatch(r"go1\.(0|[1-9][0-9]*)", name), "unsupported release tag: " + name)
        return 1 <= int(name[4:]) <= 27
    return name in {"darwin", "arm64", "cgo", "gc", "unix", "arm64.v8.0"} or name in custom


def expression(text, custom):
    tokens = re.findall(r"&&|\|\||[!()]|[A-Za-z0-9_.]+|\S", text)
    require(tokens, "empty build constraint")
    values, operators = [], []

    def apply():
        op = operators.pop()
        require(len(values) >= (1 if op == "!" else 2), "invalid build constraint")
        right = values.pop()
        if op == "!":
            values.append(not right)
        else:
            left = values.pop()
            values.append(left and right if op == "&&" else left or right)

    operand = True
    precedence = {"!": 3, "&&": 2, "||": 1}
    for token in tokens:
        if operand:
            if token in ("!", "("):
                operators.append(token)
            else:
                require(re.fullmatch(r"[A-Za-z0-9_.]+", token), "unsupported build constraint token: " + token)
                values.append(tag_value(token, custom))
                operand = False
        elif token == ")":
            while operators and operators[-1] != "(":
                apply()
            require(operators and operators.pop() == "(", "unmatched build constraint parenthesis")
        else:
            require(token in ("&&", "||"), "missing build constraint operator")
            while operators and operators[-1] != "(" and precedence[operators[-1]] >= precedence[token]:
                apply()
            operators.append(token)
            operand = True
    require(not operand, "incomplete build constraint")
    while operators:
        require(operators[-1] != "(", "unmatched build constraint parenthesis")
        apply()
    require(len(values) == 1, "invalid build constraint")
    return values[0]


class Prefix:
    """Tokenize only Go's package/import prefix, including comment and string boundaries."""

    def __init__(self, text):
        self.text, self.offset, self.comments = text, 0, []

    def token(self):
        text = self.text
        while self.offset < len(text):
            start = self.offset
            ch = text[start]
            if ch in " \t\r":
                self.offset += 1
                continue
            if text.startswith("//", start):
                end = text.find("\n", start)
                self.offset = len(text) if end < 0 else end
                self.comments.append((start, text[start:self.offset]))
                continue
            if text.startswith("/*", start):
                end = text.find("*/", start + 2)
                require(end >= 0, "unterminated Go block comment")
                comment = text[start:end + 2]
                self.offset = end + 2
                if "\n" in comment:
                    return "\n"
                continue
            if ch in "\n;().":
                self.offset += 1
                return ch
            if ch in ('"', "`"):
                end = text.find(ch, start + 1)
                require(end >= 0, "unterminated import string")
                value = text[start + 1:end]
                require("\\" not in value and "\r" not in value and "\n" not in value,
                        "unsupported escape/newline in import string")
                self.offset = end + 1
                return ("string", value)
            word = re.match(r"[A-Za-z_][A-Za-z0-9_]*", text[start:])
            require(word is not None, "unsupported Go prefix token")
            self.offset += len(word[0])
            return word[0]
        return None

    def significant(self):
        token = self.token()
        while token in ("\n", ";"):
            token = self.token()
        return token


def source_prefix(raw, custom):
    text = raw.decode("utf-8-sig")
    require("\0" not in text, "NUL in Go source")
    parser = Prefix(text)
    require(parser.significant() == "package", "missing Go package clause")
    package_start = parser.offset - len("package")
    constraints = [comment[len("//go:build"):].strip() for start, comment in parser.comments
                   if start < package_start and re.match(r"//go:build(?:\s|$)", comment)]
    legacy = [re.sub(r"^//\s*\+build", "", comment).strip() for start, comment in parser.comments
              if start < package_start and re.match(r"//\s*\+build(?:\s|$)", comment)]
    for start, comment in parser.comments:
        if start < package_start and re.match(r"//(?:go:build|\s*\+build)(?:\s|$)", comment):
            require(re.search(r"\n[ \t\r]*\n", text[start + len(comment):package_start]),
                    "build constraint lacks separating blank line")
    require(len(constraints) <= 1, "multiple go:build lines")
    if constraints:
        enabled = expression(constraints[0], custom)
    else:
        enabled = True
        for line in legacy:
            require(line, "empty legacy build constraint")
            terms = []
            for group in line.split():
                parts = group.split(",")
                require(all(re.fullmatch(r"!?[A-Za-z0-9_.]+", part) for part in parts),
                        "unsupported legacy build constraint")
                terms.append(all([not tag_value(part[1:], custom) if part.startswith("!") else
                                  tag_value(part, custom) for part in parts]))
            enabled = enabled and any(terms)
    name = parser.token()
    require(isinstance(name, str) and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name),
            "unsupported package identifier")
    require(parser.token() in ("\n", ";", None), "missing package clause separator")
    if not enabled:
        return {"package": name, "enabled": False, "constraint": constraints or legacy, "imports": []}
    imports = []

    def spec(token):
        if isinstance(token, str):
            require(token == "." or re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", token), "invalid import alias")
            token = parser.token()
        require(isinstance(token, tuple) and token[0] == "string", "unsupported import specification")
        path = token[1]
        require(re.fullmatch(r"[A-Za-z0-9._~+/-]+", path) and
                all(part not in ("", ".", "..") for part in path.split("/")), "unsafe import path: " + path)
        if path not in imports:
            imports.append(path)

    token = parser.significant()
    while token == "import":
        token = parser.token()
        if token == "(":
            token = parser.significant()
            while token != ")":
                require(token is not None, "unterminated import block")
                spec(token)
                token = parser.token()
                require(token in ("\n", ";", ")"), "missing import separator")
                if token != ")":
                    token = parser.significant()
        else:
            spec(token)
        require(parser.token() in ("\n", ";", None), "missing import declaration separator")
        token = parser.significant()
    require(token is None or token in ("const", "var", "type", "func"), "unsupported declaration after imports")
    require(not any(start >= package_start and
                    re.match(r"//(?:go:build|\s*\+build)(?:\s|$)", comment)
                    for start, comment in parser.comments), "misplaced build constraint")
    return {"package": name, "enabled": True, "constraint": constraints or legacy, "imports": imports}


def filename_enabled(path):
    name = path.rsplit("/", 1)[-1]
    if not name.endswith(".go") or name.startswith((".", "_")) or name.endswith("_test.go"):
        return False
    # Only suffixes after an underscore are implicit GOOS/GOARCH constraints.
    parts = name[:-3].split("_")[1:]
    if parts and parts[-1] in GOARCH:
        if parts.pop() != "arm64":
            return False
    if parts and parts[-1] in GOOS:
        return parts[-1] == "darwin"
    return True


def module_requirements(text):
    requirements, group, module, language = {}, None, None, None
    for raw in text.splitlines():
        line = raw.split("//", 1)[0].strip()
        if not line:
            continue
        if line == ")":
            require(group is not None, "unmatched go.mod group")
            group = None
            continue
        fields = line.split()
        directive, values = (group, fields) if group else (fields[0], fields[1:])
        require(directive in ("module", "go", "tool", "require"), "unsupported go.mod directive: " + directive)
        if values == ["("]:
            require(group is None and directive in ("require", "tool"), "unsupported go.mod group")
            group = directive
        elif directive == "require":
            require(len(values) == 2 and re.fullmatch(r"v[0-9A-Za-z.+-]+", values[1]) and
                    values[0] not in requirements, "unsupported/duplicate go.mod requirement")
            requirements[values[0]] = values[1]
        elif directive == "module":
            require(module is None and values == [MODULE], "main-module declaration mismatch")
            module = values[0]
        elif directive == "go":
            require(language is None and values == ["1.27"], "unsupported go.mod language")
            language = values[0]
        else:
            require(len(values) == 1 and re.fullmatch(r"[A-Za-z0-9._~+/-]+", values[0]),
                    "unsupported tool declaration")
    require(group is None and module == MODULE and language == "1.27", "incomplete go.mod")
    return requirements


def analyze(manifest, requests, read_source, maximum, deadline, tags=()):
    report = {"status": "rejected", "target": "darwin/arm64", "cgo": True, "tags": list(tags),
              "go_release": "go1.27", "packages": [], "files": [], "edges": [],
              "standard_imports": [], "external_imports": [], "residuals": RESIDUALS, "errors": []}
    try:
        require(tags in ((), ("sandbox",)), "only no-tags or historical sandbox-negative configuration supported")
        rows, directories = {}, {}
        for row in manifest["files"]:
            path = row["path"]
            require(path not in rows and "\\" not in path and
                    all(part not in ("", ".", "..") for part in path.split("/")), "invalid/duplicate frozen path")
            require(row["mode"] in ("100644", "100755") and re.fullmatch(r"[0-9a-f]{40}", row["sha"]) and
                    isinstance(row["bytes"], int) and row["bytes"] >= 0, "invalid frozen file identity")
            rows[path] = row
            directory = path.rsplit("/", 1)[0] if "/" in path else ""
            directories.setdefault(directory, []).append(row)
        require(not any(path.endswith("/go.mod") or path.startswith("vendor/") for path in rows),
                "nested modules/vendor unsupported")
        used_bytes = 0

        def load(row):
            nonlocal used_bytes
            require(time.monotonic() < deadline and used_bytes + row["bytes"] <= maximum,
                    "static import source deadline/byte bound")
            raw = read_source(row["path"], row["bytes"])
            require(len(raw) == row["bytes"] and hashlib.sha1(
                b"blob " + str(len(raw)).encode() + b"\0" + raw).hexdigest() == row["sha"],
                "frozen source blob mismatch: " + row["path"])
            used_bytes += len(raw)
            return raw

        require("go.mod" in rows, "missing frozen go.mod")
        module_text = load(rows["go.mod"]).decode("utf-8")
        required = module_requirements(module_text)
        modules = {}
        for request in requests:
            module = request["module"]
            require(re.fullmatch(r"[A-Za-z0-9._~+/-]+", module) and
                    all(part not in ("", ".", "..") for part in module.split("/")), "invalid planned module")
            modules.setdefault(module, set()).add(request["version"])
        pending, visited = ["cmd/ipsw"], set()
        while pending:
            directory = pending.pop(0)
            if directory in visited:
                continue
            require(all(not part.startswith((".", "_")) and part not in ("testdata", "vendor")
                        for part in directory.split("/")), "unsupported reachable package directory: " + directory)
            require(directory in directories, "missing main-module package: " + MODULE + "/" + directory)
            visited.add(directory)
            names, imports = set(), set()
            for row in sorted(directories[directory], key=lambda row: row["path"]):
                if not row["path"].endswith(".go"):
                    continue
                if not filename_enabled(row["path"]):
                    report["files"].append({"path": row["path"], "selected": False, "reason": "filename/test"})
                    continue
                try:
                    info = source_prefix(load(row), tags)
                except (Error, UnicodeError) as error:
                    raise Error(row["path"] + ": " + str(error)) from error
                report["files"].append({"path": row["path"], "blob": row["sha"],
                                        "selected": info["enabled"], "constraint": info["constraint"]})
                if info["enabled"]:
                    names.add(info["package"])
                    imports.update(info["imports"])
            require(len(names) == 1, "empty or mixed active package: " + directory)
            require(("main" in names) == (directory == "cmd/ipsw"), "unexpected main package: " + directory)
            report["packages"].append(directory)
            for imported in sorted(imports):
                report["edges"].append({"from": directory, "import": imported})
                if imported == MODULE or imported.startswith(MODULE + "/"):
                    child = imported[len(MODULE):].lstrip("/")
                    require(child, "main-module root package unsupported")
                    pending.append(child)
                elif imported == "C" or imported in STANDARD:
                    report["standard_imports"].append(imported)
                else:
                    matches = [module for module in modules if imported == module or imported.startswith(module + "/")]
                    require(matches, "external import absent from frozen module plan: " + imported)
                    module = max(matches, key=len)
                    require(module in required and required[module] in modules[module],
                            "external module/version absent from frozen go.mod or plan: " + module)
                    report["external_imports"].append({"import": imported, "module": module,
                                                       "required_version": required[module],
                                                       "planned_versions": sorted(modules[module])})
        report["standard_imports"] = sorted(set(report["standard_imports"]))
        dependencies = {directory: set() for directory in report["packages"]}
        for edge in report["edges"]:
            if edge["import"].startswith(MODULE + "/"):
                dependencies[edge["from"]].add(edge["import"][len(MODULE) + 1:])
        while dependencies:
            require(time.monotonic() < deadline, "static import graph deadline")
            leaves = {directory for directory, children in dependencies.items() if not children}
            require(leaves, "main-module import cycle")
            dependencies = {directory: children - leaves for directory, children in dependencies.items()
                            if directory not in leaves}
        report["source_bytes_read"] = used_bytes
        report["status"] = "static-closure-only"
    except (Error, UnicodeError) as error:
        report["errors"].append({"type": type(error).__name__, "message": str(error)})
    return report


def check_tree(root, manifest, requests, maximum, deadline, tags=()):
    def read_source(relative, expected):
        file = root.joinpath(*relative.split("/"))
        require(stat.S_ISREG(file.lstat().st_mode), "nonregular source file: " + relative)
        with file.open("rb") as source:
            return source.read(expected + 1)
    return analyze(manifest, requests, read_source, maximum, deadline, tags)
