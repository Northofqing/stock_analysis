#!/usr/bin/env python3
"""Closed recording owner. No issuance, accepted include, runtime token or promotion.

Only `record` is public. The fixed adjacent manifest is reviewed source, never
caller input. Cargo and approved generators/filesystem ownership are trusted.
Recordings are diagnostic evidence requiring subsequent independent policy review.
"""
from __future__ import annotations

import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import threading
import uuid

ROOT = Path(__file__).resolve().parent.parent
POLICY = ROOT / "tools/replay_build_pin_v1_manifest.json"
SESSIONS = ROOT / ".replay-build-records"
SCHEMA = "stock-analysis-replay-build-record-v1"
PROFILE = "normaldev-library"
TARGET = "x86_64-apple-darwin"
HEX = re.compile(r"[0-9a-f]{64}\Z")
ID = re.compile(r"[0-9a-f]{32}\Z")
ENV_KEYS = {"PATH", "PROTOC", "PROTOC_INCLUDE", "CC", "CXX", "AR", "SDKROOT",
            "MACOSX_DEPLOYMENT_TARGET", "DEVELOPER_DIR", "SOURCE_DATE_EPOCH"}
FORBIDDEN_ENV = {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_BOOTSTRAP",
                 "RUSTC_WORKSPACE_WRAPPER", "DYLD_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES",
                 "LD_PRELOAD", "LD_LIBRARY_PATH"}


class Refusal(Exception):
    pass


def require(ok, reason):
    if not ok:
        raise Refusal(reason)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_hash(path):
    with open(path, "rb") as stream:
        result = hashlib.sha256()
        for block in iter(lambda: stream.read(65536), b""):
            result.update(block)
    return result.hexdigest()


def strict_json(raw):
    def object_pairs(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "DuplicateJsonKey")
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=object_pairs)


def keys(value, expected):
    require(isinstance(value, dict) and set(value) == set(expected), "PolicyFields")


def atomic_json(path, value):
    temp = path.with_name(path.name + ".pending-" + uuid.uuid4().hex)
    with open(temp, "xb") as stream:
        stream.write((json.dumps(value, sort_keys=True, ensure_ascii=True, indent=2) + "\n").encode())
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temp, path)


def regular(path):
    require(not path.is_symlink() and stat.S_ISREG(path.stat().st_mode), "NotRegularFile")


def relative_name(name):
    require(isinstance(name, str) and name and "\\" not in name, "InventoryPath")
    path = Path(name)
    require(not path.is_absolute() and all(p not in {"", ".", ".."} for p in name.split("/")), "InventoryPath")
    return path


def inside(path, root):
    try:
        path.resolve().relative_to(root.resolve())
        return True
    except ValueError:
        return False


def inventory(spec, copy_to=None):
    """Exact ordinary-file inventory within reviewed roots; no symlink following."""
    keys(spec, {"root", "roots", "files"})
    root = Path(spec["root"])
    require(root.is_absolute() and root.resolve() == root and root.is_dir(), "InventoryRoot")
    require(isinstance(spec["roots"], list) and spec["roots"], "InventoryRoots")
    require(isinstance(spec["files"], dict) and spec["files"], "InventoryFiles")
    members = {}
    for name in spec["roots"]:
        base = root / relative_name(name)
        require(inside(base, root) and not base.is_symlink(), "InventoryEscape")
        require(base.exists(), "MissingInventoryMember")
        candidates = [base] if base.is_file() else base.rglob("*")
        for member in candidates:
            require(not member.is_symlink(), "InventorySymlink")
            if member.is_dir():
                continue
            regular(member)
            name = member.relative_to(root).as_posix()
            require(name not in members, "OverlappingInventoryRoots")
            members[name] = file_hash(member)
    require(set(members) == set(spec["files"]), "InventoryMemberSet")
    for name, expected in spec["files"].items():
        relative_name(name)
        require(isinstance(expected, str) and HEX.fullmatch(expected), "InventoryDigest")
        require(members[name] == expected, "InventoryMismatch")
        if copy_to is not None:
            dest = copy_to / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            with open(root / name, "rb") as source, open(dest, "xb") as target:
                shutil.copyfileobj(source, target)
            require(file_hash(dest) == expected, "CopyMismatch")
            dest.chmod(0o444 | ((root / name).stat().st_mode & 0o111))
    return members


def pinned_file(spec):
    keys(spec, {"path", "sha256"})
    path = Path(spec["path"])
    require(path.is_absolute() and path.resolve() == path, "ToolPath")
    regular(path)
    require(isinstance(spec["sha256"], str) and HEX.fullmatch(spec["sha256"]), "ToolDigest")
    require(file_hash(path) == spec["sha256"], "ToolMismatch")
    return str(path)


def load_policy():
    regular(POLICY)
    raw = POLICY.read_bytes()
    policy = strict_json(raw)
    keys(policy, {"schema", "mode", "profile", "inventory"})
    require(policy["schema"] == SCHEMA and policy["mode"] == "RecordingOnly"
            and policy["profile"] in {PROFILE, BUNDLED_PROFILE}, "PolicyIdentity")
    if policy["inventory"] is None:
        return policy, digest(raw)
    inv = policy["inventory"]
    keys(inv, {"owner_sha256", "cargo", "rustc", "python", "sysroot", "application",
               "vendor", "packages", "generators", "environment", "ancestor_configs"})
    require(inv["owner_sha256"] == file_hash(Path(__file__)), "OwnerMismatch")
    for name in ("cargo", "rustc", "python"):
        pinned_file(inv[name])
    require(str(Path(sys.executable).resolve()) == inv["python"]["path"], "PythonMismatch")
    require(isinstance(inv["generators"], dict) and "PROTOC" in inv["generators"], "GeneratorInventory")
    for generator in inv["generators"].values():
        pinned_file(generator)
    require(isinstance(inv["ancestor_configs"], list), "AncestorConfigPolicy")
    for config in inv["ancestor_configs"]:
        pinned_file(config)
    env = inv["environment"]
    require(isinstance(env, dict) and set(env) <= ENV_KEYS and "PATH" in env, "EnvironmentPolicy")
    require(all(isinstance(v, str) and "\x00" not in v for v in env.values()), "EnvironmentValue")
    for name, generator in inv["generators"].items():
        require(name in ENV_KEYS and env.get(name) == generator["path"], "GeneratorEnvironment")
    require(isinstance(inv["packages"], list) and inv["packages"], "PackageInventory")
    seen = set()
    for package in inv["packages"]:
        keys(package, {"id", "tree", "manifest"})
        require(isinstance(package["id"], str) and package["id"] and package["id"] not in seen, "PackageIdentity")
        remainder = package["id"].replace("{application_uri}", "")
        require("{" not in remainder and "}" not in remainder, "PackageIdTemplate")
        require("{application_uri}" not in package["id"] or package["tree"] == "application", "PackageIdTemplate")
        require(package["tree"] in {"application", "vendor"}, "PackageTree")
        require(relative_name(package["manifest"]).name == "Cargo.toml", "PackageManifest")
        require(package["manifest"] in inv[package["tree"]]["files"], "UninventoriedManifest")
        seen.add(package["id"])
    require("Cargo.toml" in inv["application"]["files"] and "Cargo.lock" in inv["application"]["files"], "ApplicationManifest")
    if policy["profile"] == BUNDLED_PROFILE:
        require({"CC", "AR"} <= set(inv["generators"]), "NativeToolInventory")
        require(sum(p["id"] == SQLITE_PACKAGE for p in inv["packages"]) == 1
                and sum(p["id"] == CC_PACKAGE for p in inv["packages"]) == 1, "NativePackageInventory")
    return policy, digest(raw)


def validate_config_chain(path, approved):
    observed = set()
    expected = {pinned_file(spec) for spec in approved}
    require(len(expected) == len(approved), "DuplicateAncestorConfig")
    for parent in (path, *path.parents):
        for name in (".cargo/config", ".cargo/config.toml"):
            candidate = parent / name
            if candidate.exists() or candidate.is_symlink():
                regular(candidate)
                require(candidate.resolve() == candidate, "AncestorConfigAlias")
                observed.add(str(candidate))
    require(observed == expected, "UnreviewedAncestorCargoConfig")


def package_id(package, session):
    # Only this session-path substitution exists; package/version/source suffix
    # remains reviewed exact text. Raw Cargo package IDs remain in the log.
    return package["id"].replace("{application_uri}", (session / "application").as_uri())


def run_streamed(argv, cwd, env, stdout_file, stderr_file, echo=False, *, pass_fds=(),
                 stdin_bytes=None, stdin_failures=None):
    """Preserve exact child bytes while forwarding both streams without buffering a tree."""
    process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               close_fds=True, pass_fds=pass_fds,
                               stdin=subprocess.PIPE if stdin_bytes is not None else None)
    failures = []
    def pump(source, path, fd):
        try:
            with open(path, "xb") as output:
                for block in iter(lambda: source.read(65536), b""):
                    output.write(block)
                    if echo:
                        view = memoryview(block)
                        while view:
                            written = os.write(fd, view)
                            view = view[written:]
        except BaseException as error:
            failures.append(repr(error))
        finally:
            source.close()
    threads = [threading.Thread(target=pump, args=(process.stdout, stdout_file, 1)),
               threading.Thread(target=pump, args=(process.stderr, stderr_file, 2))]
    for thread in threads:
        thread.start()
    if stdin_bytes is not None:
        # Closed source is at most 60 bytes; both output pumps are already draining.
        try:
            process.stdin.write(stdin_bytes)
            process.stdin.close()
        except OSError as error:
            stdin_failures.append("ChildStdinWrite:" + type(error).__name__)
            try:
                process.stdin.close()
            except OSError:
                pass
    code = process.wait()
    for thread in threads:
        thread.join()
    require(not failures, "ChildOutputCapture")
    return code


RUSTC_VALUE_FLAGS = {"--crate-name", "--edition", "--crate-type", "--emit", "--out-dir", "--target",
                     "--extern", "--cfg", "--check-cfg", "--cap-lints", "--error-format", "--json",
                     "--color", "--sysroot", "--print", "--remap-path-prefix", "--diagnostic-width"}


def parse_rustc(args):
    """Finite Cargo call forms, exact original argv retained separately. Not rustc grammar."""
    values = RUSTC_VALUE_FLAGS
    booleans = {"--test", "-g", "-O", "-vV", "-V", "--version"}
    options, inputs = {}, []
    i = 0
    while i < len(args):
        arg = args[i]
        require(not arg.startswith("@"), "ResponseFile")
        if arg in booleans:
            key, value = arg, True
        elif arg in {"-C", "-L", "-A", "-W", "-D", "-F"}:
            require(i + 1 < len(args), "MissingFlagValue")
            key, value = arg, args[i + 1]
            i += 1
        elif any(arg.startswith(prefix) and len(arg) > 2 for prefix in ("-C", "-L", "-A", "-W", "-D", "-F")):
            key, value = arg[:2], arg[2:]
        elif arg.startswith(("--allow=", "--warn=", "--deny=")):
            name, value = arg.split("=", 1)
            require(re.fullmatch(r"(?:clippy::)?[a-z_][a-z0-9_]*", value) or value in {
                "clippy::unnecessary-wraps", "clippy::or-fun-call",
                "clippy::branches-sharing-code", "clippy::alloc-instead-of-core"} or arg in {
                # Record6 indexmap: exact observed level/name pairs, not normalization.
                "--deny=unsafe-code", "--deny=unreachable-pub", "--deny=unnameable-types",
                "--deny=private-interfaces", "--deny=private-bounds", "--warn=rust-2018-idioms"},
                    "UnsupportedLintArgument")
            key = {"--allow": "-A", "--warn": "-W", "--deny": "-D"}[name]
        elif arg.split("=", 1)[0] in values:
            key = arg.split("=", 1)[0]
            if "=" in arg:
                value = arg.split("=", 1)[1]
            else:
                require(i + 1 < len(args), "MissingFlagValue")
                i += 1
                value = args[i]
        elif arg == "-" or (not arg.startswith("-") and arg.endswith(".rs")):
            inputs.append(arg)
            i += 1
            continue
        else:
            raise Refusal("UnsupportedRustcArgument:" + arg)
        options.setdefault(key, []).append(value)
        i += 1
    for key in {"--crate-name", "--edition", "--out-dir", "--target", "--sysroot", "--emit"}:
        require(len(options.get(key, [])) <= 1, "DuplicateRustcFlag:" + key)
    codegen = {}
    allowed_codegen = {"opt-level", "embed-bitcode", "debuginfo", "debug-assertions", "metadata",
                       "extra-filename", "incremental", "panic", "codegen-units", "target-cpu",
                       "target-feature", "overflow-checks", "strip", "linker", "link-arg",
                       "prefer-dynamic", "lto", "force-frame-pointers", "split-debuginfo"}
    for value in options.get("-C", []):
        key, separator, val = value.partition("=")
        require(key in allowed_codegen, "UnsupportedCodegen")
        require(key not in codegen or key == "link-arg", "DuplicateCodegen")
        codegen.setdefault(key, []).append(val if separator else "yes")
    prints = options.get("--print", [])
    probe = bool(prints or any(key in options for key in ("-vV", "-V", "--version")))
    if probe:
        require(set(prints) <= {"sysroot", "target-libdir", "cfg", "file-names", "split-debuginfo", "crate-name"}, "UnsupportedProbe")
        require(not inputs or inputs == ["-"], "ProbeSource")
    else:
        require(len(inputs) == 1 and inputs[0] != "-", "CompileSource")
        require(all(options.get(key) for key in ("--crate-name", "--out-dir", "--emit")), "IncompleteCompile")
    return {"options": options, "codegen": codegen, "inputs": inputs, "probe": probe}


def make_words(raw):
    """Encountered dep-info words only: escapes, whitespace and literal doubled $."""
    result, word, i = [], bytearray(), 0
    while i < len(raw):
        ch = raw[i]
        if ch == 92:
            i += 1
            require(i < len(raw) and raw[i] in b" \\#:\t", "UnsupportedDepEscape")
            word.append(raw[i])
        elif ch in b" \t\r":
            if word:
                result.append(bytes(word)); word.clear()
        elif ch == 36:
            require(i + 1 < len(raw) and raw[i + 1] == 36, "UnsupportedDepExpansion")
            word.append(36); i += 1
        else:
            word.append(ch)
        i += 1
    if word:
        result.append(bytes(word))
    return result


def dep_info(raw):
    raw = raw.replace(b"\\\r\n", b"").replace(b"\\\n", b"")
    paths, env = set(), []
    for line in raw.splitlines():
        if not line.strip():
            continue
        if line.startswith(b"# env-dep:"):
            env.append(line[len(b"# env-dep:"):].hex()); continue
        require(not line.startswith(b"#"), "UnsupportedDepComment")
        escaped = False
        colon = None
        for i, ch in enumerate(line):
            if ch == 58 and not escaped:
                colon = i; break
            escaped = ch == 92 and not escaped
        require(colon is not None, "UnsupportedDepRule")
        make_words(line[:colon])
        paths.update(os.fsdecode(word) for word in make_words(line[colon + 1:]))
    return {"paths": sorted(paths), "environment_comment_hex": env}


def selected_outputs(parsed, cwd, target):
    options, codegen = parsed["options"], parsed["codegen"]
    out = (cwd / options["--out-dir"][0]).resolve()
    require(inside(out, target), "OutputEscape")
    name = options["--crate-name"][0]
    require(re.fullmatch(r"[A-Za-z0-9_]+", name), "CrateName")
    suffix = codegen.get("extra-filename", [""])[0]
    require(re.fullmatch(r"[-A-Za-z0-9_]*", suffix), "OutputSuffix")
    base = name + suffix
    kinds = [part for value in options.get("--crate-type", ["bin"]) for part in value.split(",")]
    require(set(kinds) <= {"lib", "rlib", "bin", "proc-macro", "dylib", "cdylib"}, "CrateType")
    result = []
    for emit in options["--emit"][0].split(","):
        kind, separator, explicit = emit.partition("=")
        require(kind in {"dep-info", "metadata", "link"}, "UnsupportedEmit")
        if separator:
            paths = [(cwd / explicit).resolve()]
        elif kind == "dep-info":
            paths = [out / (base + ".d")]
        elif kind == "metadata":
            paths = [out / ("lib" + base + ".rmeta")]
        else:
            paths = []
            for crate_type in kinds:
                filename = base if crate_type == "bin" else "lib" + base + (
                    ".rlib" if crate_type in {"lib", "rlib"} else ".dylib")
                paths.append(out / filename)
        for path in paths:
            require(inside(path, target), "EmitEscape")
            result.append({"path": str(path), "kind": kind})
    require(len({item["path"] for item in result}) == len(result), "AmbiguousEmit")
    return result


def sysroot_loader_path(sysroot):
    """Only the complete inventoried lib tree supplies the owner loader path."""
    require("lib" in sysroot["roots"], "IncompleteLoaderInventory")
    path = Path(sysroot["root"]) / "lib"
    require(path.is_absolute() and path.resolve() == path and path.is_dir()
            and ":" not in str(path), "SysrootLoaderPath")
    return str(path)


RECORD10_KINDS = {"RustversionBuildVersion", "ThiserrorStaticFeatureProbe"}
RECORD10_PRIVATE = b"#[doc(hidden)]\npub mod __private18 {\n    #[doc(hidden)]\n    pub use crate::private::*;\n}\n"


def record10_candidate(args, env, cwd, session):
    # Recognize an origin independently of the tokens that its template requires.
    names = [n for n in ("rustversion", "thiserror")
             if env.get("CARGO_PKG_NAME") == n or cwd == session / "vendor" / n]
    nested = (env.get("DYLD_FALLBACK_LIBRARY_PATH", "").startswith(str(session / "target/debug") + ":")
              or "build/probe.rs" in args[1:]
              or "OUT_DIR" in env and any(a in {"--version", "--rustc"} for a in args[1:]))
    if not names or not nested:
        return None
    require(len(names) == 1, "Record10Origin")
    return names[0]


def record10_transient_namespace(args, env, cwd, session):
    # Negative-only quarantine: required features, source and template do not
    # grant these names, and their later rejection must not erase the names.
    if record10_candidate(args, env, cwd, session) != "thiserror":
        return set()
    raw = env.get("OUT_DIR", "")
    out = Path(raw)
    parent = session / "target" / TARGET / "debug/build"
    require(out.is_absolute() and str(out) == raw and out.resolve() == out
            and out.is_dir() and not out.is_symlink() and out.name == "out"
            and out.parent.parent == parent
            and re.fullmatch(r"thiserror-[0-9a-f]{16}", out.parent.name), "Record10OutDir")
    require((out / "probe").resolve() == out / "probe", "Record10ProbeDirectory")
    return {str(out / "probe" / name) for name in ("thiserror.d", "libthiserror.rmeta")}


def record10_source(name, env, cwd, session, inv):
    require(name in {"rustversion", "thiserror"}, "Record10Origin")
    if name == "rustversion":
        version, parts = "1.0.22", ("1", "0", "22")
        sources = ("Cargo.toml", "build/build.rs", "build/rustc.rs", "src/lib.rs")
        parent = session / "target/debug/build"
    else:
        version, parts = "2.0.18", ("2", "0", "18")
        sources = ("Cargo.toml", "build.rs", "build/probe.rs", "src/lib.rs")
        parent = session / "target" / TARGET / "debug/build"
    package = {"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
               "tree": "vendor", "manifest": name + "/Cargo.toml"}
    root = session / "vendor" / name
    require(inv["packages"].count(package) == 1 and cwd == root
            and env.get("CARGO_MANIFEST_DIR") == str(root)
            and env.get("CARGO_MANIFEST_PATH") == str(root / "Cargo.toml")
            and env.get("CARGO_PKG_NAME") == name and env.get("CARGO_PKG_VERSION") == version
            and all(env.get(k) == v for k, v in zip(("CARGO_PKG_VERSION_MAJOR", "CARGO_PKG_VERSION_MINOR",
                "CARGO_PKG_VERSION_PATCH"), parts)) and env.get("CARGO_PKG_VERSION_PRE") == "",
            "Record10Package")
    for source in sources:
        path = root / source
        regular(path)
        require(path.resolve() == path and path.stat().st_nlink == 1
                and inv["vendor"]["files"].get(name + "/" + source) == file_hash(path),
                "Record10Source")
    raw = env.get("OUT_DIR", "")
    out = Path(raw)
    require(out.is_absolute() and str(out) == raw and out.resolve() == out and out.is_dir()
            and not out.is_symlink() and out.name == "out" and out.parent.parent == parent
            and re.fullmatch(name + r"-[0-9a-f]{16}", out.parent.name), "Record10OutDir")
    return package, root, out


def record10_context(args, env, cwd, session, inv):
    name = record10_candidate(args, env, cwd, session)
    if name is None:
        return None
    package, root, out = record10_source(name, env, cwd, session, inv)
    features = set() if name == "rustversion" else {"CARGO_FEATURE_DEFAULT", "CARGO_FEATURE_STD"}
    require({k for k in env if k.startswith("CARGO_FEATURE_")} == features
            and all(env[k] == "1" for k in features), "Record10Features")
    require(args and args[0] == inv["rustc"]["path"] and env.get("RUSTC") == args[0]
            and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and "RUSTC_WORKSPACE_WRAPPER" not in env and "RUSTC_STAGE" not in env
            and "RUSTC_BOOTSTRAP" not in env and env.get("HOST") == TARGET
            and env.get("TARGET") == TARGET and env.get("CARGO_ENCODED_RUSTFLAGS") == "",
            "Record10Environment")
    if name == "rustversion":
        exact, kind, role = ["--version"], "RustversionBuildVersion", "Host"
    else:
        require((out / "probe").resolve() == out / "probe", "Record10ProbeDirectory")
        exact = ["--edition=2018", "--crate-name=thiserror", "--crate-type=lib", "--cap-lints=allow",
                 "--emit=dep-info,metadata", "--out-dir", str(out / "probe"), "build/probe.rs", "--target", TARGET]
        kind, role = "ThiserrorStaticFeatureProbe", "Target"
    require(args[1:] == exact, "Record10Template")
    return parse_rustc(args[1:]), {"kind": kind, "package_id": package["id"], "manifest": str(root),
                                  "out_dir": str(out), "source_role": role}


def record10_capture_outputs(call, outputs, code):
    # Preserve earlier retained files even if a later capture or dep-info fails.
    captured, blockers = [], []
    for index, item in enumerate(outputs):
        path = Path(item["path"])
        try:
            if not path.exists() and not path.is_symlink():
                if code == 0:
                    blockers.append("MissingDeclaredOutput:" + str(path))
                continue
            require(path.resolve() == path, "TransientOutputAlias")
            regular(path)
            before = path.stat()
            require(before.st_nlink == 1, "TransientOutputAlias")
            snapshot = call / ("probe-output-" + str(index) + ".raw")
            with open(path, "rb") as source, open(snapshot, "xb") as dest:
                opened = os.fstat(source.fileno())
                require((opened.st_dev, opened.st_ino, opened.st_size) == (before.st_dev, before.st_ino, before.st_size),
                        "TransientOutputChanged")
                shutil.copyfileobj(source, dest)
            output = dict(item, sha256=file_hash(snapshot), snapshot=snapshot.name)
            captured.append(output)
            after = path.stat()
            require(path.resolve() == path and after.st_nlink == 1
                    and (after.st_dev, after.st_ino, after.st_size) == (before.st_dev, before.st_ino, before.st_size)
                    and file_hash(path) == output["sha256"], "TransientOutputChanged")
            if item["kind"] == "dep-info":
                output["dep_info"] = dep_info(snapshot.read_bytes())
        except (Refusal, OSError) as error:
            blockers.append("TransientEvidence:" + str(error))
    return captured, blockers


def record10_evidence(receipt, call, session, inv):
    request = strict_json((call / "request.json").read_bytes())
    initial = strict_json((call / "invocation.json").read_bytes())
    args = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
    env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
    cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
    claimed = any(x.get("context", {}).get("kind") in RECORD10_KINDS for x in (initial, receipt))
    special = record10_context(args, env, cwd, session, inv)
    if special is None:
        require(not claimed, "Record10Invocation")
        return None, []
    parsed, context = special
    compiler_environment(env, session, inv["sysroot"], probe=parsed["probe"], context=context)
    outputs = [] if parsed["probe"] else selected_outputs(parsed, cwd, session / "target")
    require(all(initial[k] == receipt[k] for k in ("context", "argv_hex", "environment_hex", "parsed",
            "source", "package", "role", "kind", "declared_outputs", "cwd", "compiler_sha256"))
            and receipt["context"] == context and receipt["parsed"] == parsed
            and receipt["argv_hex"] == request["argv_hex"] and receipt["environment_hex"] == request["environment_hex"]
            and receipt["cwd"] == str(cwd) and receipt["declared_outputs"] == outputs
            and receipt["compiler_sha256"] == inv["rustc"]["sha256"], "Record10Invocation")
    if parsed["probe"]:
        require(receipt["kind"] == "Probe" and receipt["role"] == "Probe"
                and receipt["source"] is None and receipt["package"] is None and receipt["outputs"] == [],
                "Record10VersionEvidence")
    else:
        require(receipt["kind"] == "TransientProbe" and receipt["role"] == "Target"
                and receipt["source"] == str(cwd / "build/probe.rs")
                and receipt["package"] == {"id": context["package_id"], "tree": "vendor", "manifest": "thiserror/Cargo.toml"},
                "Record10ProbeEvidence")
        declarations = {o["path"]: (i, o) for i, o in enumerate(outputs)}
        require(len({o["path"] for o in receipt["outputs"]}) == len(receipt["outputs"]), "Record10ProbeEvidence")
        if receipt["exit_code"] == 0:
            require({o["path"] for o in receipt["outputs"]} == set(declarations), "Record10ProbeEvidence")
        for output in receipt["outputs"]:
            require(output["path"] in declarations, "Record10ProbeEvidence")
            index, declaration = declarations[output["path"]]
            require(output["kind"] == declaration["kind"] and output["snapshot"] == "probe-output-" + str(index) + ".raw",
                    "Record10ProbeEvidence")
            snapshot = call / output["snapshot"]
            regular(snapshot)
            require(snapshot.resolve() == snapshot and snapshot.stat().st_nlink == 1
                    and file_hash(snapshot) == output["sha256"], "Record10Snapshot")
            if output["kind"] == "dep-info":
                require(output.get("dep_info") == dep_info(snapshot.read_bytes()), "Record10DepInfo")
    return context, outputs


def record10_graph_blockers(receipts, associations, artifacts, events, session, inv):
    blockers, groups, raw_consumers = [], set(), {}
    by_id = {r["invocation_id"]: r for r in receipts}
    for r in receipts:
        try:
            request = strict_json((session / "invocations" / r["invocation_id"] / "request.json").read_bytes())
            raw = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
            cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
        except (Refusal, OSError, KeyError, TypeError, ValueError):
            blockers.append("Record10Graph:RequestEvidence")
            continue
        for name, version in (("rustversion", "1.0.22"), ("thiserror", "2.0.18")):
            root = session / "vendor" / name
            package = "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version
            consumer = any(not a.startswith("-") and a.endswith(".rs") and (cwd / a).resolve() == root / "src/lib.rs" for a in raw[1:])
            if consumer:
                raw_consumers.setdefault(name, []).append(r)
            child = r.get("context", {}).get("kind") in RECORD10_KINDS and r["context"].get("package_id") == package
            if consumer or child:
                groups.add((name, package))
    for name, version in (("rustversion", "1.0.22"), ("thiserror", "2.0.18")):
        package = "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version
        if any(e["reason"] == "compiler-artifact" and e.get("package_id") == package
               and e.get("target", {}).get("src_path") == str(session / "vendor" / name / "src/lib.rs") for e in events):
            groups.add((name, package))
    # One package has one fixed no-retry/no-bootstrap origin, even when no child was reached.
    for name, package in groups:
        try:
            root = session / "vendor" / name
            origins = [a for a in associations if a["package_id"] == package]
            origin_events = [e for e in events if e["reason"] == "build-script-executed" and e.get("package_id") == package]
            require(len(origins) == len(origin_events) == 1 and origins[0]["cargo_event"] == origin_events[0], "Record10OriginJoin")
            association = origins[0]
            builder = by_id[association["producer_invocation"]]
            build_source = str(root / ("build/build.rs" if name == "rustversion" else "build.rs"))
            expected_features = [] if name == "rustversion" else ["default", "std"]
            builder_call = session / "invocations" / builder["invocation_id"]
            builder_request = strict_json((builder_call / "request.json").read_bytes())
            builder_initial = strict_json((builder_call / "invocation.json").read_bytes())
            builder_raw = [os.fsdecode(bytes.fromhex(a)) for a in builder_request["argv_hex"]]
            builder_env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in builder_request["environment_hex"].items()}
            require(all(builder_initial[k] == builder[k] for k in ("context", "argv_hex", "environment_hex", "parsed", "source", "package", "role", "kind", "cwd"))
                    and builder_request["argv_hex"] == builder["argv_hex"]
                    and builder_request["environment_hex"] == builder["environment_hex"]
                    and builder["cwd"] == os.fsdecode(bytes.fromhex(builder_request["cwd_hex"])) == str(root)
                    and builder_raw[0] == inv["rustc"]["path"] and builder["parsed"] == parse_rustc(builder_raw[1:])
                    and len(builder["parsed"]["inputs"]) == 1
                    and str((Path(builder["cwd"]) / builder["parsed"]["inputs"][0]).resolve()) == build_source
                    and not any(k in builder_env for k in ("RUSTC_BOOTSTRAP", "RUSTC_STAGE", "RUSTC_WORKSPACE_WRAPPER"))
                    and not builder_env.get("CARGO_ENCODED_RUSTFLAGS"), "Record10BuilderEvidence")
            require(builder["kind"] == "Compile" and builder["role"] == "Host" and builder["exit_code"] == 0
                    and not builder["blockers"] and builder["source"] == build_source
                    and builder["package"] == {"id": package, "tree": "vendor", "manifest": name + "/Cargo.toml"}
                    and sorted(builder["parsed"]["options"].get("--cfg", [])) == sorted('feature="' + f + '"' for f in expected_features),
                    "Record10BuilderJoin")
            ba = [a for a in artifacts if a["invocation_id"] == builder["invocation_id"] and "builder_alias" in a]
            be = [e for e in events if e["reason"] == "compiler-artifact" and e.get("package_id") == package
                  and e.get("target", {}).get("kind") == ["custom-build"]]
            require(len(ba) == len(be) == 1 and be[0]["target"].get("src_path") == build_source
                    and sorted(be[0].get("features", [])) == expected_features, "Record10BuilderJoin")
            build_deps = {str((Path(builder["cwd"]) / p).resolve()) for o in builder["outputs"]
                          for p in o.get("dep_info", {}).get("paths", [])}
            required = {build_source} | ({str(root / "build/rustc.rs")} if name == "rustversion" else set())
            require(required <= build_deps, "Record10BuilderSources")
            children = [r for r in receipts if r.get("context", {}).get("kind") in RECORD10_KINDS
                        and r["context"].get("package_id") == package]
            kind = "RustversionBuildVersion" if name == "rustversion" else "ThiserrorStaticFeatureProbe"
            require(len(children) == 1 and children[0]["context"]["kind"] == kind
                    and children[0]["context"]["out_dir"] == association["out_dir"]
                    and not children[0]["blockers"] and children[0]["exit_code"] in ((0,) if name == "rustversion" else (0, 1)),
                    "Record10ChildJoin")
            consumers = raw_consumers.get(name, [])
            require(len(consumers) == 1, "Record10ConsumerJoin")
            consumer = consumers[0]
            call = session / "invocations" / consumer["invocation_id"]
            request = strict_json((call / "request.json").read_bytes())
            initial = strict_json((call / "invocation.json").read_bytes())
            raw = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
            env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
            raw_cwd = os.fsdecode(bytes.fromhex(request["cwd_hex"]))
            cwd = Path(raw_cwd)
            require(all(initial[k] == consumer[k] for k in ("context", "argv_hex", "environment_hex", "source", "package", "parsed", "role", "kind", "cwd"))
                    and request["argv_hex"] == consumer["argv_hex"] and request["environment_hex"] == consumer["environment_hex"]
                    and raw[0] == inv["rustc"]["path"] and consumer["parsed"] == parse_rustc(raw[1:])
                    and consumer["source"] == str(root / "src/lib.rs")
                    and consumer["cwd"] == raw_cwd and str(cwd) == raw_cwd, "Record10ConsumerJoin")
            fixed_package, _, out = record10_source(name, env, cwd, session, inv)
            # Ordinary Cargo rustc consumers need not have build-script feature
            # environment flags. Their raw cfg and artifact feature set below
            # are mandatory; a present environment set must still be exact.
            feature_keys = {k for k in env if k.startswith("CARGO_FEATURE_")}
            expected_keys = {"CARGO_FEATURE_" + f.upper() for f in expected_features}
            require(not feature_keys or feature_keys == expected_keys and all(env[k] == "1" for k in feature_keys),
                    "Record10ConsumerFeatures")
            role, crate_type = ("Host", "proc-macro") if name == "rustversion" else ("Target", "lib")
            require(str(out) == association["out_dir"] and consumer["package"] == fixed_package
                    and consumer["kind"] == "Compile" and consumer["role"] == role and consumer["exit_code"] == 0
                    and not consumer["blockers"] and consumer["context"]["kind"] == "DirectCargoCompile", "Record10ConsumerJoin")
            ca = [a for a in artifacts if a["invocation_id"] == consumer["invocation_id"]]
            ce = [e for e in events if e["reason"] == "compiler-artifact" and e.get("package_id") == package
                  and e.get("target", {}).get("src_path") == consumer["source"]]
            require(len(ca) == len(ce) == 1 and ce[0]["target"].get("kind") == [crate_type]
                    and ce[0]["target"].get("crate_types") == [crate_type] and ce[0]["target"].get("name") == name
                    and sorted(ce[0].get("features", [])) == expected_features, "Record10ConsumerJoin")
            event = association["cargo_event"]
            cfgs = ["error_generic_member_access"] if name == "thiserror" and children[0]["exit_code"] == 0 else []
            require(event["cfgs"] == cfgs and all(event[k] == [] for k in ("linked_libs", "linked_paths", "env"))
                    and sorted(consumer["parsed"]["options"].get("--cfg", [])) == sorted(
                        ['feature="' + f + '"' for f in expected_features] + cfgs), "Record10CfgJoin")
            filename = "version.expr" if name == "rustversion" else "private.rs"
            generated = out / filename
            regular(generated)
            require(generated.resolve() == generated and generated.stat().st_nlink == 1
                    and association["generated_files"].get(filename) == file_hash(generated)
                    and (name == "rustversion" or generated.read_bytes() == RECORD10_PRIVATE), "Record10GeneratedJoin")
            consumed = {str((Path(consumer["cwd"]) / p).resolve()) for o in consumer["outputs"]
                        for p in o.get("dep_info", {}).get("paths", [])}
            require({str(generated), consumer["source"]} <= consumed, "Record10GeneratedJoin")
        except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError) as error:
            blockers.append("Record10Graph:" + (str(error) if isinstance(error, Refusal) else "Record10Evidence"))
    return blockers


def invocation_context(args, parsed, env, cwd, session, inv):
    """Reached generator templates only; final Cargo events must bind their origin."""
    nested_loader = str(session / "target/debug") + ":"
    feature = any(value.startswith("src/probe/") for value in parsed["inputs"])
    nested = feature or (parsed["probe"] and "OUT_DIR" in env) or env.get(
        "DYLD_FALLBACK_LIBRARY_PATH", "").startswith(nested_loader)
    if not nested:
        return {"kind": "DirectCargoProbe" if parsed["probe"] else "DirectCargoCompile"}
    require(env.get("RUSTC") == inv["rustc"]["path"]
            and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and not env.get("RUSTC_WORKSPACE_WRAPPER")
            and env.get("HOST") == TARGET and env.get("TARGET") == TARGET,
            "NestedCompilerContext")
    name = "libc" if args[1:] == ["--version"] else "proc-macro2"
    version = {"libc": "0.2.184", "proc-macro2": "1.0.106"}[name]
    expected = {"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                "tree": "vendor", "manifest": name + "/Cargo.toml"}
    require(inv["packages"].count(expected) == 1, "NestedPackageInventory")
    package_root = session / "vendor" / name
    require(cwd == package_root and env.get("CARGO_MANIFEST_DIR") == str(package_root)
            and env.get("CARGO_PKG_NAME") == name, "NestedSourcePackage")
    for relative in (name + "/Cargo.toml", name + "/build.rs"):
        path = session / "vendor" / relative
        regular(path)
        require(path.resolve() == path and inv["vendor"]["files"].get(relative) == file_hash(path),
                "NestedSourceMismatch")
    raw_out = env.get("OUT_DIR", "")
    out = Path(raw_out)
    require(out.is_absolute() and str(out) == raw_out and out.resolve() == out
            and out.is_dir() and not out.is_symlink(), "NestedOutDir")
    target = session / "target"
    prefixes = [target / "debug/build"]
    if name == "libc":
        prefixes.append(target / TARGET / "debug/build")
    require(out.name == "out" and out.parent.parent in prefixes
            and re.fullmatch(r"[A-Za-z0-9_-]+", out.parent.name), "NestedOutDir")
    if name == "libc":
        kind, probe = "LibcBuildVersion", None
    else:
        require(len(parsed["inputs"]) == 1, "NestedProbeTemplate")
        source = parsed["inputs"][0]
        allowed = {"src/probe/" + part + ".rs" for part in (
            "proc_macro_span", "proc_macro_span_file", "proc_macro_span_location")}
        require(source in allowed, "NestedProbeTemplate")
        exact = ["--cfg=procmacro2_build_probe", "--edition=2021", "--crate-name=proc_macro2",
                 "--crate-type=lib", "--cap-lints=allow", "--emit=dep-info,metadata",
                 "--out-dir", str(out / "probe"), source, "--target", TARGET]
        require(args[1:] == exact, "NestedProbeTemplate")
        path = package_root / source
        regular(path)
        require(path.resolve() == path and inv["vendor"]["files"].get(name + "/" + source)
                == file_hash(path), "NestedSourceMismatch")
        kind, probe = "ProcMacro2FeatureProbe", source
    return {"kind": kind, "package_id": expected["id"], "manifest": str(package_root),
            "out_dir": str(out), "probe": probe}


# Source-derived finite num-traits@0.2.19/autocfg@1.5.0 bodies, not caller Rust.
AUTOCFG_BODIES = {
    "EmptyStd": b"", "NoStd": b"#![no_std]",
    "TotalCmp": b"pub fn probe() { let _ = 1f64.total_cmp(&2f64); }",
    "NoStdTotalCmp": b"#![no_std]\npub fn probe() { let _ = 1f64.total_cmp(&2f64); }",
}
AUTOCFG_KINDS = {"NumTraitsAutocfgVersion", "NumTraitsAutocfgStdinProbe"}


def autocfg_context(args, env, cwd, session, inv):
    """Closed raw templates before generic parsing rejects verbose/stdin/LLVM IR."""
    raw = args[1:]
    if "--verbose" not in raw and "--emit=llvm-ir" not in raw:
        return None
    package = {"id": "registry+https://github.com/rust-lang/crates.io-index#num-traits@0.2.19",
               "tree": "vendor", "manifest": "num-traits/Cargo.toml"}
    helper = {"id": "registry+https://github.com/rust-lang/crates.io-index#autocfg@1.5.0",
              "tree": "vendor", "manifest": "autocfg/Cargo.toml"}
    root = session / "vendor/num-traits"
    require(inv["packages"].count(package) == 1 and inv["packages"].count(helper) == 1,
            "AutocfgPackageInventory")
    require(cwd == root and env.get("CARGO_MANIFEST_DIR") == str(root)
            and env.get("CARGO_PKG_NAME") == "num-traits"
            and env.get("RUSTC") == inv["rustc"]["path"]
            and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and not env.get("RUSTC_WORKSPACE_WRAPPER")
            and env.get("HOST") == TARGET and env.get("TARGET") == TARGET, "AutocfgContext")
    for name in ("num-traits/Cargo.toml", "num-traits/build.rs", "autocfg/Cargo.toml",
                 "autocfg/src/lib.rs", "autocfg/src/rustc.rs", "autocfg/src/version.rs"):
        path = session / "vendor" / name
        regular(path)
        require(path.resolve() == path and inv["vendor"]["files"].get(name) == file_hash(path),
                "AutocfgSourceMismatch")
    raw_out = env.get("OUT_DIR", "")
    out = Path(raw_out)
    require(out.is_absolute() and str(out) == raw_out and out.resolve() == out
            and out.is_dir() and not out.is_symlink() and out.name == "out"
            and out.parent.parent == session / "target" / TARGET / "debug/build"
            and re.fullmatch(r"[A-Za-z0-9_-]+", out.parent.name), "AutocfgOutDir")
    context = {"kind": "NumTraitsAutocfgVersion", "package_id": package["id"],
               "manifest": str(root), "out_dir": str(out), "helper_package_id": helper["id"]}
    if raw == ["--version", "--verbose"]:
        return {"options": {"--version": [True], "--verbose": [True]}, "codegen": {},
                "inputs": [], "probe": True}, context
    require(len(raw) == 9 and raw[0] == "--crate-name", "AutocfgTemplate")
    match = re.fullmatch(r"autocfg_([0-9a-f]{16})_([0-2])", raw[1])
    require(match is not None and raw == ["--crate-name", raw[1], "--crate-type=lib", "--out-dir",
            str(out), "--emit=llvm-ir", "--target", TARGET, "-"], "AutocfgTemplate")
    context.update(kind="NumTraitsAutocfgStdinProbe", crate_name=raw[1],
                   prefix=match.group(1), index=int(match.group(2)))
    return {"options": {"--crate-name": [raw[1]], "--crate-type": ["lib"], "--out-dir": [str(out)],
                        "--emit": ["llvm-ir"], "--target": [TARGET]},
            "codegen": {}, "inputs": ["-"], "probe": False}, context


def capture_autocfg_stdin(call, context):
    raw, eof = bytearray(), False
    while len(raw) < 61:
        block = os.read(0, 61 - len(raw))
        if not block:
            eof = True
            break
        raw.extend(block)
    body = bytes(raw)
    path = call / "stdin.raw"
    with open(path, "xb") as stream:
        stream.write(body)
    template = next((name for name, expected in AUTOCFG_BODIES.items() if body == expected), None)
    evidence = {"snapshot": path.name, "length": len(body), "sha256": digest(body),
                "eof": eof, "truncated": not eof, "template": template}
    atomic_json(call / "stdin.json", evidence)
    require(eof and template is not None, "AutocfgStdinTemplate")
    allowed = {0: {"EmptyStd"}, 1: {"NoStd", "TotalCmp"},
               2: {"TotalCmp", "NoStdTotalCmp"}}
    require(template in allowed[context["index"]], "AutocfgStdinIndex")
    return body, evidence



RUSTIX_KIND = "RustixMetadataProbe"
RUSTIX_FEATURES = {"alloc", "default", "fs", "std", "stdio", "termios"}
RUSTIX_BODIES = (
    b"const unsafe fn foo(p: *const u8) -> isize { p.offset_from(p) }\n",
    b"fn a(x: &core::num::NonZeroI32, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { core::fmt::LowerExp::fmt(x, f) }\n",
    b"#[diagnostic::on_unimplemented()] trait Foo {}\n",
)
RUSTIX_CFGS = ("static_assertions", "lower_upper_exp_for_non_zero", "rustc_diagnostics")
FRAMEWORK_LITERAL = "framework=SystemConfiguration"


def fixed_source_package(name, version, cwd, env, session, inv, sources):
    package = {"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
               "tree": "vendor", "manifest": name + "/Cargo.toml"}
    root = session / "vendor" / name
    require(inv["packages"].count(package) == 1 and cwd == root
            and env.get("CARGO_MANIFEST_DIR") == str(root)
            and env.get("CARGO_PKG_NAME") == name and env.get("CARGO_PKG_VERSION") == version,
            "FixedPackageContext")
    for source in ("Cargo.toml", "build.rs", *sources):
        path = root / source
        regular(path)
        require(path.resolve() == path and inv["vendor"]["files"].get(name + "/" + source)
                == file_hash(path), "FixedPackageSource")
    raw = env.get("OUT_DIR", "")
    out = Path(raw)
    require(out.is_absolute() and str(out) == raw and out.resolve() == out and out.is_dir()
            and not out.is_symlink() and out.name == "out"
            and out.parent.parent == session / "target" / TARGET / "debug/build"
            and re.fullmatch(r"[A-Za-z0-9_-]+", out.parent.name), "FixedPackageOutDir")
    return package, root, out


RING_PACKAGE = "registry+https://github.com/rust-lang/crates.io-index#ring@0.17.14"
RING_LIBS = ("static=ring_core_0_17_14_", "static=ring_core_0_17_14__test")
RING_ARCHIVES = ("libring_core_0_17_14_.a", "libring_core_0_17_14__test.a")
RING_FEATURES = ("alloc", "default", "dev_urandom_fallback", "std")


def ring_static_context(args, env, cwd, session, inv):
    raw = args[1:]
    native = any(a.startswith("-l") or a.startswith("--extern-native") for a in raw)
    if not native:
        if env.get("CARGO_PKG_NAME") != "ring":
            return None
        # Recognition cannot depend on the declarations whose absence we reject.
        # Parse ordinary forms to preserve genuine Host builds and probe routes.
        candidate = parse_rustc(raw)
        options = candidate["options"]
        if (candidate["probe"] or options.get("--target") != [TARGET]
                or options.get("--crate-type") != ["lib"]):
            return None
    elif not (env.get("CARGO_PKG_NAME") == "ring"
              or any("ring_core_0_17_14_" in a for a in raw)):
        return None
    out_raw = env.get("OUT_DIR", "")
    require(raw[-6:] == ["-L", "native=" + out_raw, "-l", RING_LIBS[0], "-l", RING_LIBS[1]]
            and sum(a.startswith("-l") or a.startswith("--extern-native") for a in raw) == 2,
            "RingStaticTemplate")
    package, root, out = fixed_source_package("ring", "0.17.14", cwd, env, session, inv,
                                             ("src/lib.rs", "src/prefixed.rs"))
    require(re.fullmatch(r"ring-[0-9a-f]{16}", out.parent.name), "FixedPackageOutDir")
    parsed = parse_rustc(raw[:-6])
    o = parsed["options"]
    deps = session / "target" / TARGET / "debug/deps"
    require(not parsed["probe"] and parsed["inputs"] == [str(root / "src/lib.rs")]
            and o.get("--crate-name") == ["ring"] and o.get("--crate-type") == ["lib"]
            and o.get("--edition") == ["2021"] and o.get("--target") == [TARGET]
            and o.get("--out-dir") == [str(deps)] and o.get("--emit") == ["dep-info,metadata,link"]
            and "--test" not in o and not any(k in parsed["codegen"] for k in ("link-arg", "linker"))
            and o.get("-L") == ["dependency=" + str(deps), "dependency=" + str(session / "target/debug/deps")],
            "RingCompileContext")
    require(sorted(o.get("--cfg", [])) == sorted('feature="' + f + '"' for f in RING_FEATURES),
            "RingFeatureContext")
    externs = [v.partition("=") for v in o.get("--extern", [])]
    require(len(externs) == 3 and {v[0] for v in externs} == {"cfg_if", "getrandom", "untrusted"}
            and all(sep and Path(path).is_absolute() and Path(path).parent == deps
                    and str(Path(path)) == path and Path(path).resolve() == Path(path)
                    for _, sep, path in externs), "RingCompileContext")
    require(env.get("CARGO_MANIFEST_PATH") == str(root / "Cargo.toml")
            and env.get("CARGO_CRATE_NAME") == "ring"
            and all(env.get(k) == v for k, v in {"CARGO_PKG_VERSION_MAJOR": "0",
                "CARGO_PKG_VERSION_MINOR": "17", "CARGO_PKG_VERSION_PATCH": "14", "CARGO_PKG_VERSION_PRE": ""}.items())
            and env.get("RUSTC") == inv["rustc"]["path"]
            and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and not env.get("RUSTC_WORKSPACE_WRAPPER")
            and all(env.get(k) == inv["environment"].get(k) for k in ("CC", "CXX", "AR", "SDKROOT")),
            "RingEnvironmentContext")
    o["-L"].append("native=" + str(out))
    o["-l"] = list(RING_LIBS)
    return parsed, {"kind": "DirectCargoCompile", "ring_static_declarations": list(RING_LIBS),
                    "package_id": package["id"], "manifest": str(root), "out_dir": str(out)}


def ring_archive_observation(call, context, phase):
    require(phase in ("pre", "post"), "RingArchiveEvidence")
    observations = []
    try:
        for index, name in enumerate(RING_ARCHIVES):
            path = Path(context["out_dir"]) / name
            regular(path)
            require(path.resolve() == path and path.stat().st_nlink == 1, "RingArchiveEvidence")
            snapshot = call / ("ring-archive-" + phase + "-" + str(index) + ".raw")
            with open(path, "rb") as source, open(snapshot, "xb") as dest:
                shutil.copyfileobj(source, dest, 65536)
            sha = file_hash(snapshot)
            require(file_hash(path) == sha and path.stat().st_size == snapshot.stat().st_size,
                    "RingArchiveEvidence")
            observations.append({"path": str(path), "snapshot": snapshot.name,
                                 "length": snapshot.stat().st_size, "sha256": sha})
    except (OSError, Refusal) as error:
        raise Refusal("RingArchiveEvidence") from error
    return observations


def validate_ring_static_evidence(receipt, call, session, inv):
    request = strict_json((call / "request.json").read_bytes())
    initial = strict_json((call / "invocation.json").read_bytes())
    args = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
    env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v))
           for k, v in request["environment_hex"].items()}
    cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
    special = ring_static_context(args, env, cwd, session, inv)
    claimed = any("ring_static_declarations" in r.get("context", {})
                  or any(k.startswith("ring_archive_") for k in r) for r in (initial, receipt))
    if special is None:
        # The same raw classifier above rejects known Target/lib consumers even
        # when all native tokens and optional evidence annotations are absent.
        require(not claimed, "RingInvocationEvidence")
        return None
    require(claimed, "RingInvocationEvidence")
    parsed, context = special
    require(args[0] == inv["rustc"]["path"] and file_hash(Path(args[0])) == inv["rustc"]["sha256"]
            and receipt["compiler_sha256"] == inv["rustc"]["sha256"]
            and request["argv_hex"] == receipt["argv_hex"]
            and request["environment_hex"] == receipt["environment_hex"]
            and all(initial[k] == receipt[k] for k in ("context", "argv_hex", "environment_hex", "parsed",
                "source", "package", "role", "kind", "cwd", "compiler_sha256", "ring_archive_pre"))
            and receipt["parsed"] == parsed and receipt["context"] == context
            and receipt["cwd"] == str(cwd) and receipt["role"] == "Target" and receipt["kind"] == "Compile"
            and receipt["source"] == str(Path(context["manifest"]) / "src/lib.rs")
            and receipt["package"] == {"id": RING_PACKAGE, "tree": "vendor", "manifest": "ring/Cargo.toml"},
            "RingInvocationEvidence")
    compiler_environment(env, session, inv["sysroot"], probe=False, context=context)
    try:
        for phase in ("pre", "post"):
            rows = receipt.get("ring_archive_" + phase)
            require(isinstance(rows, list) and len(rows) == 2, "RingArchiveBinding")
            for index, row in enumerate(rows):
                require(set(row) == {"path", "snapshot", "length", "sha256"}
                        and row["path"] == str(Path(context["out_dir"]) / RING_ARCHIVES[index])
                        and row["snapshot"] == "ring-archive-" + phase + "-" + str(index) + ".raw",
                        "RingArchiveBinding")
                for path in (call / row["snapshot"], Path(row["path"])):
                    regular(path)
                    require(path.resolve() == path and path.stat().st_nlink == 1
                            and path.stat().st_size == row["length"] and file_hash(path) == row["sha256"],
                            "RingArchiveBinding")
        require(all({k: a[k] for k in ("path", "length", "sha256")} ==
                    {k: b[k] for k in ("path", "length", "sha256")}
                    for a, b in zip(receipt["ring_archive_pre"], receipt["ring_archive_post"])), "RingArchiveBinding")
    except (Refusal, OSError, KeyError, TypeError) as error:
        raise Refusal("RingArchiveBinding") from error
    return context


def ring_static_graph(receipts, associations, artifacts, events, edges, session, inv):
    blockers, declarations = [], []
    by_id = {r["invocation_id"]: r for r in receipts}
    for r in receipts:
        call = session / "invocations" / r["invocation_id"]
        # Do not reinterpret unrelated legacy receipts, including their negative fixtures.
        request = strict_json((call / "request.json").read_bytes())
        initial = strict_json((call / "invocation.json").read_bytes())
        candidates = [r, initial]
        claimed = any("ring_static_declarations" in item.get("context", {})
                      or any(k.startswith("ring_archive_") for k in item) for item in candidates)
        raw_candidates = request.get("argv_hex", [])
        ring_literal = os.fsencode("ring_core_0_17_14_").hex()
        if not (claimed or any(isinstance(a, str) and ring_literal in a for a in raw_candidates)
                or request.get("environment_hex", {}).get(os.fsencode("CARGO_PKG_NAME").hex())
                   == os.fsencode("ring").hex()):
            continue
        try:
            context = validate_ring_static_evidence(r, call, session, inv)
            if context is None:
                continue
            origins = [a for a in associations if a["package_id"] == RING_PACKAGE
                       and a["out_dir"] == context["out_dir"]]
            origin_events = [e for e in events if e["reason"] == "build-script-executed"
                             and e.get("package_id") == RING_PACKAGE]
            require(len(origins) == 1 and len(origin_events) == 1
                    and origins[0]["cargo_event"] == origin_events[0], "RingOrigin")
            association = origins[0]
            builder = by_id.get(association["producer_invocation"])
            require(builder is not None and builder["kind"] == "Compile" and builder["role"] == "Host"
                    and builder["exit_code"] == 0 and not builder["blockers"]
                    and builder["source"] == str(Path(context["manifest"]) / "build.rs")
                    and builder["package"]["id"] == RING_PACKAGE, "RingBuilder")
            ba = [a for a in artifacts if a["invocation_id"] == builder["invocation_id"] and "builder_alias" in a]
            be = [e for e in events if e["reason"] == "compiler-artifact" and e.get("package_id") == RING_PACKAGE
                  and e.get("target", {}).get("kind") == ["custom-build"]]
            require(len(ba) == len(be) == 1 and be[0]["target"].get("src_path") == builder["source"], "RingBuilder")
            require(sorted(builder["parsed"]["options"].get("--cfg", [])) ==
                    sorted('feature="' + f + '"' for f in RING_FEATURES)
                    and sorted(be[0].get("features", [])) == sorted(RING_FEATURES), "RingBuilderFeatures")
            cc = [e for e in edges if e["consumer"] == builder["invocation_id"] and e["name"] == "cc"]
            require(len(cc) == 1 and len(cc[0]["producers"]) == 1, "RingBuilderCc")
            helper = by_id[cc[0]["producers"][0]]
            require(helper["kind"] == "Compile" and helper["role"] == "Host" and helper["exit_code"] == 0
                    and not helper["blockers"] and helper["package"] == {
                        "id": "registry+https://github.com/rust-lang/crates.io-index#cc@1.2.59",
                        "tree": "vendor", "manifest": "cc/Cargo.toml"}
                    and helper["source"] == str(session / "vendor/cc/src/lib.rs"), "RingBuilderCc")
            event = association["cargo_event"]
            require(event["linked_libs"] == list(RING_LIBS)
                    and event["linked_paths"] == ["native=" + context["out_dir"]]
                    and event["cfgs"] == [] and event["env"] == [], "RingDeclaration")
            ca = [a for a in artifacts if a["invocation_id"] == r["invocation_id"]]
            ce = [e for e in events if e["reason"] == "compiler-artifact" and e.get("package_id") == RING_PACKAGE
                  and e.get("target", {}).get("src_path") == r["source"]]
            require(r["exit_code"] == 0 and not r["blockers"] and len(ca) == len(ce) == 1
                    and ce[0]["target"].get("kind") == ["lib"] and ce[0]["target"].get("crate_types") == ["lib"]
                    and ce[0]["target"].get("name") == "ring" and ce[0]["target"].get("edition") == "2021"
                    and len([v for v in receipts if v["kind"] == "Compile" and v["role"] == "Target"
                             and v.get("source") == r["source"]]) == 1 and sorted(ce[0].get("features", [])) == sorted(RING_FEATURES)
                    and ca[0]["files"] == ce[0]["filenames"], "RingConsumer")
            rust_edges = [e for e in edges if e["consumer"] == r["invocation_id"]]
            require(len(rust_edges) == 3 and {e["name"] for e in rust_edges} == {"cfg_if", "getrandom", "untrusted"}
                    and all(len(e["producers"]) == 1 and by_id[e["producers"][0]]["kind"] == "Compile"
                            and by_id[e["producers"][0]]["role"] == "Target"
                            and by_id[e["producers"][0]]["exit_code"] == 0
                            and not by_id[e["producers"][0]]["blockers"] for e in rust_edges), "RingConsumer")
            paths = {row["path"] for row in r["ring_archive_pre"]}
            hashes = {row["sha256"] for row in r["ring_archive_pre"]}
            require(all(association["generated_files"].get(name) == row["sha256"]
                        for name, row in zip(RING_ARCHIVES, r["ring_archive_pre"]))
                    and not any(o["path"] in paths or o.get("sha256") in hashes
                                for other in receipts for o in other["declared_outputs"] + other["outputs"])
                    and not any(e["path"] in paths for e in edges)
                    and not any(str((Path(other["cwd"]) / v).resolve()) in paths
                                for other in receipts for o in other["outputs"]
                                for v in o.get("dep_info", {}).get("paths", []))
                    and not any(f in paths or sha in hashes for a in artifacts for f, sha in a["file_sha256"].items())
                    and not any(str(Path(f).resolve()) in paths for e in events
                                if e["reason"] == "compiler-artifact" for f in e.get("filenames", [])),
                    "RingArchiveBinding")
            for index, literal in enumerate(RING_LIBS):
                declarations.append({"state": "RecordingOnly", "declaration": literal,
                    "raw_argument_indices": [len(r["argv_hex"]) - 4 + index * 2, len(r["argv_hex"]) - 3 + index * 2],
                    "consumer_invocation": r["invocation_id"], "producer_invocation": builder["invocation_id"],
                    "package_id": RING_PACKAGE, "out_dir": context["out_dir"],
                    "archive_pre": r["ring_archive_pre"][index], "archive_post": r["ring_archive_post"][index],
                    "artifact_selection": "not_observed", "native_child_provenance": "not_observed",
                    "native_producer_qualification": "not_issued"})
        except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError) as error:
            # This check also handles unclaimed contexts rederived from raw argv.
            blockers.append("RingGraph:" + (str(error) if isinstance(error, Refusal) else "RingInvocationEvidence"))
    return blockers, declarations



PSM_PACKAGE = "registry+https://github.com/rust-lang/crates.io-index#psm@0.1.30"
PSM_ARCHIVE = "libpsm_s.a"
PSM_CFGS = ("asm", "link_asm", "switchable_stack")
PSM_SOURCES = ("src/lib.rs", "src/arch/x86_64.s", "src/arch/psm.h", "src/arch/gnu_stack_note.s")


def psm_candidate(args, env, cwd, session):
    source = session / "vendor/psm/src/lib.rs"
    return (any(not a.startswith("-") and a.endswith(".rs") and (cwd / a).resolve() == source for a in args[1:])
            or env.get("CARGO_PKG_NAME") == "psm" or env.get("CARGO_CRATE_NAME") == "psm"
            or (env.get("CARGO_MANIFEST_DIR") and (cwd / env["CARGO_MANIFEST_DIR"]).resolve() == source.parent.parent)
            or any(a in ("static=psm_s", "-lstatic=psm_s") for a in args[1:]))


def psm_static_context(args, env, cwd, session, inv):
    if not psm_candidate(args, env, cwd, session):
        return None
    raw = args[1:]
    root = session / "vendor/psm"
    # The observed Host builder remains ordinary; no Host-library/probe exception.
    if not any(a.startswith(("-l", "--extern-native")) for a in raw):
        ordinary = parse_rustc(raw)
        if (ordinary["inputs"] == [str(root / "build.rs")] and not ordinary["probe"]
                and ordinary["options"].get("--crate-type") == ["bin"]
                and "--target" not in ordinary["options"]):
            return None
    try:
        package, root, out = fixed_source_package("psm", "0.1.30", cwd, env, session, inv, PSM_SOURCES)
        require(re.fullmatch(r"psm-[0-9a-f]{16}", out.parent.name), "PsmSourceContext")
    except (OSError, Refusal) as error:
        raise Refusal("PsmSourceContext") from error
    suffix = ["-L", "native=" + str(out), "-l", "static=psm_s", "--cfg", "asm", "--cfg", "link_asm",
              "--cfg", "switchable_stack", "--check-cfg", "cfg(switchable_stack,asm,link_asm)"]
    require(raw[-12:] == suffix, "PsmStaticTemplate")
    try:
        parsed = parse_rustc(raw[:-12] + raw[-8:])  # Only the validated balanced native pair is removed.
    except Refusal as error:
        raise Refusal("PsmStaticTemplate") from error
    o, c = parsed["options"], parsed["codegen"]
    metadata = c.get("metadata", [""])[0]; extra = c.get("extra-filename", [""])[0]
    deps, host = session / "target" / TARGET / "debug/deps", session / "target/debug/deps"
    exact = ["--crate-name", "psm", "--edition=2021", str(root / "src/lib.rs"), "--error-format=json",
             "--json=diagnostic-rendered-ansi,artifacts,future-incompat", "--crate-type", "lib",
             "--emit=dep-info,metadata,link", "-C", "embed-bitcode=no", "-C", "debuginfo=1",
             "-C", "split-debuginfo=unpacked", "--check-cfg", "cfg(docsrs,test)", "--check-cfg",
             "cfg(feature, values())", "-C", "metadata=" + metadata, "-C", "extra-filename=" + extra,
             "--out-dir", str(deps), "--target", TARGET, "-L", "dependency=" + str(deps),
             "-L", "dependency=" + str(host), "--cap-lints", "allow", *suffix]
    require(raw == exact and re.fullmatch(r"[0-9a-f]{16}", metadata)
            and re.fullmatch(r"-[0-9a-f]{16}", extra), "PsmStaticTemplate")
    require(env.get("CARGO_MANIFEST_PATH") == str(root / "Cargo.toml") and env.get("CARGO_CRATE_NAME") == "psm"
            and all(env.get(k) == v for k, v in {"CARGO_PKG_VERSION_MAJOR": "0", "CARGO_PKG_VERSION_MINOR": "1",
                "CARGO_PKG_VERSION_PATCH": "30", "CARGO_PKG_VERSION_PRE": ""}.items())
            and not any(k.startswith("CARGO_FEATURE_") for k in env)
            and env.get("RUSTC") == inv["rustc"]["path"] and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and not env.get("RUSTC_WORKSPACE_WRAPPER")
            and all(env.get(k) == inv["environment"].get(k) for k in ("CC", "CXX", "AR", "SDKROOT")),
            "PsmEnvironmentContext")
    o["-L"].append("native=" + str(out)); o["-l"] = ["static=psm_s"]
    return parsed, {"kind": "DirectCargoCompile", "psm_static_declaration": "static=psm_s", "package_id": package["id"],
                    "manifest": str(root), "out_dir": str(out), "native_argument_indices": list(range(len(args) - 12, len(args) - 8))}


def psm_archive_observation(call, context, phase):
    return static_archive_observation(call, context, phase, "psm", PSM_ARCHIVE, "PsmArchiveEvidence")


def static_archive_observation(call, context, phase, prefix, archive, reason):
    require(phase in ("pre", "post"), reason)
    try:
        path = Path(context["out_dir"]) / archive; regular(path)
        before = path.stat()
        require(path.resolve() == path and before.st_nlink == 1, reason)
        snapshot = call / (prefix + "-archive-" + phase + ".raw")
        with open(path, "rb") as source, open(snapshot, "xb") as dest:
            opened = os.fstat(source.fileno())
            require((opened.st_dev, opened.st_ino, opened.st_size) == (before.st_dev, before.st_ino, before.st_size),
                    reason)
            shutil.copyfileobj(source, dest, 65536)
            after = os.fstat(source.fileno())
        current = path.stat(); sha = file_hash(snapshot)
        require(path.resolve() == path and not path.is_symlink() and current.st_nlink == 1
                and (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                    (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns) ==
                    (current.st_dev, current.st_ino, current.st_size, current.st_mtime_ns, current.st_ctime_ns)
                and snapshot.stat().st_size == before.st_size and file_hash(path) == sha, reason)
        return {"path": str(path), "snapshot": snapshot.name, "length": before.st_size, "sha256": sha}
    except (OSError, Refusal) as error:
        raise Refusal(reason) from error


def psm_archive_namespace(args, env, cwd, call, session):
    # Negative-only exclusion: these names never establish production or acquisition authority.
    paths = {str(call / ("psm-archive-" + phase + ".raw")) for phase in ("pre", "post")}
    out = (cwd / env.get("OUT_DIR", "")).resolve()
    if (out.name == "out" and out.parent.parent == session / "target" / TARGET / "debug/build"
            and re.fullmatch(r"psm-[0-9a-f]{16}", out.parent.name)):
        paths.add(str(out / PSM_ARCHIVE))
    hashes = {file_hash(Path(p)) for p in paths if Path(p).is_file() and not Path(p).is_symlink()}
    return paths, hashes


def validate_psm_static_evidence(receipt, call, session, inv):
    request = strict_json((call / "request.json").read_bytes()); initial = strict_json((call / "invocation.json").read_bytes())
    args = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
    env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
    cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
    special = psm_static_context(args, env, cwd, session, inv)
    claimed = any("psm_static_declaration" in r.get("context", {}) or any(k.startswith("psm_archive_") for k in r)
                  for r in (initial, receipt))
    if special is None:
        require(not claimed, "PsmInvocationEvidence"); return None
    parsed, context = special
    require(claimed and args[0] == inv["rustc"]["path"] and file_hash(Path(args[0])) == inv["rustc"]["sha256"]
            and receipt["compiler_sha256"] == inv["rustc"]["sha256"]
            and request["argv_hex"] == initial["argv_hex"] == receipt["argv_hex"]
            and request["environment_hex"] == initial["environment_hex"] == receipt["environment_hex"]
            and all(initial[k] == receipt[k] for k in ("context", "parsed", "source", "package", "role", "kind", "cwd",
                                                       "compiler_sha256", "psm_archive_pre"))
            and receipt["parsed"] == parsed and receipt["context"] == context and receipt["cwd"] == str(cwd)
            and receipt["role"] == "Target" and receipt["kind"] == "Compile" and receipt["externs"] == []
            and receipt["source"] == str(session / "vendor/psm/src/lib.rs")
            and receipt["package"] == {"id": PSM_PACKAGE, "tree": "vendor", "manifest": "psm/Cargo.toml"},
            "PsmInvocationEvidence")
    compiler_environment(env, session, inv["sysroot"], probe=False, context=context)
    try:
        for phase in ("pre", "post"):
            row = receipt["psm_archive_" + phase]
            require(set(row) == {"path", "snapshot", "length", "sha256"}
                    and type(row["length"]) is int and row["length"] >= 0
                    and row["path"] == str(Path(context["out_dir"]) / PSM_ARCHIVE)
                    and row["snapshot"] == "psm-archive-" + phase + ".raw", "PsmArchiveBinding")
            for path in (call / row["snapshot"], Path(row["path"])):
                regular(path)
                require(path.resolve() == path and path.stat().st_nlink == 1 and path.stat().st_size == row["length"]
                        and file_hash(path) == row["sha256"], "PsmArchiveBinding")
        require(all(receipt["psm_archive_pre"][k] == receipt["psm_archive_post"][k]
                    for k in ("path", "length", "sha256")), "PsmArchiveBinding")
    except (Refusal, OSError, KeyError, TypeError) as error:
        raise Refusal("PsmArchiveBinding") from error
    return context


def psm_static_graph(receipts, associations, artifacts, events, edges, session, inv):
    blockers, declarations = [], []; by_id = {r["invocation_id"]: r for r in receipts}
    for r in receipts:
        call = session / "invocations" / r["invocation_id"]
        request = strict_json((call / "request.json").read_bytes())
        args = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
        env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
        cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
        if not (psm_candidate(args, env, cwd, session) or "psm_static_declaration" in r.get("context", {})
                or any(k.startswith("psm_archive_") for k in r)):
            continue
        try:
            context = validate_psm_static_evidence(r, call, session, inv)
            if context is None: continue
            origins = [a for a in associations if a["package_id"] == PSM_PACKAGE and a["out_dir"] == context["out_dir"]]
            oe = [e for e in events if e["reason"] == "build-script-executed" and e.get("package_id") == PSM_PACKAGE]
            require(len(origins) == len(oe) == 1 and origins[0]["cargo_event"] == oe[0], "PsmOrigin")
            association = origins[0]; builder = by_id.get(association["producer_invocation"])
            require(builder is not None and builder["kind"] == "Compile" and builder["role"] == "Host"
                    and builder["exit_code"] == 0 and not builder["blockers"] and builder["source"] == str(session / "vendor/psm/build.rs")
                    and builder["package"] == {"id": PSM_PACKAGE, "tree": "vendor", "manifest": "psm/Cargo.toml"}, "PsmBuilder")
            bcall = session / "invocations" / builder["invocation_id"]
            br = strict_json((bcall / "request.json").read_bytes()); bi = strict_json((bcall / "invocation.json").read_bytes())
            barg = [os.fsdecode(bytes.fromhex(a)) for a in br["argv_hex"]]
            benv = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in br["environment_hex"].items()}
            bcwd = Path(os.fsdecode(bytes.fromhex(br["cwd_hex"])))
            require(bcwd == session / "vendor/psm" and barg[0] == inv["rustc"]["path"]
                    and br["argv_hex"] == bi["argv_hex"] == builder["argv_hex"]
                    and br["environment_hex"] == bi["environment_hex"] == builder["environment_hex"]
                    and all(bi[k] == builder[k] for k in ("source", "package", "role", "kind", "cwd", "parsed", "compiler_sha256"))
                    and builder["parsed"] == parse_rustc(barg[1:]) and builder["cwd"] == str(bcwd)
                    and builder["parsed"]["inputs"] == [builder["source"]]
                    and builder["parsed"]["options"].get("--crate-type") == ["bin"]
                    and not builder["parsed"]["options"].get("--target") and builder["compiler_sha256"] == inv["rustc"]["sha256"]
                    and benv.get("CARGO_MANIFEST_DIR") == str(bcwd) and benv.get("CARGO_PKG_NAME") == "psm"
                    and benv.get("CARGO_PKG_VERSION") == "0.1.30", "PsmBuilder")
            compiler_environment(benv, session, inv["sysroot"], probe=False, context=builder["context"])
            ba = [a for a in artifacts if a["invocation_id"] == builder["invocation_id"] and "builder_alias" in a]
            be = [e for e in events if e["reason"] == "compiler-artifact" and e.get("package_id") == PSM_PACKAGE
                  and e.get("target", {}).get("kind") == ["custom-build"]]
            require(len(ba) == len(be) == 1 and be[0]["target"].get("src_path") == builder["source"]
                    and not be[0].get("features") and not builder["parsed"]["options"].get("--cfg")
                    and not any(k.startswith("CARGO_FEATURE_") for k in benv), "PsmBuilder")
            bedges = [e for e in edges if e["consumer"] == builder["invocation_id"]]
            require(len(bedges) == 2 and {e["name"] for e in bedges} == {"cc", "ar_archive_writer"}, "PsmBuilderExtern")
            for name, version in (("cc", "1.2.59"), ("ar_archive_writer", "0.5.1")):
                ee = [e for e in bedges if e["name"] == name]
                require(len(ee) == 1 and len(ee[0]["producers"]) == 1, "PsmBuilderExtern")
                helper = by_id[ee[0]["producers"][0]]
                require(helper["kind"] == "Compile" and helper["role"] == "Host" and helper["exit_code"] == 0
                        and not helper["blockers"] and helper["package"] == {"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                            "tree": "vendor", "manifest": name + "/Cargo.toml"}
                        and helper["source"] == str(session / "vendor" / name / "src/lib.rs"), "PsmBuilderExtern")
                hcall = session / "invocations" / helper["invocation_id"]
                hr = strict_json((hcall / "request.json").read_bytes()); hi = strict_json((hcall / "invocation.json").read_bytes())
                hargs = [os.fsdecode(bytes.fromhex(a)) for a in hr["argv_hex"]]
                require(hr["argv_hex"] == hi["argv_hex"] == helper["argv_hex"]
                        and hr["environment_hex"] == hi["environment_hex"] == helper["environment_hex"]
                        and Path(os.fsdecode(bytes.fromhex(hr["cwd_hex"]))) == session / "vendor" / name
                        and helper["cwd"] == str(session / "vendor" / name)
                        and all(hi[k] == helper[k] for k in ("source", "package", "role", "kind", "cwd", "parsed", "compiler_sha256"))
                        and hargs[0] == inv["rustc"]["path"] and helper["compiler_sha256"] == inv["rustc"]["sha256"]
                        and parse_rustc(hargs[1:]) == helper["parsed"] and not helper["parsed"]["probe"]
                        and "--target" not in helper["parsed"]["options"]
                        and helper["parsed"]["options"].get("--crate-type") == ["lib"]
                        and helper["context"] == hi["context"] == {"kind": "DirectCargoCompile"}
                        and helper["parsed"]["inputs"] == [helper["source"]]
                        and len([a for a in artifacts if a["invocation_id"] == helper["invocation_id"]]) == 1,
                        "PsmBuilderExtern")
            event = association["cargo_event"]
            require(event["linked_libs"] == ["static=psm_s"] and event["linked_paths"] == ["native=" + context["out_dir"]]
                    and event["cfgs"] == list(PSM_CFGS) and event["env"] == [], "PsmDeclaration")
            ca = [a for a in artifacts if a["invocation_id"] == r["invocation_id"]]
            ce = [e for e in events if e["reason"] == "compiler-artifact" and e.get("package_id") == PSM_PACKAGE
                  and e.get("target", {}).get("src_path") == r["source"]]
            require(r["exit_code"] == 0 and not r["blockers"] and len(ca) == len(ce) == 1 and ce[0].get("features") == []
                    and ce[0]["target"].get("kind") == ["lib"] and ce[0]["target"].get("crate_types") == ["lib"]
                    and ce[0]["target"].get("name") == "psm" and ce[0]["target"].get("edition") == "2021"
                    and len([v for v in receipts if v.get("source") == r["source"]]) == 1 and ca[0]["files"] == ce[0]["filenames"]
                    and set(ce[0]["filenames"]) == {o["path"] for o in r["declared_outputs"] if o["kind"] in ("metadata", "link")}
                    and not any(e["consumer"] == r["invocation_id"] for e in edges), "PsmConsumer")
            require(association["generated_files"].get(PSM_ARCHIVE) == r["psm_archive_pre"]["sha256"], "PsmArchiveBinding")
            paths, hashes = psm_archive_namespace(args, env, cwd, call, session)
            require(not any(o["path"] in paths or o.get("sha256") in hashes or (Path(o["path"]).is_file()
                                and not Path(o["path"]).is_symlink() and file_hash(Path(o["path"])) in hashes)
                            for v in receipts for o in v["declared_outputs"] + v["outputs"])
                    and not any(e["path"] in paths for e in edges)
                    and not any(str((Path(v["cwd"]) / q).resolve()) in paths or ((Path(v["cwd"]) / q).is_file()
                                and not (Path(v["cwd"]) / q).is_symlink() and file_hash(Path(v["cwd"]) / q) in hashes)
                                for v in receipts for o in v["outputs"] for q in o.get("dep_info", {}).get("paths", []))
                    and not any(f in paths or sha in hashes for a in artifacts for f, sha in a["file_sha256"].items())
                    and not any(str(Path(f).resolve()) in paths or (Path(f).is_file() and not Path(f).is_symlink()
                                and file_hash(Path(f)) in hashes) for e in events if e["reason"] == "compiler-artifact"
                                for f in e.get("filenames", [])), "PsmArchiveOwnership")
            declarations.append({"state": "RecordingOnly", "declaration": "static=psm_s", "raw_argument_indices": context["native_argument_indices"],
                "consumer_invocation": r["invocation_id"], "producer_invocation": builder["invocation_id"], "package_id": PSM_PACKAGE,
                "out_dir": context["out_dir"], "archive_pre": r["psm_archive_pre"], "archive_post": r["psm_archive_post"],
                "artifact_selection": "not_observed", "native_child_provenance": "not_observed", "native_producer_qualification": "not_issued"})
        except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError) as error:
            blockers.append("PsmGraph:" + (str(error) if isinstance(error, Refusal) else "PsmInvocationEvidence"))
    return blockers, declarations


ZSTD_PACKAGE = "registry+https://github.com/rust-lang/crates.io-index#zstd-sys@2.0.16+zstd.1.5.7"
ZSTD_FEATURES = ("legacy", "std", "zdict_builder")
ZSTD_SOURCES = ("Cargo.toml", "build.rs", "src/lib.rs", "src/bindings_zstd.rs", "src/bindings_zdict.rs")
ZSTD_CHECK = 'cfg(feature, values("bindgen", "debug", "default", "experimental", "fat-lto", "legacy", "no_asm", "no_wasm_shim", "non-cargo", "pkg-config", "seekable", "std", "thin", "thin-lto", "zdict_builder", "zstdmt"))'
ANYHOW_PACKAGE = "registry+https://github.com/rust-lang/crates.io-index#anyhow@1.0.102"
ANYHOW_KIND = "AnyhowStaticFeatureProbe"
ANYHOW_SOURCES = ("Cargo.toml", "build.rs", "src/backtrace.rs", "src/chain.rs", "src/context.rs", "src/ensure.rs",
    "src/error.rs", "src/fmt.rs", "src/kind.rs", "src/lib.rs", "src/macros.rs", "src/nightly.rs", "src/ptr.rs", "src/wrapper.rs")
SERDE_PACKAGE = "registry+https://github.com/rust-lang/crates.io-index#serde_core@1.0.228"
SERDE_SOURCES = ("Cargo.toml", "build.rs", "src/crate_root.rs", "src/de/ignored_any.rs", "src/de/impls.rs", "src/de/mod.rs",
    "src/de/value.rs", "src/format.rs", "src/lib.rs", "src/macros.rs", "src/private/content.rs", "src/private/doc.rs",
    "src/private/mod.rs", "src/private/seed.rs", "src/private/size_hint.rs", "src/private/string.rs", "src/ser/fmt.rs",
    "src/ser/impls.rs", "src/ser/impossible.rs", "src/ser/mod.rs", "src/std_error.rs")
SERDE_FEATURES = (("alloc", "default", "rc", "result", "std"), ("result", "std"))
SERDE_CHECK = 'cfg(feature, values("alloc", "default", "rc", "result", "std", "unstable"))'
SERDE_EXTRA_CHECKS = ("if_docsrs_then_no_serde_core", "no_core_cstr", "no_core_error", "no_core_net", "no_core_num_saturating",
    "no_diagnostic_namespace", "no_serde_derive", "no_std_atomic", "no_std_atomic64", "no_target_has_atomic")
SERDE_PRIVATE = b'#[doc(hidden)]\npub mod __private228 {\n    #[doc(hidden)]\n    pub use crate::private::*;\n}\n'
SERDE_MAPPING = "RecordingOnlySerdeCoreConsumerFeatureMappingV1"


def tools12_source(name, version, sources, env, cwd, session, inv, reason):
    root = session / "vendor" / name
    package = {"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
               "tree": "vendor", "manifest": name + "/Cargo.toml"}
    parts = version.split("+", 1)[0].split(".")
    try:
        require(inv["packages"].count(package) == 1 and cwd == root
                and env.get("CARGO_MANIFEST_DIR") == str(root) and env.get("CARGO_MANIFEST_PATH") == str(root / "Cargo.toml")
                and env.get("CARGO_PKG_NAME") == name and env.get("CARGO_PKG_VERSION") == version
                and all(env.get(k) == v for k, v in zip(("CARGO_PKG_VERSION_MAJOR", "CARGO_PKG_VERSION_MINOR",
                    "CARGO_PKG_VERSION_PATCH"), parts)) and env.get("CARGO_PKG_VERSION_PRE") == "", reason)
        for leaf in sources:
            path = root / leaf; regular(path)
            require(path.resolve() == path and path.stat().st_nlink == 1
                    and inv["vendor"]["files"].get(name + "/" + leaf) == file_hash(path), reason)
    except (Refusal, OSError) as error:
        raise Refusal(reason) from error
    return package, root


def tools12_out(name, env, session, *, host=False, reason):
    raw = env.get("OUT_DIR", ""); out = Path(raw)
    parent = session / "target/debug/build" if host else session / "target" / TARGET / "debug/build"
    require(out.is_absolute() and str(out) == raw and out.resolve() == out and out.is_dir() and not out.is_symlink()
            and out.name == "out" and out.parent.parent == parent
            and re.fullmatch(re.escape(name) + r"-[0-9a-f]{16}", out.parent.name), reason)
    return out


def tools12_candidate(name, source, args, env, cwd, session):
    root = session / "vendor" / name
    return (any(not a.startswith("-") and a.endswith(".rs") and (cwd / a).resolve() == root / source for a in args[1:])
            or env.get("CARGO_PKG_NAME") == name or env.get("CARGO_CRATE_NAME") == name.replace("-", "_")
            or bool(env.get("CARGO_MANIFEST_DIR")) and (cwd / env["CARGO_MANIFEST_DIR"]).resolve() == root)


def zstd_static_context(args, env, cwd, session, inv):
    if not (tools12_candidate("zstd-sys", "src/lib.rs", args, env, cwd, session)
            or any(a in ("static=zstd", "-lstatic=zstd") for a in args[1:])):
        return None
    raw = args[1:]; root = session / "vendor/zstd-sys"
    if not any(a.startswith(("-l", "--extern-native")) for a in raw):
        ordinary = parse_rustc(raw)
        if ordinary["inputs"] == [str(root / "build.rs")] and ordinary["options"].get("--crate-type") == ["bin"] and "--target" not in ordinary["options"]:
            return None
    package, root = tools12_source("zstd-sys", "2.0.16+zstd.1.5.7", ZSTD_SOURCES, env, cwd, session, inv, "ZstdSourceContext")
    out = tools12_out("zstd-sys", env, session, reason="ZstdSourceContext")
    suffix = ["-L", "native=" + str(out), "-l", "static=zstd"]
    require(raw[-4:] == suffix, "ZstdStaticTemplate")
    try:
        parsed = parse_rustc(raw[:-4])
    except Refusal as error:
        raise Refusal("ZstdStaticTemplate") from error
    c = parsed["codegen"]; metadata = c.get("metadata", [""])[0]; extra = c.get("extra-filename", [""])[0]
    deps, host = session / "target" / TARGET / "debug/deps", session / "target/debug/deps"
    exact = ["--crate-name", "zstd_sys", "--edition=2018", str(root / "src/lib.rs"), "--error-format=json",
        "--json=diagnostic-rendered-ansi,artifacts,future-incompat", "--crate-type", "lib", "--emit=dep-info,metadata,link",
        "-C", "embed-bitcode=no", "-C", "debuginfo=1", "-C", "split-debuginfo=unpacked", "--allow=non_upper_case_globals",
        *[v for f in ZSTD_FEATURES for v in ("--cfg", 'feature="' + f + '"')], "--check-cfg", "cfg(docsrs,test)",
        "--check-cfg", ZSTD_CHECK, "-C", "metadata=" + metadata, "-C", "extra-filename=" + extra,
        "--out-dir", str(deps), "--target", TARGET, "-L", "dependency=" + str(deps), "-L", "dependency=" + str(host),
        "--cap-lints", "allow", *suffix]
    require(raw == exact and re.fullmatch(r"[0-9a-f]{16}", metadata) and re.fullmatch(r"-[0-9a-f]{16}", extra), "ZstdStaticTemplate")
    require(env.get("CARGO_CRATE_NAME") == "zstd_sys" and not any(k.startswith("CARGO_FEATURE_") for k in env)
            and args[0] == inv["rustc"]["path"] and env.get("RUSTC") == args[0]
            and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and not any(k in env for k in ("RUSTC_WORKSPACE_WRAPPER", "RUSTC_STAGE", "RUSTC_BOOTSTRAP"))
            and all(env.get(k) == inv["environment"].get(k) for k in ("CC", "CXX", "AR", "SDKROOT")), "ZstdEnvironmentContext")
    parsed["options"]["-L"].append("native=" + str(out)); parsed["options"]["-l"] = ["static=zstd"]
    return parsed, {"kind": "DirectCargoCompile", "zstd_static_declaration": "static=zstd", "package_id": package["id"],
                    "manifest": str(root), "out_dir": str(out), "native_argument_indices": list(range(len(args) - 4, len(args)))}


def anyhow_candidate(args, env, cwd, session):
    return (tools12_candidate("anyhow", "src/nightly.rs", args, env, cwd, session)
            and (env.get("DYLD_FALLBACK_LIBRARY_PATH", "").startswith(str(session / "target/debug") + ":")
                 or "--cfg=anyhow_build_probe" in args[1:]
                 or any(a == "--cfg" and b == "anyhow_build_probe" for a, b in zip(args[1:], args[2:]))
                 or any(not a.startswith("-") and (a == "src/nightly.rs" or a.endswith("/src/nightly.rs")) for a in args[1:])))


def anyhow_context(args, env, cwd, session, inv):
    if not anyhow_candidate(args, env, cwd, session): return None
    package, root = tools12_source("anyhow", "1.0.102", ANYHOW_SOURCES, env, cwd, session, inv, "AnyhowSourceContext")
    out = tools12_out("anyhow", env, session, host=True, reason="AnyhowOutDir")
    require((out / "probe").resolve() == out / "probe", "AnyhowProbeDirectory")
    require({k for k in env if k.startswith("CARGO_FEATURE_")} == {"CARGO_FEATURE_DEFAULT", "CARGO_FEATURE_STD"}
            and env["CARGO_FEATURE_DEFAULT"] == env["CARGO_FEATURE_STD"] == "1", "AnyhowFeatures")
    require(args[0] == inv["rustc"]["path"] and env.get("RUSTC") == args[0]
            and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and not any(k in env for k in ("RUSTC_WORKSPACE_WRAPPER", "RUSTC_STAGE", "RUSTC_BOOTSTRAP"))
            and env.get("HOST") == env.get("TARGET") == TARGET and env.get("CARGO_ENCODED_RUSTFLAGS") == "", "AnyhowEnvironment")
    exact = ["--cfg=anyhow_build_probe", "--edition=2018", "--crate-name=anyhow", "--crate-type=lib", "--cap-lints=allow",
             "--emit=dep-info,metadata", "--out-dir", str(out / "probe"), "src/nightly.rs", "--target", TARGET]
    require(args[1:] == exact, "AnyhowTemplate")
    return parse_rustc(args[1:]), {"kind": ANYHOW_KIND, "package_id": package["id"], "manifest": str(root), "out_dir": str(out)}


def tools12_request(receipt, session, inv, reason, *, parsed=None):
    call = session / "invocations" / receipt["invocation_id"]
    request = strict_json((call / "request.json").read_bytes()); initial = strict_json((call / "invocation.json").read_bytes())
    args = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
    env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
    cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
    require(args[0] == inv["rustc"]["path"] and file_hash(Path(args[0])) == inv["rustc"]["sha256"]
            and receipt["compiler_sha256"] == inv["rustc"]["sha256"]
            and request["argv_hex"] == initial["argv_hex"] == receipt["argv_hex"]
            and request["environment_hex"] == initial["environment_hex"] == receipt["environment_hex"]
            and receipt["cwd"] == str(cwd) == os.fsdecode(bytes.fromhex(request["cwd_hex"]))
            and all(initial[k] == receipt[k] for k in ("context", "source", "package", "role", "kind", "cwd", "parsed",
                "compiler_sha256", "declared_outputs", "externs")), reason)
    require(receipt["parsed"] == (parse_rustc(args[1:]) if parsed is None else parsed)
            and env.get("RUSTC") == inv["rustc"]["path"] and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and not any(k in env for k in ("RUSTC_BOOTSTRAP", "RUSTC_STAGE", "RUSTC_WORKSPACE_WRAPPER"))
            and not env.get("CARGO_ENCODED_RUSTFLAGS"), reason)
    compiler_environment(env, session, inv["sysroot"], probe=False, context=receipt["context"])
    return call, args, env, cwd


def tools12_artifact(receipt, artifacts, events, features, kind, reason):
    aa = [a for a in artifacts if a["invocation_id"] == receipt["invocation_id"]]
    ee = [e for e in events if e["reason"] == "compiler-artifact" and e.get("package_id") == receipt["package"]["id"]
          and e.get("target", {}).get("src_path") == receipt["source"]
          and aa and e.get("filenames") == aa[0]["files"]]
    require(len(aa) == len(ee) == 1 and ee[0]["target"].get("kind") == [kind]
            and ee[0]["target"].get("crate_types") == ["bin" if kind == "custom-build" else kind]
            and sorted(ee[0].get("features", [])) == sorted(features)
            and len(features) == len(set(features))
            and ee[0]["target"].get("name") == ("build-script-build" if kind == "custom-build" else receipt["parsed"]["options"]["--crate-name"][0])
            and ee[0]["target"].get("edition") == receipt["parsed"]["options"]["--edition"][0]
            and ee[0].get("manifest_path") == str(Path(receipt["cwd"]) / "Cargo.toml"), reason)
    if kind == "custom-build":
        require(aa[0].get("builder_alias") == custom_build_producer(ee[0], receipt, aa[0]["file_sha256"]), reason)
        alias = aa[0]["builder_alias"]
        require(all(Path(alias[k]).is_file() and not Path(alias[k]).is_symlink()
                    and file_hash(Path(alias[k])) == alias["sha256"] for k in ("path", "link_path")), reason)
    else:
        require(set(aa[0]["files"]) == {o["path"] for o in receipt["declared_outputs"] if o["kind"] in ("metadata", "link")}, reason)
    return aa[0], ee[0]


def tools12_ordinary(receipt, name, version, source, role, features, session, inv, reason):
    _, args, env, cwd = tools12_request(receipt, session, inv, reason)
    root = session / "vendor" / name
    package = {"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
               "tree": "vendor", "manifest": name + "/Cargo.toml"}
    require(receipt["kind"] == "Compile" and receipt["context"]["kind"] == "DirectCargoCompile"
            and receipt["role"] == role and receipt["exit_code"] == 0 and not receipt["blockers"]
            and receipt["source"] == str(root / source) and receipt["package"] == package and cwd == root
            and receipt["parsed"]["inputs"] == [receipt["source"]]
            and (receipt["parsed"]["options"].get("--target") == [TARGET] if role == "Target" else "--target" not in receipt["parsed"]["options"])
            and sorted(receipt["parsed"]["options"].get("--cfg", [])) == sorted('feature="' + f + '"' for f in features)
            and env.get("CARGO_MANIFEST_DIR") == str(root) and env.get("CARGO_PKG_NAME") == name
            and env.get("CARGO_PKG_VERSION") == version and env.get("CARGO_MANIFEST_PATH") == str(root / "Cargo.toml")
            and env.get("CARGO_CRATE_NAME") == ("build_script_build" if source == "build.rs" else name.replace("-", "_"))
            and receipt["parsed"]["options"].get("--crate-name") == [env["CARGO_CRATE_NAME"]]
            and receipt["parsed"]["options"].get("--crate-type") == ["bin" if source == "build.rs" else "lib"]
            and receipt["parsed"]["options"].get("--edition") == ["2018" if name in {"zstd-sys", "cc", "pkg-config"} else "2021"]
            and env.get("RUSTC") == inv["rustc"]["path"]
            and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and not any(k in env for k in ("RUSTC_BOOTSTRAP", "RUSTC_STAGE", "RUSTC_WORKSPACE_WRAPPER"))
            and not env.get("CARGO_ENCODED_RUSTFLAGS"), reason)
    feature_keys = {k for k in env if k.startswith("CARGO_FEATURE_")}
    require(not feature_keys or feature_keys == {"CARGO_FEATURE_" + f.upper() for f in features}
            and all(env[k] == "1" for k in feature_keys), reason)
    require(all(Path(o["path"]).is_file() and not Path(o["path"]).is_symlink()
                and file_hash(Path(o["path"])) == o["sha256"] for o in receipt["outputs"]), reason)
    return args, env, cwd


def tools12_namespace(name, args, env, cwd, call, session):
    # Negative-only current and retained bytes; no qualified annotations needed.
    archive = name == "zstd-sys"
    leaves = ("zstd-archive-pre.raw", "zstd-archive-post.raw") if archive else ("probe-output-0.raw", "probe-output-1.raw")
    paths = {str(call / leaf) for leaf in leaves} if archive or anyhow_candidate(args, env, cwd, session) else set()
    out = (cwd / env.get("OUT_DIR", "")).resolve()
    parent = session / "target" / TARGET / "debug/build" if archive else session / "target/debug/build"
    if out.name == "out" and out.parent.parent == parent and re.fullmatch(re.escape(name) + r"-[0-9a-f]{16}", out.parent.name):
        paths.update({str(out / "libzstd.a")} if archive else {str(out / "probe" / leaf) for leaf in ("anyhow.d", "libanyhow.rmeta")})
    hashes = {file_hash(Path(p)) for p in paths if Path(p).is_file() and not Path(p).is_symlink()}
    for leaf in ("invocation.json", "receipt.json"):
        path = call / leaf
        if not path.is_file() or path.is_symlink(): continue
        data = strict_json(path.read_bytes())
        rows = [data.get("zstd_archive_" + phase, {}) for phase in ("pre", "post")] if archive else (
            data.get("outputs", []) if anyhow_candidate(args, env, cwd, session) or data.get("context", {}).get("kind") == ANYHOW_KIND else [])
        for row in rows:
            sha = row.get("sha256") if isinstance(row, dict) else None
            if isinstance(sha, str) and re.fullmatch(r"[0-9a-f]{64}", sha): hashes.add(sha)
    return paths, hashes


def tools12_owned(path, sha, paths, hashes):
    p = Path(path)
    return str(p.resolve()) in paths or sha in hashes or p.is_file() and not p.is_symlink() and file_hash(p) in hashes


def zstd_evidence(receipt, session, inv):
    call = session / "invocations" / receipt["invocation_id"]
    request = strict_json((call / "request.json").read_bytes())
    args = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
    env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
    cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
    special = zstd_static_context(args, env, cwd, session, inv)
    claimed = "zstd_static_declaration" in receipt.get("context", {}) or any(k.startswith("zstd_archive_") for k in receipt)
    if special is None:
        require(not claimed, "ZstdInvocationEvidence"); return None
    parsed, context = special
    tools12_request(receipt, session, inv, "ZstdInvocationEvidence", parsed=parsed)
    initial = strict_json((call / "invocation.json").read_bytes())
    require(receipt["context"] == context and receipt["role"] == "Target" and receipt["kind"] == "Compile"
            and receipt["source"] == str(session / "vendor/zstd-sys/src/lib.rs") and receipt["externs"] == []
            and receipt["package"] == {"id": ZSTD_PACKAGE, "tree": "vendor", "manifest": "zstd-sys/Cargo.toml"}
            and receipt["declared_outputs"] == selected_outputs(parsed, cwd, session / "target")
            and initial.get("zstd_archive_pre") == receipt.get("zstd_archive_pre"), "ZstdInvocationEvidence")
    try:
        for phase in ("pre", "post"):
            row = receipt["zstd_archive_" + phase]
            require(set(row) == {"path", "snapshot", "length", "sha256"} and type(row["length"]) is int and row["length"] >= 0
                    and row["path"] == str(Path(context["out_dir"]) / "libzstd.a")
                    and row["snapshot"] == "zstd-archive-" + phase + ".raw", "ZstdArchiveBinding")
            for path in (call / row["snapshot"], Path(row["path"])):
                regular(path)
                require(path.resolve() == path and path.stat().st_nlink == 1 and path.stat().st_size == row["length"]
                        and file_hash(path) == row["sha256"], "ZstdArchiveBinding")
        require(all(receipt["zstd_archive_pre"][k] == receipt["zstd_archive_post"][k] for k in ("path", "length", "sha256")), "ZstdArchiveBinding")
    except (Refusal, OSError, KeyError, TypeError) as error:
        raise Refusal("ZstdArchiveBinding") from error
    return context


def zstd_graph(receipts, associations, artifacts, events, edges, session, inv, paths, hashes):
    blockers, declarations = [], []; by_id = {r["invocation_id"]: r for r in receipts}
    for r in receipts:
        try:
            context = zstd_evidence(r, session, inv)
            if context is None: continue
            origins = [a for a in associations if a["package_id"] == ZSTD_PACKAGE and a["out_dir"] == context["out_dir"]]
            oe = [e for e in events if e["reason"] == "build-script-executed" and e.get("package_id") == ZSTD_PACKAGE]
            require(len(origins) == len(oe) == 1 and origins[0]["cargo_event"] == oe[0], "ZstdOrigin")
            association = origins[0]; builder = by_id[association["producer_invocation"]]
            tools12_ordinary(builder, "zstd-sys", "2.0.16+zstd.1.5.7", "build.rs", "Host", ZSTD_FEATURES, session, inv, "ZstdBuilder")
            require(builder["parsed"]["options"].get("--crate-type") == ["bin"], "ZstdBuilder")
            tools12_artifact(builder, artifacts, events, ZSTD_FEATURES, "custom-build", "ZstdBuilder")
            be = [e for e in edges if e["consumer"] == builder["invocation_id"]]
            require(len(be) == 2 and {e["name"] for e in be} == {"cc", "pkg_config"}, "ZstdBuilderExtern")
            for edge_name, name, version, features in (("cc", "cc", "1.2.59", ("parallel",)), ("pkg_config", "pkg-config", "0.3.32", ())):
                ee = [e for e in be if e["name"] == edge_name]
                require(len(ee) == 1 and len(ee[0]["producers"]) == 1, "ZstdBuilderExtern")
                helper = by_id[ee[0]["producers"][0]]
                tools12_ordinary(helper, name, version, "src/lib.rs", "Host", features, session, inv, "ZstdBuilderExtern")
                tools12_artifact(helper, artifacts, events, features, "lib", "ZstdBuilderExtern")
            event = association["cargo_event"]
            require(event["linked_libs"] == ["static=zstd"] and event["linked_paths"] == ["native=" + context["out_dir"]]
                    and event["cfgs"] == event["env"] == [], "ZstdDeclaration")
            require(r["exit_code"] == 0 and not r["blockers"]
                    and len([v for v in receipts if v["source"] == r["source"]]) == 1, "ZstdConsumer")
            tools12_artifact(r, artifacts, events, ZSTD_FEATURES, "lib", "ZstdConsumer")
            require(association["generated_files"].get("libzstd.a") == r["zstd_archive_pre"]["sha256"], "ZstdArchiveBinding")
            require(not any(tools12_owned(o["path"], o.get("sha256"), paths, hashes)
                            for v in receipts for o in v["declared_outputs"] + v["outputs"])
                    and not any(tools12_owned(e["path"], None, paths, hashes) for e in edges)
                    and not any(tools12_owned(str((Path(v["cwd"]) / q).resolve()), None, paths, hashes)
                                for v in receipts for o in v["outputs"] for q in o.get("dep_info", {}).get("paths", []))
                    and not any(tools12_owned(f, None, paths, hashes) for e in events if e["reason"] == "compiler-artifact" for f in e.get("filenames", [])),
                    "ZstdArchiveOwnership")
            declarations.append({"state": "RecordingOnly", "declaration": "static=zstd", "raw_argument_indices": context["native_argument_indices"],
                "consumer_invocation": r["invocation_id"], "producer_invocation": builder["invocation_id"], "package_id": ZSTD_PACKAGE,
                "out_dir": context["out_dir"], "archive_pre": r["zstd_archive_pre"], "archive_post": r["zstd_archive_post"],
                "artifact_selection": "not_observed", "native_child_provenance": "not_observed", "native_producer_qualification": "not_issued"})
        except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError) as error:
            blockers.append("ZstdGraph:" + (str(error) if isinstance(error, Refusal) else "ZstdInvocationEvidence"))
    return blockers, declarations


def anyhow_evidence(receipt, session, inv):
    call = session / "invocations" / receipt["invocation_id"]
    request = strict_json((call / "request.json").read_bytes())
    args = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
    env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
    cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
    special = anyhow_context(args, env, cwd, session, inv)
    if special is None:
        require(receipt.get("context", {}).get("kind") != ANYHOW_KIND, "AnyhowInvocationEvidence"); return None
    parsed, context = special
    tools12_request(receipt, session, inv, "AnyhowInvocationEvidence", parsed=parsed)
    outputs = selected_outputs(parsed, cwd, session / "target")
    require(receipt["context"] == context and receipt["kind"] == "TransientProbe" and receipt["role"] == "Target"
            and receipt["source"] == str(cwd / "src/nightly.rs") and receipt["package"] == {"id": ANYHOW_PACKAGE, "tree": "vendor", "manifest": "anyhow/Cargo.toml"}
            and receipt["declared_outputs"] == outputs and receipt["externs"] == []
            and receipt["probe_outcome"] == {0: "Supported", 1: "Unsupported"}.get(receipt["exit_code"], "CompilerFailure"), "AnyhowInvocationEvidence")
    declarations = {o["path"]: (i, o) for i, o in enumerate(outputs)}
    require(len({o["path"] for o in receipt["outputs"]}) == len(receipt["outputs"])
            and (receipt["exit_code"] != 0 or {o["path"] for o in receipt["outputs"]} == set(declarations)), "AnyhowProbeEvidence")
    for output in receipt["outputs"]:
        require(output["path"] in declarations, "AnyhowProbeEvidence")
        index, declaration = declarations[output["path"]]; snapshot = call / ("probe-output-" + str(index) + ".raw")
        require(output["kind"] == declaration["kind"] and output["snapshot"] == snapshot.name, "AnyhowProbeEvidence")
        regular(snapshot)
        require(snapshot.resolve() == snapshot and snapshot.stat().st_nlink == 1 and file_hash(snapshot) == output["sha256"], "AnyhowSnapshot")
        current = Path(output["path"])
        if current.exists() or current.is_symlink():
            regular(current)
            require(current.resolve() == current and current.stat().st_nlink == 1 and file_hash(current) == output["sha256"], "AnyhowSnapshot")
        if output["kind"] == "dep-info": require(output.get("dep_info") == dep_info(snapshot.read_bytes()), "AnyhowDepInfo")
    return context


def anyhow_graph(receipts, associations, artifacts, events, session, inv):
    if not any(isinstance(r.get("package"), dict) and r["package"]["id"] == ANYHOW_PACKAGE for r in receipts): return []
    try:
        children = [(r, anyhow_evidence(r, session, inv)) for r in receipts]
        children = [(r, c) for r, c in children if c is not None]
        require(len(children) == 1, "AnyhowChildJoin")
        child, context = children[0]
        require(child["exit_code"] in (0, 1) and not child["blockers"], "AnyhowChildJoin")
        origins = [a for a in associations if a["package_id"] == ANYHOW_PACKAGE]
        oe = [e for e in events if e["reason"] == "build-script-executed" and e.get("package_id") == ANYHOW_PACKAGE]
        require(len(origins) == len(oe) == 1 and origins[0]["cargo_event"] == oe[0]
                and origins[0]["out_dir"] == context["out_dir"], "AnyhowOriginJoin")
        association = origins[0]; by_id = {r["invocation_id"]: r for r in receipts}; builder = by_id[association["producer_invocation"]]
        tools12_ordinary(builder, "anyhow", "1.0.102", "build.rs", "Host", ("default", "std"), session, inv, "AnyhowBuilderJoin")
        tools12_artifact(builder, artifacts, events, ("default", "std"), "custom-build", "AnyhowBuilderJoin")
        consumers = [r for r in receipts if r["source"] == str(session / "vendor/anyhow/src/lib.rs")]
        require(len(consumers) == 1, "AnyhowConsumerJoin"); consumer = consumers[0]
        cfgs = ["error_generic_member_access"] if child["exit_code"] == 0 else []
        event = association["cargo_event"]
        require(event["cfgs"] == cfgs and all(event[k] == [] for k in ("linked_libs", "linked_paths", "env"))
                and sorted(consumer["parsed"]["options"].get("--cfg", [])) == sorted(['feature="default"', 'feature="std"'] + cfgs), "AnyhowCfgJoin")
        # Validate the ordinary Host consumer without treating generated cfg as a feature.
        _, _, env, cwd = tools12_request(consumer, session, inv, "AnyhowConsumerJoin")
        tools12_source("anyhow", "1.0.102", ANYHOW_SOURCES, env, cwd, session, inv, "AnyhowConsumerJoin")
        require(consumer["kind"] == "Compile" and consumer["context"]["kind"] == "DirectCargoCompile"
                and consumer["role"] == "Host" and "--target" not in consumer["parsed"]["options"]
                and consumer["exit_code"] == 0 and not consumer["blockers"] and env.get("OUT_DIR") == context["out_dir"]
                and env.get("CARGO_CRATE_NAME") == "anyhow"
                and consumer["parsed"]["options"].get("--crate-name") == ["anyhow"]
                and consumer["parsed"]["options"].get("--crate-type") == ["lib"], "AnyhowConsumerJoin")
        tools12_artifact(consumer, artifacts, events, ("default", "std"), "lib", "AnyhowConsumerJoin")
        require(association["generated_files"] == {}, "AnyhowGeneratedJoin")
        return []
    except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError) as error:
        return ["AnyhowGraph:" + (str(error) if isinstance(error, Refusal) else "AnyhowInvocationEvidence")]


def serde_core_mapping(event, producers, receipts, artifacts, events, session, inv):
    """Finite RecordingOnly mapping rule; Cargo did not observe alias execution."""
    reason = "SerdeCoreMapping"
    require(event["cfgs"] == event["env"] == event["linked_libs"] == event["linked_paths"] == [], reason)
    consumers = [r for r in receipts if r["source"] == str(session / "vendor/serde_core/src/lib.rs")]
    require(len(consumers) == 1, reason); consumer = consumers[0]
    _, args, env, cwd = tools12_request(consumer, session, inv, reason)
    tools12_source("serde_core", "1.0.228", SERDE_SOURCES, env, cwd, session, inv, reason)
    out = tools12_out("serde_core", env, session, reason=reason)
    require(str(out) == event["out_dir"] and consumer["role"] == "Target" and consumer["package"]["id"] == SERDE_PACKAGE
            and consumer["kind"] == "Compile" and consumer["context"]["kind"] == "DirectCargoCompile"
            and consumer["exit_code"] == 0 and not consumer["blockers"] and consumer["externs"] == [], reason)
    features = tuple(v[len('feature="'):-1] for v in consumer["parsed"]["options"].get("--cfg", []) if v.startswith('feature="') and v.endswith('"'))
    feature_keys = {k for k in env if k.startswith("CARGO_FEATURE_")}
    require(features in SERDE_FEATURES and env.get("CARGO_CRATE_NAME") == "serde_core"
            and (not feature_keys or feature_keys == {"CARGO_FEATURE_" + f.upper() for f in features}
                 and all(env[k] == "1" for k in feature_keys)), reason)
    tools12_artifact(consumer, artifacts, events, features, "lib", reason)
    serde_core_template(args, features, cwd, session, builder=False)
    compatible = []
    for artifact in producers:
        builder = next(r for r in receipts if r["invocation_id"] == artifact["invocation_id"])
        _, bargs, benv, bcwd = tools12_request(builder, session, inv, reason)
        tools12_source("serde_core", "1.0.228", SERDE_SOURCES, benv, bcwd, session, inv, reason)
        bf = tuple(v[len('feature="'):-1] for v in builder["parsed"]["options"].get("--cfg", []) if v.startswith('feature="') and v.endswith('"'))
        require(bf in SERDE_FEATURES and not any(k in benv for k in ("OUT_DIR", "HOST", "TARGET")), reason)
        tools12_ordinary(builder, "serde_core", "1.0.228", "build.rs", "Host", bf, session, inv, reason)
        tools12_artifact(builder, artifacts, events, bf, "custom-build", reason)
        serde_core_template(bargs, bf, bcwd, session, builder=True)
        if bf == features: compatible.append(artifact)
    require(len(compatible) == 1, reason)
    private = out / "private.rs"; regular(private)
    require(private.resolve() == private and private.stat().st_nlink == 1 and private.read_bytes() == SERDE_PRIVATE, reason)
    di = [o["dep_info"] for o in consumer["outputs"] if o["kind"] == "dep-info"]
    require(len(di) == 1 and str(private) in {str((cwd / p).resolve()) for p in di[0]["paths"]}
            and os.fsencode("OUT_DIR=" + str(out)).hex() in di[0]["environment_comment_hex"], reason)
    return compatible, {"state": "RecordingOnly", "rule": SERDE_MAPPING, "execution_edge": "not_observed",
                        "consumer_invocation": consumer["invocation_id"], "features": list(features)}


def serde_core_template(args, features, root, session, *, builder):
    parsed = parse_rustc(args[1:]); c = parsed["codegen"]
    metadata = c.get("metadata", [""])[0]; extra = c.get("extra-filename", [""])[0]
    require(re.fullmatch(r"[0-9a-f]{16}", metadata) and re.fullmatch(r"-[0-9a-f]{16}", extra), "SerdeCoreMapping")
    dest = session / "target/debug/build" / ("serde_core" + extra) if builder else session / "target" / TARGET / "debug/deps"
    exact = ["--crate-name", "build_script_build" if builder else "serde_core", "--edition=2021", str(root / ("build.rs" if builder else "src/lib.rs")),
        "--error-format=json", "--json=diagnostic-rendered-ansi,artifacts,future-incompat", "--crate-type", "bin" if builder else "lib",
        "--emit=" + ("dep-info,link" if builder else "dep-info,metadata,link"), "-C", "embed-bitcode=no", "-C", "debuginfo=1", "-C", "split-debuginfo=unpacked",
        *[v for f in features for v in ("--cfg", 'feature="' + f + '"')], "--check-cfg", "cfg(docsrs,test)", "--check-cfg", SERDE_CHECK,
        "-C", "metadata=" + metadata, "-C", "extra-filename=" + extra, "--out-dir", str(dest)]
    if not builder: exact += ["--target", TARGET, "-L", "dependency=" + str(dest)]
    exact += ["-L", "dependency=" + str(session / "target/debug/deps"), "--cap-lints", "allow"]
    if not builder: exact += [v for f in SERDE_EXTRA_CHECKS for v in ("--check-cfg", "cfg(" + f + ")")]
    require(args[1:] == exact, "SerdeCoreMapping")


def framework_context(args, env, cwd, session, inv):
    raw = args[1:]
    if not any(a == "-l" or a.startswith("-l") or a.startswith("--extern-native") for a in raw):
        return None
    require(raw[-2:] == ["-l", FRAMEWORK_LITERAL]
            and sum(a == "-l" or a.startswith("-l") for a in raw) == 1, "FrameworkTemplate")
    package, root, out = fixed_source_package("system-configuration-sys", "0.6.0", cwd, env,
                                            session, inv, ("src/lib.rs",))
    parsed = parse_rustc(raw[:-2])
    options = parsed["options"]
    require(not parsed["probe"] and parsed["inputs"] == [str(root / "src/lib.rs")]
            and options.get("--crate-name") == ["system_configuration_sys"]
            and options.get("--crate-type") == ["lib"] and options.get("--target") == [TARGET]
            and options.get("--out-dir") == [str(session / "target" / TARGET / "debug/deps")]
            and options.get("--emit") == ["dep-info,metadata,link"]
            and not any(k in parsed["codegen"] for k in ("link-arg", "linker"))
            and all(value in {"dependency=" + str(session / "target" / TARGET / "debug/deps"),
                              "dependency=" + str(session / "target/debug/deps")}
                    for value in options.get("-L", [])),
            "FrameworkCompileContext")
    options["-l"] = [FRAMEWORK_LITERAL]
    return parsed, {"kind": "DirectCargoCompile", "framework_declaration": FRAMEWORK_LITERAL,
                    "package_id": package["id"], "manifest": str(root), "out_dir": str(out)}


def rustix_context(args, env, cwd, session, inv):
    raw = args[1:]
    if "-o" not in raw:
        return None
    package, root, out = fixed_source_package("rustix", "1.1.4", cwd, env, session, inv, ())
    require(raw == ["--crate-type=rlib", "--emit=metadata", "--target", TARGET,
                    "-o", str(out / "rustix_test_can_compile"), "-"], "RustixTemplate")
    require(env.get("RUSTC") == inv["rustc"]["path"]
            and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and not env.get("RUSTC_WORKSPACE_WRAPPER")
            and env.get("HOST") == TARGET and env.get("TARGET") == TARGET,
            "RustixCompilerContext")
    expected = {"CARGO_CFG_TARGET_ARCH": "x86_64", "CARGO_CFG_TARGET_OS": "macos",
                "CARGO_CFG_TARGET_ENDIAN": "little", "CARGO_CFG_TARGET_POINTER_WIDTH": "64",
                "CARGO_CFG_TARGET_ABI": "", "CARGO_CFG_TARGET_ENV": ""}
    require(all(env.get(k) == v for k, v in expected.items())
            and {k: v for k, v in env.items() if k.startswith("CARGO_FEATURE_")} ==
                {"CARGO_FEATURE_" + f.upper(): "1" for f in RUSTIX_FEATURES}
            and not any(k in env for k in ("CARGO_CFG_MIRI", "CARGO_CFG_RUSTIX_USE_EXPERIMENTAL_FEATURES",
                "CARGO_CFG_RUSTIX_USE_EXPERIMENTAL_ASM", "CARGO_CFG_RUSTIX_USE_LIBC", "CARGO_CFG_RUSTIX_NO_LINUX_RAW"))
            and not env.get("CARGO_ENCODED_RUSTFLAGS"), "RustixConfiguration")
    return ({"options": {"--crate-type": ["rlib"], "--emit": ["metadata"], "--target": [TARGET],
                         "-o": [str(out / "rustix_test_can_compile")]},
             "codegen": {}, "inputs": ["-"], "probe": False},
            {"kind": RUSTIX_KIND, "package_id": package["id"], "manifest": str(root), "out_dir": str(out)})


def capture_rustix_stdin(call):
    raw, eof = bytearray(), False
    while len(raw) < 123:
        block = os.read(0, 123 - len(raw))
        if not block:
            eof = True
            break
        raw.extend(block)
    body = bytes(raw)
    with open(call / "stdin.raw", "xb") as stream:
        stream.write(body)
    index = RUSTIX_BODIES.index(body) if body in RUSTIX_BODIES else None
    evidence = {"snapshot": "stdin.raw", "length": len(body), "sha256": digest(body),
                "eof": eof, "truncated": not eof, "template_index": index}
    atomic_json(call / "stdin.json", evidence)
    require(eof and index is not None, "RustixStdinTemplate")
    return body, evidence


def metadata_state(call, path, phase):
    if not path.exists() and not path.is_symlink():
        return {"exists": False}
    regular(path)
    require(path.resolve() == path, "RustixMetadataAlias")
    snapshot = call / ("metadata-" + phase + ".raw")
    with open(path, "rb") as source, open(snapshot, "xb") as dest:
        shutil.copyfileobj(source, dest)
    sha = file_hash(snapshot)
    require(file_hash(path) == sha, "RustixMetadataChanged")
    return {"exists": True, "sha256": sha, "snapshot": snapshot.name}


def state_identity(state):
    return (state["exists"], state.get("sha256"))


def rustix_predecessor(session, call, context, index, pre):
    previous = {}
    for directory in (session / "invocations").iterdir():
        if directory == call or not (directory / "invocation.json").is_file():
            continue
        request = strict_json((directory / "invocation.json").read_bytes())
        other = request.get("context", {})
        if other.get("kind") != RUSTIX_KIND or (other.get("package_id"), other.get("out_dir")) != (context["package_id"], context["out_dir"]):
            continue
        require((directory / "receipt.json").is_file(), "RustixPredecessorIncomplete")
        receipt = strict_json((directory / "receipt.json").read_bytes())
        number = receipt["stdin"]["template_index"]
        require(number not in previous and receipt["exit_code"] in (0, 1) and not receipt["blockers"],
                "RustixPredecessor")
        previous[number] = (directory.name, receipt)
    require(set(previous) == set(range(index)), "RustixSequence")
    if index == 0:
        require(not pre["exists"], "RustixInitialMetadata")
        return None
    identifier, receipt = previous[index - 1]
    require(state_identity(pre) == state_identity(receipt["metadata_post"]), "RustixPrestate")
    return identifier


def verify_metadata_state(directory, state, phase):
    require(isinstance(state, dict) and type(state.get("exists")) is bool, "RustixMetadataEvidence")
    if not state["exists"]:
        require(state == {"exists": False}, "RustixMetadataEvidence")
        return
    require(set(state) == {"exists", "sha256", "snapshot"}
            and state["snapshot"] == "metadata-" + phase + ".raw", "RustixMetadataEvidence")
    path = directory / state["snapshot"]
    regular(path)
    require(path.resolve() == path and file_hash(path) == state["sha256"], "RustixMetadataEvidence")


def verify_rustix_evidence(directory, receipt):
    path = directory / "stdin.raw"
    regular(path)
    require(path.resolve() == path and path.stat().st_size <= 122, "RustixStdinEvidence")
    body = path.read_bytes()
    index = RUSTIX_BODIES.index(body) if body in RUSTIX_BODIES else None
    expected = {"snapshot": "stdin.raw", "length": len(body), "sha256": digest(body),
                "eof": True, "truncated": False, "template_index": index}
    metadata = directory / "stdin.json"
    regular(metadata)
    require(index is not None and metadata.resolve() == metadata and receipt["stdin"] == expected
            and strict_json(metadata.read_bytes()) == expected, "RustixStdinEvidence")
    initial = strict_json((directory / "invocation.json").read_bytes())
    require(all(initial[k] == receipt[k] for k in ("context", "stdin", "declared_outputs", "metadata_pre", "predecessor_invocation",
                                                   "argv_hex", "environment_hex", "parsed", "role", "kind")),
            "RustixInvocationEvidence")
    output = str(Path(receipt["context"]["out_dir"]) / "rustix_test_can_compile")
    require(receipt["kind"] == "TransientProbe" and receipt["declared_outputs"] == [{"path": output, "kind": "metadata"}],
            "RustixDeclarationEvidence")
    for phase in ("pre", "post"):
        verify_metadata_state(directory, receipt["metadata_" + phase], phase)
    post = receipt["metadata_post"]
    expected_outputs = ([{"path": output, "kind": "metadata", "sha256": post["sha256"],
                         "snapshot": post["snapshot"], "observation_only": True}] if post["exists"] else [])
    require(receipt["outputs"] == expected_outputs and (receipt["exit_code"] != 0 or post["exists"]),
            "RustixOutputEvidence")


def new_role_graphs(receipts, associations):
    blockers, declarations = [], []
    by_id = {r["invocation_id"]: r for r in receipts}
    def origin(context, name):
        found = [a for a in associations if a["package_id"] == context["package_id"] and a["out_dir"] == context["out_dir"]]
        require(len(found) == 1, name + "Origin")
        association = found[0]
        builder = by_id[association["producer_invocation"]]
        require(builder["kind"] == "Compile" and builder["role"] == "Host" and builder["exit_code"] == 0
                and not builder["blockers"] and builder["source"] == str(Path(context["manifest"]) / "build.rs")
                and builder["package"]["id"] == context["package_id"], name + "Builder")
        return association, builder
    groups = {}
    for r in receipts:
        c = r.get("context", {})
        if c.get("kind") == RUSTIX_KIND:
            groups.setdefault((c["package_id"], c["out_dir"]), []).append(r)
        if "framework_declaration" in c:
            try:
                association, _ = origin(c, "Framework")
                event = association["cargo_event"]
                require(r["kind"] == "Compile" and r["role"] == "Target" and r["exit_code"] == 0 and not r["blockers"]
                        and c["framework_declaration"] == FRAMEWORK_LITERAL and r["parsed"]["options"].get("-l") == [FRAMEWORK_LITERAL]
                        and event["linked_libs"] == [FRAMEWORK_LITERAL]
                        and all(event[k] == [] for k in ("linked_paths", "cfgs", "env")), "FrameworkDeclaration")
                declarations.append({"state": "RecordingOnly", "consumer_invocation": r["invocation_id"],
                                     "producer_invocation": association["producer_invocation"],
                                     "declaration": FRAMEWORK_LITERAL, "out_dir": c["out_dir"]})
            except (Refusal, KeyError, TypeError) as error:
                blockers.append("FrameworkGraph:" + str(error))
    for group in groups.values():
        try:
            association, builder = origin(group[0]["context"], "Rustix")
            require(len(builder["parsed"]["options"].get("--cfg", [])) == len(RUSTIX_FEATURES)
                    and set(builder["parsed"]["options"].get("--cfg", [])) == {'feature="' + f + '"' for f in RUSTIX_FEATURES},
                    "RustixBuilderFeatures")
            indexed = {r["stdin"]["template_index"]: r for r in group}
            require(len(group) == 3 and set(indexed) == {0, 1, 2}, "RustixSequence")
            previous, post = None, {"exists": False}
            cfgs = []
            for index in range(3):
                r = indexed[index]
                require(r["exit_code"] in (0, 1) and not r["blockers"]
                        and r["predecessor_invocation"] == previous
                        and state_identity(r["metadata_pre"]) == state_identity(post), "RustixSequence")
                previous, post = r["invocation_id"], r["metadata_post"]
                if r["exit_code"] == 0:
                    cfgs.append(RUSTIX_CFGS[index])
            event = association["cargo_event"]
            require(event["cfgs"] == cfgs + ["libc", "apple", "bsd"]
                    and all(event[k] == [] for k in ("linked_libs", "linked_paths", "env")), "RustixCfgGraph")
        except (Refusal, KeyError, TypeError) as error:
            blockers.append("RustixGraph:" + str(error))
    return blockers, declarations


def compiler_environment(env, session, sysroot, *, probe, context=None):
    require(not any(env.get(key) for key in FORBIDDEN_ENV), "CompilerEnvironmentInjection")
    expected = sysroot_loader_path(sysroot)
    kind = (context or {}).get("kind")
    if kind in {"LibcBuildVersion", "ProcMacro2FeatureProbe", RUSTIX_KIND, ANYHOW_KIND} | AUTOCFG_KINDS | RECORD10_KINDS:
        relative = "lib/rustlib/" + TARGET + "/lib"
        host_lib = Path(sysroot["root"]) / relative
        require(host_lib.is_dir() and host_lib.resolve() == host_lib
                and any(name.startswith(relative + "/") for name in sysroot["files"]),
                "IncompleteHostLoaderInventory")
        components = (str(session / "target/debug"), str(session / "target/debug/deps"), str(host_lib), expected)
        require(all(":" not in part for part in components), "SessionLoaderPath")
        expected = ":".join(components)
    elif not probe:
        prefix = str(session / "target/debug/deps")
        require(":" not in prefix, "SessionLoaderPath")
        expected = prefix + ":" + expected
    # Original strings only: do not normalize, reorder, or accept empty components.
    require(env.get("DYLD_FALLBACK_LIBRARY_PATH") == expected, "CompilerEnvironmentInjection")


def capture_transient_outputs(call, outputs, code):
    captured, blockers = [], []
    for index, item in enumerate(outputs):
        path = Path(item["path"])
        if not path.exists() and not path.is_symlink():
            if code == 0:
                blockers.append("MissingDeclaredOutput:" + str(path))
            continue
        require(path.resolve() == path, "TransientOutputAlias")
        regular(path)
        snapshot = call / ("probe-output-" + str(index) + ".raw")
        with open(path, "rb") as source, open(snapshot, "xb") as dest:
            shutil.copyfileobj(source, dest)
        output = dict(item, sha256=file_hash(snapshot), snapshot=snapshot.name)
        require(file_hash(path) == output["sha256"], "TransientOutputChanged")
        if item["kind"] == "dep-info":
            try:
                output["dep_info"] = dep_info(snapshot.read_bytes())
            except Refusal as error:
                blockers.append(str(error))
        captured.append(output)
    return captured, blockers


def inherited_jobserver_fds(env):
    """Forward only the original pipe pair from the trusted fixed Cargo parent."""
    require("MAKEFLAGS" not in env and "MFLAGS" not in env, "UnsupportedJobserverEnvironment")
    raw = env.get("CARGO_MAKEFLAGS")
    if raw is None:
        return ()
    match = re.fullmatch(r"-j --jobserver-fds=([1-9][0-9]*),([1-9][0-9]*) "
                         r"--jobserver-auth=\1,\2", raw)
    require(match is not None, "UnsupportedJobserverFlags")
    try:
        read, write = (int(value) for value in match.groups())
        require(read >= 3 and write >= 3 and read != write, "InvalidJobserverDescriptors")
        for fd, access in ((read, os.O_RDONLY), (write, os.O_WRONLY)):
            require(stat.S_ISFIFO(os.fstat(fd).st_mode), "InvalidJobserverDescriptors")
            require(fcntl.fcntl(fd, fcntl.F_GETFL) & os.O_ACCMODE == access,
                    "InvalidJobserverDescriptors")
    except (OSError, OverflowError, ValueError):
        raise Refusal("InvalidJobserverDescriptors") from None
    return (read, write)


# Finite prospective origins from frozen metadata SHA-256
# 1b87b599363d23bfa26ddf176f12a449193e4154da894c731607a96b898c795a.
# Eligibility is not a reached/selected-artifact claim. All paths are explicit;
# duplicate crate names are distinguished by full package ID, never normalized.
BARE_PROC_MACRO_ORIGINS = {
    ('registry+https://github.com/rust-lang/crates.io-index#async-stream-impl@0.3.6', 'async_stream_impl'):
        ('async-stream-impl', '0.3.6', 'async-stream-impl/Cargo.toml', 'async-stream-impl/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#async-trait@0.1.89', 'async_trait'):
        ('async-trait', '0.1.89', 'async-trait/Cargo.toml', 'async-trait/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#bincode_derive@2.0.1', 'bincode_derive'):
        ('bincode_derive', '2.0.1', 'bincode_derive/Cargo.toml', 'bincode_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#bytemuck_derive@1.10.2', 'bytemuck_derive'):
        ('bytemuck_derive', '1.10.2', 'bytemuck_derive/Cargo.toml', 'bytemuck_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#clap_derive@4.6.0', 'clap_derive'):
        ('clap_derive', '4.6.0', 'clap_derive/Cargo.toml', 'clap_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#darling_macro@0.14.4', 'darling_macro'):
        ('darling_macro', '0.14.4', 'darling_macro-0.14.4/Cargo.toml', 'darling_macro-0.14.4/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#darling_macro@0.21.3', 'darling_macro'):
        ('darling_macro', '0.21.3', 'darling_macro/Cargo.toml', 'darling_macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#derive_builder_macro@0.12.0', 'derive_builder_macro'):
        ('derive_builder_macro', '0.12.0', 'derive_builder_macro/Cargo.toml', 'derive_builder_macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#diesel_derives@2.3.7', 'diesel_derives'):
        ('diesel_derives', '2.3.7', 'diesel_derives/Cargo.toml', 'diesel_derives/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#displaydoc@0.2.5', 'displaydoc'):
        ('displaydoc', '0.2.5', 'displaydoc/Cargo.toml', 'displaydoc/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#document-features@0.2.12', 'document_features'):
        ('document-features', '0.2.12', 'document-features/Cargo.toml', 'document-features/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#foreign-types-macros@0.2.3', 'foreign_types_macros'):
        ('foreign-types-macros', '0.2.3', 'foreign-types-macros/Cargo.toml', 'foreign-types-macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#futures-macro@0.3.32', 'futures_macro'):
        ('futures-macro', '0.3.32', 'futures-macro/Cargo.toml', 'futures-macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#jiff-static@0.2.23', 'jiff_static'):
        ('jiff-static', '0.2.23', 'jiff-static/Cargo.toml', 'jiff-static/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#num-derive@0.4.2', 'num_derive'):
        ('num-derive', '0.4.2', 'num-derive/Cargo.toml', 'num-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#openssl-macros@0.1.1', 'openssl_macros'):
        ('openssl-macros', '0.1.1', 'openssl-macros/Cargo.toml', 'openssl-macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#pin-project-internal@1.1.13', 'pin_project_internal'):
        ('pin-project-internal', '1.1.13', 'pin-project-internal/Cargo.toml', 'pin-project-internal/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#prost-derive@0.14.4', 'prost_derive'):
        ('prost-derive', '0.14.4', 'prost-derive/Cargo.toml', 'prost-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#recursive-proc-macro-impl@0.1.1', 'recursive_proc_macro_impl'):
        ('recursive-proc-macro-impl', '0.1.1', 'recursive-proc-macro-impl/Cargo.toml', 'recursive-proc-macro-impl/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#rustversion@1.0.22', 'rustversion'):
        ('rustversion', '1.0.22', 'rustversion/Cargo.toml', 'rustversion/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#serde_derive@1.0.228', 'serde_derive'):
        ('serde_derive', '1.0.228', 'serde_derive/Cargo.toml', 'serde_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#serial_test_derive@0.10.0', 'serial_test_derive'):
        ('serial_test_derive', '0.10.0', 'serial_test_derive/Cargo.toml', 'serial_test_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#sqlparser_derive@0.4.0', 'sqlparser_derive'):
        ('sqlparser_derive', '0.4.0', 'sqlparser_derive/Cargo.toml', 'sqlparser_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#strum_macros@0.27.2', 'strum_macros'):
        ('strum_macros', '0.27.2', 'strum_macros/Cargo.toml', 'strum_macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#thiserror-impl@1.0.69', 'thiserror_impl'):
        ('thiserror-impl', '1.0.69', 'thiserror-impl-1.0.69/Cargo.toml', 'thiserror-impl-1.0.69/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#thiserror-impl@2.0.18', 'thiserror_impl'):
        ('thiserror-impl', '2.0.18', 'thiserror-impl/Cargo.toml', 'thiserror-impl/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#time-macros@0.2.32', 'time_macros'):
        ('time-macros', '0.2.32', 'time-macros/Cargo.toml', 'time-macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#tokio-macros@2.7.0', 'tokio_macros'):
        ('tokio-macros', '2.7.0', 'tokio-macros/Cargo.toml', 'tokio-macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#tracing-attributes@0.1.31', 'tracing_attributes'):
        ('tracing-attributes', '0.1.31', 'tracing-attributes/Cargo.toml', 'tracing-attributes/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#wasm-bindgen-macro@0.2.117', 'wasm_bindgen_macro'):
        ('wasm-bindgen-macro', '0.2.117', 'wasm-bindgen-macro/Cargo.toml', 'wasm-bindgen-macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#windows-implement@0.60.2', 'windows_implement'):
        ('windows-implement', '0.60.2', 'windows-implement/Cargo.toml', 'windows-implement/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#windows-interface@0.59.3', 'windows_interface'):
        ('windows-interface', '0.59.3', 'windows-interface/Cargo.toml', 'windows-interface/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#wit-bindgen-rust-macro@0.51.0', 'wit_bindgen_rust_macro'):
        ('wit-bindgen-rust-macro', '0.51.0', 'wit-bindgen-rust-macro/Cargo.toml', 'wit-bindgen-rust-macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#yoke-derive@0.8.2', 'yoke_derive'):
        ('yoke-derive', '0.8.2', 'yoke-derive/Cargo.toml', 'yoke-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#zerocopy-derive@0.8.48', 'zerocopy_derive'):
        ('zerocopy-derive', '0.8.48', 'zerocopy-derive/Cargo.toml', 'zerocopy-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#zerofrom-derive@0.1.7', 'zerofrom_derive'):
        ('zerofrom-derive', '0.1.7', 'zerofrom-derive/Cargo.toml', 'zerofrom-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#zerovec-derive@0.11.3', 'zerovec_derive'):
        ('zerovec-derive', '0.11.3', 'zerovec-derive/Cargo.toml', 'zerovec-derive/src/lib.rs'),
}
BARE_PROC_MACRO_CANDIDATES = tuple(
    "lib/rustlib/x86_64-apple-darwin/lib/libproc_macro-b94f7a67a9654a0b." + suffix
    for suffix in ("rlib", "rmeta"))


def raw_bare_externs(args):
    """Keep token positions; deliberately do not normalize the extern spelling."""
    result, i = [], 1  # argv[0] is the compiler, also retained in argument_index.
    while i < len(args):
        value = args[i]
        if value == "--extern" and i + 1 < len(args):
            if "=" not in args[i + 1]:
                result.append((i + 1, args[i + 1], True))
            i += 2
        elif value.startswith("--extern="):
            if "=" not in value[len("--extern="):]:
                result.append((i, value[len("--extern="):], False))
            i += 1
        else:
            # A cfg/check-cfg/codegen value is not another command-line option.
            i += 2 if value in RUSTC_VALUE_FLAGS or value in {"-C", "-L", "-A", "-W", "-D", "-F"} else 1
    return result


def bare_proc_macro_declarations(args, parsed, context, cwd, session, inv, package, source, env):
    bare = raw_bare_externs(args)
    if not bare:
        return []
    require(all(value == "proc_macro" and separated for _, value, separated in bare),
            "UnsupportedBareExtern")
    options = parsed["options"]
    require(len(bare) == 1 and options.get("--extern", []).count("proc_macro") == 1
            and not any(value.startswith("proc_macro=") for value in options.get("--extern", [])),
            "BareProcMacroAmbiguous")
    host = str(session / "target/debug/deps")
    require(not parsed["probe"] and context == {"kind": "DirectCargoCompile"}
            and "--target" not in options and options.get("--crate-type") == ["proc-macro"]
            and options.get("--emit") == ["dep-info,link"]
            and options.get("--out-dir") == [host] and options.get("-L") == ["dependency=" + host],
            "BareProcMacroRole")
    crate = options.get("--crate-name", [None])[0]
    key = (package["id"], crate)
    require(key in BARE_PROC_MACRO_ORIGINS, "BareProcMacroOrigin")
    name, version, manifest_relative, source_relative = BARE_PROC_MACRO_ORIGINS[key]
    expected = {"id": key[0], "tree": "vendor", "manifest": manifest_relative}
    manifest = session / "vendor" / manifest_relative
    expected_source = session / "vendor" / source_relative
    require(package == expected and inv["packages"].count(expected) == 1
            and cwd == manifest.parent and source == expected_source
            and env.get("CARGO_MANIFEST_DIR") == str(manifest.parent)
            and env.get("CARGO_MANIFEST_PATH") == str(manifest)
            and env.get("CARGO_PKG_NAME") == name and env.get("CARGO_PKG_VERSION") == version,
            "BareProcMacroOrigin")
    try:
        for path in (manifest, expected_source):
            regular(path)
            require(path.resolve() == path and inv["vendor"]["files"].get(
                path.relative_to(session / "vendor").as_posix()) == file_hash(path), "BareProcMacroOrigin")
    except (OSError, Refusal):
        raise Refusal("BareProcMacroOrigin") from None
    candidates = []
    try:
        spec = inv["sysroot"]
        root = Path(spec["root"])
        require(root.is_absolute() and root.resolve() == root, "BareProcMacroSysroot")
        prefix = "lib/rustlib/x86_64-apple-darwin/lib/libproc_macro-"
        require({key for key in spec["files"] if key.startswith(prefix)} == set(BARE_PROC_MACRO_CANDIDATES),
                "BareProcMacroSysroot")
        for relative in BARE_PROC_MACRO_CANDIDATES:
            path = root / relative
            regular(path)
            require(path.resolve() == path and spec["files"][relative] == file_hash(path), "BareProcMacroSysroot")
            candidates.append({"relative_path": relative, "sha256": spec["files"][relative]})
    except (KeyError, OSError, Refusal):
        raise Refusal("BareProcMacroSysroot") from None
    # Availability is not resolver selection: rustc can also search the -L path.
    return [{"kind": "BareProcMacroSearchV1", "name": "proc_macro", "host": TARGET,
             "compiler_sha256": inv["rustc"]["sha256"], "sysroot_root": inv["sysroot"]["root"],
             "argument_index": bare[0][0], "candidates": candidates, "artifact_selection": "not_observed"}]


def verify_sysroot_extern_declaration(directory, receipt, session, inv):
    request = strict_json((directory / "request.json").read_bytes())
    initial = strict_json((directory / "invocation.json").read_bytes())
    raw = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
    if not (raw_bare_externs(raw) or "sysroot_extern_declarations" in initial
            or "sysroot_extern_declarations" in receipt
            or raw_bare_externs([os.fsdecode(bytes.fromhex(a)) for a in receipt["argv_hex"]])
            or raw_bare_externs([os.fsdecode(bytes.fromhex(a)) for a in initial["argv_hex"]])):
        return []
    require(request.get("state") == initial.get("state") == receipt.get("state") == "RecordingOnly"
            and all(initial[k] == receipt[k] for k in ("argv_hex", "environment_hex", "parsed", "context",
                "cwd", "source", "package", "role", "kind", "compiler_sha256", "externs", "declared_outputs"))
            and request["argv_hex"] == receipt["argv_hex"]
            and request["environment_hex"] == receipt["environment_hex"]
            and os.fsdecode(bytes.fromhex(request["cwd_hex"])) == receipt["cwd"],
            "ChangedSysrootExternDeclaration")
    parsed = parse_rustc(raw[1:])
    env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v))
           for k, v in request["environment_hex"].items()}
    cwd = Path(receipt["cwd"])
    require(raw[0] == inv["rustc"]["path"] and parsed == receipt["parsed"]
            and receipt["role"] == "Host" and receipt["kind"] == "Compile"
            and receipt["compiler_sha256"] == inv["rustc"]["sha256"]
            and invocation_context(raw, parsed, env, cwd, session, inv) == receipt["context"],
            "ChangedSysrootExternDeclaration")
    compiler_environment(env, session, inv["sysroot"], probe=False, context=receipt["context"])
    require("--sysroot" not in parsed["options"] or parsed["options"]["--sysroot"] == [inv["sysroot"]["root"]],
            "ChangedSysrootExternDeclaration")
    source = (cwd / parsed["inputs"][0]).resolve()
    require(str(source) == receipt["source"], "ChangedSysrootExternDeclaration")
    expected = bare_proc_macro_declarations(raw, parsed, receipt["context"], cwd, session, inv,
                                            receipt["package"], source, env)
    require(expected and initial.get("sysroot_extern_declarations") == expected
            and receipt.get("sysroot_extern_declarations") == expected, "ChangedSysrootExternDeclaration")
    externs = []
    for value in parsed["options"].get("--extern", []):
        if value == "proc_macro":
            continue
        name, separator, path = value.partition("=")
        require(separator and re.fullmatch(r"[A-Za-z0-9_]+", name), "ChangedSysrootExternDeclaration")
        path = (cwd / path).resolve()
        require(inside(path, session / "target"), "ChangedSysrootExternDeclaration")
        externs.append({"name": name, "path": str(path)})
    require(receipt["externs"] == externs
            and receipt["declared_outputs"] == selected_outputs(parsed, cwd, session / "target"),
            "ChangedSysrootExternDeclaration")
    return expected


# Closed bundled acquisition only; this evidence never issues a provider.
BUNDLED_PROFILE = "normaldev-library-bundledsqlite-v1"
NATIVE_SCHEMA = "stock-analysis-replay-native-record-v1"
SQLITE_PACKAGE = "registry+https://github.com/rust-lang/crates.io-index#libsqlite3-sys@0.28.0"
SQLITE_FEATURES = ("bundled", "bundled_bindings", "cc", "default", "min_sqlite_version_3_14_0", "pkg-config", "vcpkg")
CC_PACKAGE = "registry+https://github.com/rust-lang/crates.io-index#cc@1.2.59"
PROBE_LITERAL = "cc/src/detect_compiler_family.c"
PROBE_DIGEST = "97ca4b021495611e828becea6187add37414186a16dfedd26c2947cbce6e8b2f"
SQLITE_MEMBERS = ("build.rs", "Cargo.toml", "src/lib.rs", "sqlite3/sqlite3.c", "sqlite3/sqlite3.h", "sqlite3/bindgen_bundled_version.rs")
SQLITE_DEFINES = frozenset(("SQLITE_CORE", "SQLITE_DEFAULT_FOREIGN_KEYS=1", "SQLITE_ENABLE_API_ARMOR",
    "SQLITE_ENABLE_COLUMN_METADATA", "SQLITE_ENABLE_DBSTAT_VTAB", "SQLITE_ENABLE_FTS3",
    "SQLITE_ENABLE_FTS3_PARENTHESIS", "SQLITE_ENABLE_FTS5", "SQLITE_ENABLE_JSON1",
    "SQLITE_ENABLE_LOAD_EXTENSION=1", "SQLITE_ENABLE_MEMORY_MANAGEMENT", "SQLITE_ENABLE_RTREE",
    "SQLITE_ENABLE_STAT2", "SQLITE_ENABLE_STAT4", "SQLITE_SOUNDEX", "SQLITE_THREADSAFE=1",
    "SQLITE_USE_URI", "HAVE_USLEEP=1", "_POSIX_THREAD_SAFE_FUNCTIONS", "HAVE_ISNAN", "HAVE_LOCALTIME_R"))


def native_launchers(session, inv):
    result = {}
    for role in ("cc", "ar"):
        path = session / ("native-" + role)
        text = ("#!" + inv["python"]["path"] + " -I\nimport os,sys\nos.execv("
                + repr(inv["python"]["path"]) + ", [" + repr(inv["python"]["path"])
                + ", '-I', " + repr(str(Path(__file__).resolve())) + ", '_native', "
                + repr(session.name.removeprefix("pending-")) + ", " + repr(role) + ", *sys.argv[1:]])\n")
        with open(path, "x", encoding="utf-8") as dest:
            dest.write(text)
        path.chmod(0o700)
        result[role] = {"path": str(path), "sha256": file_hash(path)}
    return result


def native_control(session, policy, owner):
    inv = policy["inventory"]
    require(policy["profile"] == BUNDLED_PROFILE and owner["profile"] == BUNDLED_PROFILE
            and owner["state"] == "RecordingOnly" and owner["session"] == session.name.removeprefix("pending-")
            and owner["policy_sha256"] == digest(POLICY.read_bytes())
            and owner["owner_sha256"] == file_hash(Path(__file__)), "NativeSessionIdentity")
    for role in ("cc", "ar"):
        pin = owner["native_launchers"][role]
        require(pin["path"] == str(session / ("native-" + role)), "NativeLauncher")
        pinned_file(pin)
        pinned_file(inv["generators"][role.upper()])
    return inv


def native_context(env, cwd, session, inv):
    package, root, out = fixed_source_package("libsqlite3-sys", "0.28.0", cwd, env, session, inv, SQLITE_MEMBERS)
    require(package["id"] == SQLITE_PACKAGE and env.get("CARGO_MANIFEST_PATH") == str(root / "Cargo.toml")
            and re.fullmatch(r"libsqlite3-sys-[0-9a-f]{16}", out.parent.name), "NativePackage")
    require(env.get("HOST") == TARGET and env.get("TARGET") == TARGET
            and env.get("OPT_LEVEL") == "0" and env.get("DEBUG") == "true"
            and {k: v for k, v in env.items() if k.startswith("CARGO_FEATURE_")} ==
                {"CARGO_FEATURE_" + f.upper().replace("-", "_"): "1" for f in SQLITE_FEATURES}, "NativeConfiguration")
    for member in ("cc/Cargo.toml", "cc/src/lib.rs", "cc/src/tool.rs", "cc/src/tempfile.rs", PROBE_LITERAL):
        path = session / "vendor" / member
        regular(path)
        require(path.resolve() == path and inv["vendor"]["files"].get(member) == file_hash(path), "NativeCcSource")
    require(inv["vendor"]["files"].get(PROBE_LITERAL) == PROBE_DIGEST, "NativeProbeLiteral")
    return {"package_id": SQLITE_PACKAGE, "manifest": str(root), "out_dir": str(out)}


def native_environment(env, session, inv):
    for k, v in env.items():
        forbidden = (re.match(r"^(?:(?:HOST|TARGET)_)?(?:CFLAGS|CXXFLAGS|CPPFLAGS|ARFLAGS)(?:_|$)", k)
            or re.match(r"^(?:CC|CXX|AR)(?:_|-)", k)
            or re.match(r"^(?:HOST|TARGET)_(?:CC|CXX|AR)$", k)
            or k.startswith(("SQLITE", "SQLCIPHER", "OPENSSL", "LIBSQLITE3", "BINDGEN", "SCCACHE", "CCACHE", "CCC_", "CLANG_"))
            or (k.startswith("CARGO_") and k.endswith(("_RUSTFLAGS", "_LINKER", "_RUNNER"))
                and not (k == "CARGO_ENCODED_RUSTFLAGS" and v == ""))
            or k in {"CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "LIBRARY_PATH", "COMPILER_PATH",
                     "GCC_EXEC_PREFIX", "RUSTC_LINKER", "RUSTC_WRAPPER_CUSTOM", "CRATE_CC_NO_DEFAULTS"})
        require(not forbidden, "NativeEnvironmentInjection:" + k)
    require(env.get("RUSTC") == inv["rustc"]["path"] and env.get("RUSTC_WRAPPER") == str(session / "rustc-wrapper")
            and ("ZERO_AR_DATE" not in env or env["ZERO_AR_DATE"] == "1"), "NativeEnvironment")
    require(env.get("CC") == str(session / "native-cc") and env.get("AR") == str(session / "native-ar")
            and all(env.get(k) == inv["environment"].get(k) for k in ENV_KEYS - {"CC", "AR"}), "NativeEnvironment")
    compiler_environment(env, session, inv["sysroot"], probe=False, context={"kind": "LibcBuildVersion"})


def native_snapshot(call, path, leaf, *, absent=False, bound=None):
    path = Path(path)
    require(path.is_absolute() and path.resolve() == path, "NativePathAlias")
    if not path.exists():
        require(absent and not path.is_symlink(), "NativeInputMissing")
        return {"exists": False, "path": str(path)}
    regular(path)
    before = path.stat()
    require(before.st_nlink == 1, "NativeFileAlias")
    snapshot = call / leaf
    with open(path, "rb") as source, open(snapshot, "xb") as dest:
        opened = os.fstat(source.fileno())
        require((opened.st_dev, opened.st_ino) == (before.st_dev, before.st_ino), "NativeFileChanged")
        size = 0
        while True:
            block = source.read(min(65536, bound + 1 - size) if bound is not None else 65536)
            if not block:
                break
            dest.write(block); size += len(block)
            require(bound is None or size <= bound, "NativeInputExtent")
        after = os.fstat(source.fileno())
    now = path.stat()
    identity = lambda st: [st.st_dev, st.st_ino, stat.S_IFMT(st.st_mode), st.st_size, st.st_mtime_ns]
    require(identity(before) == identity(opened) == identity(after) == identity(now)
            and size == before.st_size and file_hash(path) == file_hash(snapshot), "NativeFileChanged")
    return {"exists": True, "path": str(path), "identity": identity(now), "length": size,
            "sha256": file_hash(snapshot), "snapshot": leaf}


def native_state_key(state):
    return {k: v for k, v in state.items() if k != "snapshot"}


def native_state_check(call, state, *, live=False, retire=False):
    path = Path(state["path"])
    if not state["exists"]:
        if live:
            require(not path.exists() and not path.is_symlink(), "NativeVersionChanged")
        return "Absent"
    snapshot = call / state["snapshot"]
    regular(snapshot)
    require(snapshot.parent == call and snapshot.resolve() == snapshot and snapshot.stat().st_nlink == 1
            and snapshot.stat().st_size == state["length"] and file_hash(snapshot) == state["sha256"], "NativeSnapshotChanged")
    if live:
        if retire and not path.exists() and not path.is_symlink():
            return "RetiredAfterCcReturn"
        regular(path);st = path.stat()
        require(path.resolve() == path and st.st_nlink == 1
                and [st.st_dev, st.st_ino, stat.S_IFMT(st.st_mode), st.st_size, st.st_mtime_ns] == state["identity"]
                and file_hash(path) == state["sha256"], "NativeVersionChanged")
    return "Present"


def native_prior(session, context):
    rows = []
    for call in (session / "native-invocations").iterdir():
        if (call / "receipt.json").is_file():
            r = strict_json((call / "receipt.json").read_bytes())
            if r.get("context") == context:
                rows.append((call, r))
    return rows


def native_classify(role, args, context, env, session, inv, *, inspector=False, history=None):
    root, out = Path(context["manifest"]), Path(context["out_dir"])
    rows = native_prior(session, context) if history is None else history
    if inspector:
        require(role == "ar" and ((len(args) == 2 and args[0] == "t")
                or (len(args) == 3 and args[0] == "p" and re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.-]*\.o", args[2])))
                and args[1] == str(out / "libsqlite3.a"), "NativeInspectorForm")
        return {"class": "ArchiveInspector", "archive": args[1], "member": args[2] if len(args) == 3 else None}
    if role == "cc" and args and args[0] == "-E":
        retry = len(args) == 3 and args[1] == "--"
        require(len(args) == (3 if retry else 2), "NativeProbeForm")
        path = Path(args[-1]);match = re.fullmatch(r"(0|[1-9][0-9]{0,19})detect_compiler_family\.c", path.name)
        require(match and int(match[1]) <= 2**64-1 and path.parent == out and str(path) == args[-1], "NativeProbePath")
        prior = [(c, r) for c, r in rows if r.get("operation", {}).get("source") == str(path)]
        require((not retry and not prior) or (retry and len(prior) == 1), "NativeProbePredecessor")
        predecessor = None
        if retry:
            c, r = prior[0]
            require(r["protocol_state"] == "Completed" and r["operation"]["class"] == "CompilerFamilyFileProbe"
                    and not r["operation"]["retry"]
                    and any(b"-Wslash-u-filename" in (c / (stream + ".raw")).read_bytes() for stream in ("stdout", "stderr")),
                    "NativeProbePredecessor")
            native_state_check(c, r["input_post"], live=history is None)
            predecessor = c.name
        return {"class": "CompilerFamilyFileProbe", "source": str(path), "retry": retry, "predecessor": predecessor}
    if role == "cc":
        source = root / "sqlite3/sqlite3.c"
        require(args.count("-c") == 1 and args.count("-o") == 1, "NativeCompileForm")
        src_at, out_at = args.index("-c") + 1, args.index("-o") + 1
        require(src_at < len(args) and out_at < len(args), "NativeCompileForm")
        require(args[src_at] in ("sqlite3/sqlite3.c", str(source)), "NativeCompileSource")
        output = Path(args[out_at])
        require(output.parent == out and output.resolve() == output and re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.-]*\.o", output.name), "NativeCompileOutput")
        ignored = {src_at-1, src_at, out_at-1, out_at};i = 0;defines=[]
        flags = {"-O0", "-ffunction-sections", "-fdata-sections", "-fPIC", "-g", "-gdwarf-2", "-gdwarf-4",
                 "-fno-omit-frame-pointer", "-m64", "-w", "-Wall", "-Wextra", "-std=c99", "-std=c11"}
        while i < len(args):
            if i in ignored:
                i += 1;continue
            arg=args[i]
            if arg.startswith("-D"):
                require(arg[2:] in SQLITE_DEFINES and arg[2:] not in defines, "NativeCompileMacro")
                defines.append(arg[2:])
            elif arg in ("-arch", "-isysroot", "--target", "-target"):
                require(i+1 < len(args) and i+1 not in ignored, "NativeCompileFlags")
                expected={"-arch":"x86_64", "-isysroot":env.get("SDKROOT"), "--target":TARGET, "-target":TARGET}[arg]
                require(expected is not None and args[i+1] == expected, "NativeCompileFlags");i+=1
            elif arg.startswith("-mmacosx-version-min="):
                require(env.get("MACOSX_DEPLOYMENT_TARGET") is not None and arg.split("=",1)[1] == env["MACOSX_DEPLOYMENT_TARGET"], "NativeCompileFlags")
            else:
                require(arg in flags, "NativeCompileFlags:" + arg)
            i+=1
        require("SQLITE_CORE" in defines and not any(r.get("operation",{}).get("class")=="SqliteObjectCompile" for _,r in rows), "NativeObjectProducer")
        return {"class":"SqliteObjectCompile", "source":str(source), "output":str(output)}
    require(role == "ar" and len(args) in (2,3) and args[0] in {"cqD", "cq", "sD", "s"}
            and args[1] == str(out / "libsqlite3.a"), "NativeArchiveForm")
    mode=args[0];mutators=[(c,r) for c,r in rows if r.get("operation",{}).get("class") in {"ArchiveAppend","ArchiveIndex"}]
    objects=[(c,r) for c,r in rows if r.get("operation",{}).get("class")=="SqliteObjectCompile"
             and r.get("protocol_state")=="Completed" and r.get("tool_result")==0]
    require(len(objects)==1, "NativeObjectProducer")
    oc, obj=objects[0];native_state_check(oc,obj["output_post"],live=history is None)
    require(len(args)==(3 if mode in ("cqD","cq") else 2)
            and (len(args)==2 or args[2]==obj["operation"]["output"]), "NativeArchiveObject")
    predecessor=None
    if not mutators:
        require(mode=="cqD", "NativeArchiveTransition")
    else:
        referenced={r["operation"]["predecessor"] for _,r in mutators}
        tails=[(c,r) for c,r in mutators if c.name not in referenced]
        require(len(tails)==1, "NativeArchiveTransition")
        pc, previous=tails[0];predecessor=pc.name
        require(previous["protocol_state"]=="Completed", "NativeArchiveProtocol")
        pm=previous["operation"]["mode"];code=previous["tool_result"]
        require((pm=="cqD" and isinstance(code,int) and code>0 and mode=="cq" and len(mutators)==1)
                or (pm=="cqD" and code==0 and mode=="sD" and len(mutators)==1)
                or (pm=="cq" and code==0 and mode=="s" and len(mutators)==2), "NativeArchiveTransition")
        native_state_check(pc,previous["archive_post"],live=history is None)
    if mode in ("cqD","cq","s"):
        require(env.get("ZERO_AR_DATE")=="1", "NativeArchiveEnvironment")
    return {"class":"ArchiveAppend" if mode in ("cqD","cq") else "ArchiveIndex", "mode":mode,
            "archive":args[1], "object":obj["operation"]["output"], "object_producer":oc.name, "predecessor":predecessor}


def native_streamed(argv, cwd, env, call, pass_fds, bounds):
    # Native stdin is always closed. Generic Rust streaming semantics are unchanged.
    faults=[];code=None
    try:
        process=subprocess.Popen(argv,cwd=cwd,env=env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE,close_fds=True,pass_fds=pass_fds)
    except OSError as error:
        return None,["NativeSpawn:"+type(error).__name__]
    def pump(source, name):
        dest=None;count=0
        try:
            try:dest=open(call/(name+".raw"),"xb")
            except OSError as error:faults.append("NativeCapture:"+name+":"+type(error).__name__)
            while True:
                block=source.read(65536)
                if not block:break
                count+=len(block)
                limit=bounds.get(name)
                if limit is not None and count>limit:
                    if "NativeCaptureBound:"+name not in faults:faults.append("NativeCaptureBound:"+name)
                    if dest:
                        allowed=max(0,limit+1-(count-len(block)))
                        try:dest.write(block[:allowed])
                        except OSError as error:faults.append("NativeCapture:"+name+":"+type(error).__name__)
                    continue
                if dest:
                    try:dest.write(block)
                    except OSError as error:
                        faults.append("NativeCapture:"+name+":"+type(error).__name__);dest.close();dest=None
        except BaseException as error:faults.append("NativeCapture:"+name+":"+type(error).__name__)
        finally:
            if dest:
                try:dest.close()
                except OSError as error:faults.append("NativeCapture:"+name+":"+type(error).__name__)
            source.close()
    threads=[threading.Thread(target=pump,args=(process.stdout,"stdout")),threading.Thread(target=pump,args=(process.stderr,"stderr"))]
    for t in threads:t.start()
    try:code=process.wait()
    except OSError as error:faults.append("NativeWait:"+type(error).__name__)
    for t in threads:t.join()
    return code, faults


def native_jobserver_identity(fds):
    """Self-only Darwin pipe endpoint observation; never read or write a token.

    SDK 26.5 pipe_fdinfo is 184 bytes: pipeinfo at 24, handle at 160,
    peerhandle at 168. This fixed recording ABI is not provider qualification.
    """
    import ctypes
    require(sys.platform == "darwin" and sys.byteorder == "little"
            and ctypes.sizeof(ctypes.c_void_p) == 8 and ctypes.sizeof(ctypes.c_int) == 4
            and ctypes.sizeof(ctypes.c_uint64) == 8, "NativeJobserverIdentityABI")
    require(len(fds) == 2 and all(isinstance(fd, int) and 3 <= fd <= 2**31-1 for fd in fds)
            and fds[0] != fds[1], "InvalidJobserverDescriptors")
    def endpoint(fd, access):
        try:
            st = os.fstat(fd); flags = fcntl.fcntl(fd, fcntl.F_GETFL)
        except (OSError, OverflowError, ValueError):
            raise Refusal("InvalidJobserverDescriptors") from None
        require(stat.S_ISFIFO(st.st_mode) and flags & os.O_ACCMODE == access,
                "InvalidJobserverDescriptors")
        return {"fd": fd, "device": st.st_dev, "inode": st.st_ino,
                "file_type": stat.S_IFMT(st.st_mode), "flags": flags, "access": access}
    roles = (os.O_RDONLY, os.O_WRONLY)
    before = [endpoint(fd, access) for fd, access in zip(fds, roles)]
    buffer_type = ctypes.c_uint64 * 23
    require(ctypes.sizeof(buffer_type) == 184 and ctypes.alignment(buffer_type) == 8,
            "NativeJobserverIdentityABI")
    try:
        library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        query = library.proc_pidfdinfo
        query.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
        query.restype = ctypes.c_int
        pid = os.getpid(); observations = []
        for index, (fd, access) in enumerate(zip(fds, roles)):
            buffer = buffer_type()
            count = query(pid, fd, 6, buffer, 184)
            require(count == 184, "NativeJobserverIdentityQuery")
            after = endpoint(fd, access)
            require(after == before[index], "NativeJobserverDescriptorChanged")
            raw = bytes(buffer)
            handle = int.from_bytes(raw[160:168], "little")
            peer = int.from_bytes(raw[168:176], "little")
            require(handle == before[index]["inode"] and handle != 0 and peer != 0,
                    "NativeJobserverIdentityHandle")
            observations.append(dict(before[index], returned_bytes=count, handle=handle, peer=peer))
        require([endpoint(fd, access) for fd, access in zip(fds, roles)] == before,
                "NativeJobserverDescriptorChanged")
    except (OSError, AttributeError, ctypes.ArgumentError):
        raise Refusal("NativeJobserverIdentityQuery") from None
    require(observations[0]["handle"] == observations[1]["peer"]
            and observations[1]["handle"] == observations[0]["peer"]
            and observations[0]["handle"] != observations[1]["handle"], "NativeJobserverPair")
    return {"state": "RecordingOnly", "platform": "darwin", "library": "/usr/lib/libproc.dylib",
            "symbol": "proc_pidfdinfo", "pid": pid, "flavor": 6, "buffer_bytes": 184,
            "pipe_info_offset": 24, "handle_offset": 160, "peer_offset": 168,
            "endpoints": observations}


def native_operation(session, policy, owner, role, args, cwd, env, *, inspector=False, bounds=None):
    call=session/"native-invocations"/uuid.uuid4().hex;call.mkdir(mode=0o700)
    request={"schema":NATIVE_SCHEMA,"state":"RecordingOnly","role":role,"args_hex":[os.fsencode(a).hex() for a in args],
             "cwd_hex":os.fsencode(str(cwd)).hex(),"environment_hex":{os.fsencode(k).hex():os.fsencode(v).hex() for k,v in env.items()},
             "owner_issued_inspector":inspector}
    atomic_json(call/"request.json",request)
    receipt=dict(request,operation_id=call.name,protocol_state="ProtocolRefused",tool_result=None,failures=[])
    try:
        inv=native_control(session,policy,owner);require(role in ("cc","ar"),"NativeRole")
        context=native_context(env,cwd,session,inv);receipt["context"]=context
        native_environment(env,session,inv)
        operation=native_classify(role,args,context,env,session,inv,inspector=inspector);receipt["operation"]=operation
        tool=pinned_file(inv["generators"][role.upper()]);receipt["tool_sha256"]=inv["generators"][role.upper()]["sha256"]
        receipt["argv_hex"]=[os.fsencode(v).hex() for v in [tool,*args]]
        kind=operation["class"]
        if kind in {"CompilerFamilyFileProbe","SqliteObjectCompile"}:
            bound=206 if kind=="CompilerFamilyFileProbe" else None
            receipt["input_pre"]=native_snapshot(call,operation["source"],"input-pre.raw",bound=bound)
            if bound:
                require(receipt["input_pre"]["length"]==206 and receipt["input_pre"]["sha256"]==PROBE_DIGEST
                        and (call/"input-pre.raw").read_bytes()==(session/"vendor"/PROBE_LITERAL).read_bytes(),"NativeProbeLiteral")
            if kind=="SqliteObjectCompile":
                require(receipt["input_pre"]["sha256"] == inv["vendor"]["files"]["libsqlite3-sys/sqlite3/sqlite3.c"], "NativeCompileSource")
                receipt["output_pre"]=native_snapshot(call,operation["output"],"output-pre.raw",absent=True)
                require(not receipt["output_pre"]["exists"],"NativeOutputAlreadyExists")
        if "archive" in operation:
            receipt["archive_pre"]=native_snapshot(call,operation["archive"],"archive-pre.raw",absent=kind=="ArchiveAppend")
            if kind=="ArchiveAppend" and operation["predecessor"] is None:
                require(not receipt["archive_pre"]["exists"],"NativeArchiveInitial")
            if kind in {"ArchiveAppend", "ArchiveIndex"} and operation["predecessor"] is not None:
                previous = strict_json((session/"native-invocations"/operation["predecessor"]/"receipt.json").read_bytes())
                require(native_state_key(receipt["archive_pre"]) == native_state_key(previous["archive_post"]), "NativeArchivePredecessor")
        fds=() if inspector else inherited_jobserver_fds(env)
        receipt["jobserver_identity"] = native_jobserver_identity(fds) if fds else None
        code,faults=native_streamed([tool,*args],cwd,env,call,fds,bounds or {})
        receipt["tool_result"]=code;receipt["failures"].extend(faults)
        if "input_pre" in receipt:
            receipt["input_post"]=native_snapshot(call,operation["source"],"input-post.raw",bound=bound)
            require(native_state_key(receipt["input_pre"])==native_state_key(receipt["input_post"]),"NativeInputChanged")
        if kind=="SqliteObjectCompile":
            receipt["output_post"]=native_snapshot(call,operation["output"],"output-post.raw",absent=code!=0)
        if "archive" in operation:
            receipt["archive_post"]=native_snapshot(call,operation["archive"],"archive-post.raw",absent=code!=0)
            if inspector:require(native_state_key(receipt["archive_pre"])==native_state_key(receipt["archive_post"]),"NativeInspectorDrift")
        native_control(session,policy,owner)
        require(not receipt["failures"],"NativeProtocolCapture")
        receipt["protocol_state"]="Completed"
    except (Refusal,OSError,ValueError,KeyError,TypeError) as error:
        receipt["failures"].append(str(error))
    for stream in ("stdout","stderr"):
        path=call/(stream+".raw")
        if path.is_file():receipt[stream+"_sha256"]=file_hash(path)
    # cc consumes actual streams/status, including its supported warning retry.
    # A failed delivery is a sticky protocol failure, never an observed tool failure.
    if not inspector:
        for stream,fd in (("stdout",1),("stderr",2)):
            path=call/(stream+".raw")
            try:
                if path.is_file():
                    with open(path,"rb") as source:
                        for block in iter(lambda:source.read(65536),b""):
                            view=memoryview(block)
                            while view:
                                written=os.write(fd,view)
                                require(written>0,"NativeForwardZero:"+stream)
                                view=view[written:]
            except (OSError,Refusal) as error:
                receipt["protocol_state"]="ProtocolRefused"
                receipt["failures"].append("NativeForward:"+stream+":"+str(error))
    atomic_json(call/"receipt.json",receipt)
    return call,receipt


FOREIGN_LANE = "ForeignBuilderRecordingOnly"
FOREIGN_PACKAGES = {"ring": ("0.17.14", RING_FEATURES, "ring_core_0_17_14_"),
                    "psm": ("0.1.30", (), None)}
FOREIGN_CC_MEMBERS = ("cc/Cargo.toml", "cc/src/lib.rs", "cc/src/tool.rs",
                      "cc/src/tempfile.rs", "cc/src/command_helpers.rs", PROBE_LITERAL)


# Finite source-derived CompileOnly map: bundled2 raw30, cc1.2.59 and Rust1.95.
# This is not a general DefaultHasher model or a compiler/provider fallback.
FOREIGN_MAP_RUSTC_SHA256 = "fcf2ce6f5b55d90d29867767262ad179ed761a8b54d24b5c181c535fa05ced19"
FOREIGN_MAP_CC_SHA256 = "7fe2e79744d09922c1c8eef63e90473880c9e33a6375e54ebee8fe246555b09e"
FOREIGN_OBJECT_PREFIXES = {
    ("/crypto", "c"): "a4019cc0736b0423", ("/crypto/curve25519", "c"): "25ac62e5b3c53843",
    ("/crypto/fipsmodule/aes", "c"): "0bbbd18bda93c05b", ("/crypto/fipsmodule/bn", "c"): "00c879ee3285a50d",
    ("/crypto/fipsmodule/ec", "c"): "a0330e891e733f4e", ("/crypto/limbs", "c"): "aaa1ba3e455ee2e1",
    ("/crypto/poly1305", "c"): "d5a9841f3dc6e253", ("/pregenerated", "S"): "c322a0bcc369f531",
    ("/third_party/fiat/asm", "S"): "e165cd818145c705", ("src/arch", "s"): "4f9a91766097c4c5"}
FOREIGN_COMPILE_SOURCES = {
    "ring": ("crypto/cpu_intel.c", "crypto/crypto.c", "crypto/curve25519/curve25519.c",
        "crypto/curve25519/curve25519_64_adx.c", "crypto/fipsmodule/aes/aes_nohw.c",
        "crypto/fipsmodule/bn/montgomery.c", "crypto/fipsmodule/bn/montgomery_inv.c",
        "crypto/fipsmodule/ec/ecp_nistz.c", "crypto/fipsmodule/ec/gfp_p256.c", "crypto/fipsmodule/ec/gfp_p384.c",
        "crypto/fipsmodule/ec/p256-nistz.c", "crypto/fipsmodule/ec/p256.c", "crypto/limbs/limbs.c", "crypto/mem.c",
        "crypto/poly1305/poly1305.c", "pregenerated/aes-gcm-avx2-x86_64-macosx.S",
        "pregenerated/aesni-gcm-x86_64-macosx.S", "pregenerated/aesni-x86_64-macosx.S",
        "pregenerated/chacha-x86_64-macosx.S", "pregenerated/chacha20_poly1305_x86_64-macosx.S",
        "pregenerated/ghash-x86_64-macosx.S", "pregenerated/p256-x86_64-asm-macosx.S",
        "pregenerated/sha256-x86_64-macosx.S", "pregenerated/sha512-x86_64-macosx.S",
        "pregenerated/vpaes-x86_64-macosx.S", "pregenerated/x86_64-mont-macosx.S",
        "pregenerated/x86_64-mont5-macosx.S", "third_party/fiat/asm/fiat_curve25519_adx_mul.S",
        "third_party/fiat/asm/fiat_curve25519_adx_square.S"),
    "psm": ("src/arch/x86_64.s",)}
FOREIGN_COMPILE_CC_MEMBERS = tuple("cc/src/target/" + n + ".rs" for n in ("apple", "llvm", "parser", "generated"))
FOREIGN_COMPILE_COMMON = ("-O0", "-ffunction-sections", "-fdata-sections", "-fPIC", "-g", "-gdwarf-2",
    "-fno-omit-frame-pointer", "-m64", "--target=x86_64-apple-macosx", "-mmacosx-version-min=26.5")
FOREIGN_COMPILE_RING = ("-Wall", "-Wextra", "-fvisibility=hidden", "-std=c1x", "-Wall", "-Wbad-function-cast",
    "-Wcast-align", "-Wcast-qual", "-Wconversion", "-Wmissing-field-initializers", "-Wmissing-include-dirs",
    "-Wnested-externs", "-Wredundant-decls", "-Wshadow", "-Wsign-compare", "-Wsign-conversion",
    "-Wstrict-prototypes", "-Wundef", "-Wuninitialized", "-gfull", "-DNDEBUG")
FOREIGN_COMPILE_PSM = ("-Wall", "-Wextra", "-xassembler-with-cpp", "-DCFG_TARGET_OS_macos",
    "-DCFG_TARGET_ARCH_x86_64", "-DCFG_TARGET_ENV_")


# Closed lz4/zstd context and E-only dispatch from bundled2 raw44; no C/AR authority.
FOREIGN_E_ONLY_PACKAGES = {'lz4-sys': ('1.11.1+lz4-1.10.0', ('1', '11', '1', ''), (), 'lz4'),
 'zstd-sys': ('2.0.16+zstd.1.5.7', ('2', '0', '16', ''), ('legacy', 'std', 'zdict_builder'), 'zstd')}
FOREIGN_E_ONLY_INPUTS = {'lz4-sys': ('lz4-sys/liblz4/lib/lz4.c',
             'lz4-sys/liblz4/lib/lz4.h',
             'lz4-sys/liblz4/lib/lz4file.h',
             'lz4-sys/liblz4/lib/lz4frame.c',
             'lz4-sys/liblz4/lib/lz4frame.h',
             'lz4-sys/liblz4/lib/lz4frame_static.h',
             'lz4-sys/liblz4/lib/lz4hc.c',
             'lz4-sys/liblz4/lib/lz4hc.h',
             'lz4-sys/liblz4/lib/xxhash.c',
             'lz4-sys/liblz4/lib/xxhash.h'),
 'zstd-sys': ('zstd-sys/zstd/lib/common/allocations.h',
              'zstd-sys/zstd/lib/common/bits.h',
              'zstd-sys/zstd/lib/common/bitstream.h',
              'zstd-sys/zstd/lib/common/compiler.h',
              'zstd-sys/zstd/lib/common/cpu.h',
              'zstd-sys/zstd/lib/common/debug.c',
              'zstd-sys/zstd/lib/common/debug.h',
              'zstd-sys/zstd/lib/common/entropy_common.c',
              'zstd-sys/zstd/lib/common/error_private.c',
              'zstd-sys/zstd/lib/common/error_private.h',
              'zstd-sys/zstd/lib/common/fse.h',
              'zstd-sys/zstd/lib/common/fse_decompress.c',
              'zstd-sys/zstd/lib/common/huf.h',
              'zstd-sys/zstd/lib/common/mem.h',
              'zstd-sys/zstd/lib/common/pool.c',
              'zstd-sys/zstd/lib/common/pool.h',
              'zstd-sys/zstd/lib/common/portability_macros.h',
              'zstd-sys/zstd/lib/common/threading.c',
              'zstd-sys/zstd/lib/common/threading.h',
              'zstd-sys/zstd/lib/common/xxhash.h',
              'zstd-sys/zstd/lib/common/zstd_common.c',
              'zstd-sys/zstd/lib/common/zstd_deps.h',
              'zstd-sys/zstd/lib/common/zstd_internal.h',
              'zstd-sys/zstd/lib/common/zstd_trace.h',
              'zstd-sys/zstd/lib/compress/clevels.h',
              'zstd-sys/zstd/lib/compress/fse_compress.c',
              'zstd-sys/zstd/lib/compress/hist.c',
              'zstd-sys/zstd/lib/compress/hist.h',
              'zstd-sys/zstd/lib/compress/huf_compress.c',
              'zstd-sys/zstd/lib/compress/zstd_compress.c',
              'zstd-sys/zstd/lib/compress/zstd_compress_internal.h',
              'zstd-sys/zstd/lib/compress/zstd_compress_literals.c',
              'zstd-sys/zstd/lib/compress/zstd_compress_literals.h',
              'zstd-sys/zstd/lib/compress/zstd_compress_sequences.c',
              'zstd-sys/zstd/lib/compress/zstd_compress_sequences.h',
              'zstd-sys/zstd/lib/compress/zstd_compress_superblock.c',
              'zstd-sys/zstd/lib/compress/zstd_compress_superblock.h',
              'zstd-sys/zstd/lib/compress/zstd_cwksp.h',
              'zstd-sys/zstd/lib/compress/zstd_double_fast.c',
              'zstd-sys/zstd/lib/compress/zstd_double_fast.h',
              'zstd-sys/zstd/lib/compress/zstd_fast.c',
              'zstd-sys/zstd/lib/compress/zstd_fast.h',
              'zstd-sys/zstd/lib/compress/zstd_lazy.c',
              'zstd-sys/zstd/lib/compress/zstd_lazy.h',
              'zstd-sys/zstd/lib/compress/zstd_ldm.c',
              'zstd-sys/zstd/lib/compress/zstd_ldm.h',
              'zstd-sys/zstd/lib/compress/zstd_ldm_geartab.h',
              'zstd-sys/zstd/lib/compress/zstd_opt.c',
              'zstd-sys/zstd/lib/compress/zstd_opt.h',
              'zstd-sys/zstd/lib/compress/zstd_preSplit.c',
              'zstd-sys/zstd/lib/compress/zstd_preSplit.h',
              'zstd-sys/zstd/lib/compress/zstdmt_compress.c',
              'zstd-sys/zstd/lib/compress/zstdmt_compress.h',
              'zstd-sys/zstd/lib/decompress/huf_decompress.c',
              'zstd-sys/zstd/lib/decompress/huf_decompress_amd64.S',
              'zstd-sys/zstd/lib/decompress/zstd_ddict.c',
              'zstd-sys/zstd/lib/decompress/zstd_ddict.h',
              'zstd-sys/zstd/lib/decompress/zstd_decompress.c',
              'zstd-sys/zstd/lib/decompress/zstd_decompress_block.c',
              'zstd-sys/zstd/lib/decompress/zstd_decompress_block.h',
              'zstd-sys/zstd/lib/decompress/zstd_decompress_internal.h',
              'zstd-sys/zstd/lib/deprecated/zbuff.h',
              'zstd-sys/zstd/lib/dictBuilder/cover.c',
              'zstd-sys/zstd/lib/dictBuilder/cover.h',
              'zstd-sys/zstd/lib/dictBuilder/divsufsort.c',
              'zstd-sys/zstd/lib/dictBuilder/divsufsort.h',
              'zstd-sys/zstd/lib/dictBuilder/fastcover.c',
              'zstd-sys/zstd/lib/dictBuilder/zdict.c',
              'zstd-sys/zstd/lib/legacy/zstd_legacy.h',
              'zstd-sys/zstd/lib/legacy/zstd_v01.c',
              'zstd-sys/zstd/lib/legacy/zstd_v01.h',
              'zstd-sys/zstd/lib/legacy/zstd_v02.c',
              'zstd-sys/zstd/lib/legacy/zstd_v02.h',
              'zstd-sys/zstd/lib/legacy/zstd_v03.c',
              'zstd-sys/zstd/lib/legacy/zstd_v03.h',
              'zstd-sys/zstd/lib/legacy/zstd_v04.c',
              'zstd-sys/zstd/lib/legacy/zstd_v04.h',
              'zstd-sys/zstd/lib/legacy/zstd_v05.c',
              'zstd-sys/zstd/lib/legacy/zstd_v05.h',
              'zstd-sys/zstd/lib/legacy/zstd_v06.c',
              'zstd-sys/zstd/lib/legacy/zstd_v06.h',
              'zstd-sys/zstd/lib/legacy/zstd_v07.c',
              'zstd-sys/zstd/lib/legacy/zstd_v07.h',
              'zstd-sys/zstd/lib/zdict.h',
              'zstd-sys/zstd/lib/zstd.h',
              'zstd-sys/zstd/lib/zstd_errors.h')}
FOREIGN_E_ONLY_SOURCE_PINS = {'cc/Cargo.toml': '9d24fea2d14fe0d763e2017becf9be4fbb7b88c6d17c4ab28d8d7ff23f452f29',
 'cc/src/command_helpers.rs': '7fe2e79744d09922c1c8eef63e90473880c9e33a6375e54ebee8fe246555b09e',
 'cc/src/detect_compiler_family.c': '97ca4b021495611e828becea6187add37414186a16dfedd26c2947cbce6e8b2f',
 'cc/src/lib.rs': '834b78638792fa4c8cdfc0ecc9cf2031bfcc223a85bfa0ca52e55011e64059ff',
 'cc/src/target/apple.rs': 'da9411b2c4db419e0fa39f765ee53b8665b3837fb39827679a53238734a4a1c1',
 'cc/src/target/generated.rs': '99463a5ffa411bafe7f91e8d7ee0f8f4d29ca28c59a42175e7955a3249301ce0',
 'cc/src/target/llvm.rs': '190fe8d2b204cd4a6e68f2a1aada17ecd6390799564ed25792fcc08ab34710ba',
 'cc/src/target/parser.rs': '56ebea1e462a35c54e53b50a130284f0fb6fd43ad84ddafda2cdb01272e36d60',
 'cc/src/tempfile.rs': '3d9a4bd894862a345aa230a61ec266f0c68f4ef9713d1d9c727482e61f1ea7c3',
 'cc/src/tool.rs': '1d279a6f0738f9164ca794c85bd55951aa95e2adc788e75a6bb8133393d6cce8',
 'lz4-sys/Cargo.toml': '1acf81d8bef849aa6c9b814dba8063465f945ac600d32f31d56685ea90678be6',
 'lz4-sys/build.rs': '1592aae45eff1403059337a9e81eedd5e7d48114b4e296936db011441cf90382',
 'lz4-sys/liblz4/lib/lz4.c': '9396f7de527bc8435de9c7569fb7998e56545a84b4f3c2d808c0235c01774539',
 'lz4-sys/liblz4/lib/lz4.h': '26b82efc53d1570f3b54eef02e9c4764c1ad374ff03cac04e2ced5ea4d4c552f',
 'lz4-sys/liblz4/lib/lz4file.h': '400ed90bc74324abcf338d37235dd81aa431fd815c22db4b369b88c12b213115',
 'lz4-sys/liblz4/lib/lz4frame.c': '44f421bea199c7f11da263c717f063228cd2c8c05a8384d327b49cc81ccfbac4',
 'lz4-sys/liblz4/lib/lz4frame.h': 'b845db4b7ee1bfa64b8f641a94f62e7f636a8b5d673cc61766413782283aaad1',
 'lz4-sys/liblz4/lib/lz4frame_static.h': '31ab72a6e97e4fa0bedd4420d24a3f1f8024cbfa1d8ffad88c87cbb4e04f769d',
 'lz4-sys/liblz4/lib/lz4hc.c': '126cafafdb91767e6e55238298a910903851b35b2cee27ce80ae2280469ee232',
 'lz4-sys/liblz4/lib/lz4hc.h': 'e43824e8a9ba16f54100c4ccbccfa5782a858ca9ab83c48aac303fea3e76e21e',
 'lz4-sys/liblz4/lib/xxhash.c': 'b667033dc735fb5ea5648e0a61a2e065e5ef5bbda53669730063bd856c643c48',
 'lz4-sys/liblz4/lib/xxhash.h': 'aefdd236f35130495c18764cabed3f7b216906855fc5e6a9025cd2040bc84444',
 'zstd-sys/Cargo.toml': '3f400eeb43f176bc78c2ec01d56125c1ccfea814f7c7ff81199573bdbd8d1f1a',
 'zstd-sys/build.rs': 'd92ade96b5f7c04496b1e928c564fcf52bb7d59439f56262407da77c5628458d',
 'zstd-sys/zstd/lib/common/allocations.h': '6a718e8edaca112abdc0bdfa7de67edefaa44c8d9e514b79672ab58ab3a9118a',
 'zstd-sys/zstd/lib/common/bits.h': '5bb7693a35d7ce03f8715c0b1e237062285e052250fdd34f72f38e7e92365090',
 'zstd-sys/zstd/lib/common/bitstream.h': 'a9461685c9add6f609078acc950a08db4c02e6e6a849d550f45182ba2ae38556',
 'zstd-sys/zstd/lib/common/compiler.h': 'd19829705d8039437ffba873df2b58d20d7b876d58da479121d2e515df55cbf7',
 'zstd-sys/zstd/lib/common/cpu.h': '23d520b536a6f23114e66549d03a967b1f24b15560c5b1ebd47ee392ccee4a1c',
 'zstd-sys/zstd/lib/common/debug.c': '7ab1ea104acf3243fc1cd4e6d2514046f1dfc1101c7c676145c8da080e8c24ca',
 'zstd-sys/zstd/lib/common/debug.h': '8260cd5087b8309679c61b772b535688993c61a675a30481b8ae55b45740ba15',
 'zstd-sys/zstd/lib/common/entropy_common.c': '7dfce29c6bc807645b1f0c373dad94bbb49603b175a84e72d1739bf9df65feb8',
 'zstd-sys/zstd/lib/common/error_private.c': '3f5873b2626ca1cdf554b7ec7b04ea0ef296fd0f6882d7fad42448b7e2dbb41c',
 'zstd-sys/zstd/lib/common/error_private.h': 'ae077bb433eb150ee247410a0357f910b9c07be839c6a9abe0b2e6b18ceb716d',
 'zstd-sys/zstd/lib/common/fse.h': '5235ce1e512bf80204d013b1f4cecd776fdaa9effddb48308bce08f6d0b84499',
 'zstd-sys/zstd/lib/common/fse_decompress.c': '84eba7030a036b36363a5108e760428ce549f1a833bec6464dc755f6ed51a6fb',
 'zstd-sys/zstd/lib/common/huf.h': '9a83d899c8c9bf03389d482562090ec2443bc0f87f27864a2b891b180333f2d4',
 'zstd-sys/zstd/lib/common/mem.h': '4dd5fe76fa30a020cd4b3af3134ae143920300a1fe54baac1a5da73297e66783',
 'zstd-sys/zstd/lib/common/pool.c': '9431e26cf7ca46ffb878f17df31198046664d592ae208dfbe3715e82b87af79d',
 'zstd-sys/zstd/lib/common/pool.h': 'bca19a2408e85f31bad3a13f205de92ac26ecda0c5af96fdd7f213ce03d9fc78',
 'zstd-sys/zstd/lib/common/portability_macros.h': '75e2b43968b70decc6bd17876833be04894035594ddc1e745cd6b70be8265637',
 'zstd-sys/zstd/lib/common/threading.c': '50448323b46a8e1bde042ee88ec44fcfc646f3d4aeba8a06be54d26a710ed78d',
 'zstd-sys/zstd/lib/common/threading.h': '9ca63027ea64046acacccbe056a414d37ea7ea428ce299fe08abe53c204c5c65',
 'zstd-sys/zstd/lib/common/xxhash.h': '8cb837b21a8fe9a6b9dbcd0961ab16e733bfcbfa9e003f3a496ce07ae80aa8ee',
 'zstd-sys/zstd/lib/common/zstd_common.c': 'f49eb8023a90d3de925373bd9ee0d36ed46b26c529840b64220397446cd73795',
 'zstd-sys/zstd/lib/common/zstd_deps.h': '9c77ea7d0afa8d5c868a7839e178a446a82d88eb75fd56f2fd7d9d96c5ab7c11',
 'zstd-sys/zstd/lib/common/zstd_internal.h': 'ad3d95ce2b81a8c5c6b00cfe6323d8e80cacb690d3b5883725cd0d939f4f576f',
 'zstd-sys/zstd/lib/common/zstd_trace.h': '17f6daa4c7e97055cd1ed7bcf4d9241a4e79b465f083c5d732fda1df2f0be11e',
 'zstd-sys/zstd/lib/compress/clevels.h': 'd764daa89b7a636d26fe96ff3536c5075e9867fb5dddbf8c9d57023a8ff4a155',
 'zstd-sys/zstd/lib/compress/fse_compress.c': '5070807a489757b87c1e1f50332b099e8ffe22971228218111eef64d3321ae82',
 'zstd-sys/zstd/lib/compress/hist.c': '1a613082642b9cabdeecd3b5929f7230cbb2f8344a28f5010f3aec12cb647ab8',
 'zstd-sys/zstd/lib/compress/hist.h': '9e3363e69d5fa35c1f6e8d2970c6c2453c06fd8a8ab3cc516a14c77d07cdea35',
 'zstd-sys/zstd/lib/compress/huf_compress.c': '0be78471f5175e49bbacb898d8b107bb8f7e13adc84780e0625be5861f016086',
 'zstd-sys/zstd/lib/compress/zstd_compress.c': '10c315ae609d49b2fc431521aa95908086e69fd917991c5ed9a875af3d1208e8',
 'zstd-sys/zstd/lib/compress/zstd_compress_internal.h': 'ab7754413cef565da559bda7eb738bc2b3708c48877cb9ec37aff767d32c57d1',
 'zstd-sys/zstd/lib/compress/zstd_compress_literals.c': 'a8f25f1bd4c0d2d9dda3355d2103c9dfbd0b7056bfd373e29e923662bfea5596',
 'zstd-sys/zstd/lib/compress/zstd_compress_literals.h': '02270cd6bc060279217861776a53adb67f8d7e54f47f59d2a4ddcef2e705fe2d',
 'zstd-sys/zstd/lib/compress/zstd_compress_sequences.c': '3f4334c19ed770c4007bc12df4098026d4eb1a6fbebfbf36df58054e7babc3e9',
 'zstd-sys/zstd/lib/compress/zstd_compress_sequences.h': '361d108d7d452ed26f0ae4b39964f8e62166ed91d03347a5229e5d228734efc8',
 'zstd-sys/zstd/lib/compress/zstd_compress_superblock.c': 'dd62ed79ee1feec78b2e5bfc63417ccbdcda426f4220d112b0fcd8a9d0c7e667',
 'zstd-sys/zstd/lib/compress/zstd_compress_superblock.h': '6b097999ea1d91776ca5a12e5ee02c1ed15edfcd31ba07b9ada94b0955195677',
 'zstd-sys/zstd/lib/compress/zstd_cwksp.h': 'ed77c2739b18b9800b97785262526a1dd4f385311ca5c2caa20b6f2a35f3735d',
 'zstd-sys/zstd/lib/compress/zstd_double_fast.c': '9f6d003c2fe1208737bec685910324220424ba47d0323fa30d9d5f53c50e91da',
 'zstd-sys/zstd/lib/compress/zstd_double_fast.h': 'd9c29c9c004f572532265e12af78d5949c0908f4ef0bb0158977bcf80e1cf92b',
 'zstd-sys/zstd/lib/compress/zstd_fast.c': '8037320f321836652e73ab6cbf7b8f1f4c97cef38835ac6b4aaf0af716aa398d',
 'zstd-sys/zstd/lib/compress/zstd_fast.h': 'dffdf43fae7ee10a2acc04e6a5eaaf03f7d3572898117a06c330d647eed5591d',
 'zstd-sys/zstd/lib/compress/zstd_lazy.c': 'dd6ccf357165dc8cb574ea56a34ff1db57b7b31728b32103d0c6b72319fd45b1',
 'zstd-sys/zstd/lib/compress/zstd_lazy.h': '6f13007fbd6824058a73b097960446defe265e33b22c7c6dc2304dcf53998bbf',
 'zstd-sys/zstd/lib/compress/zstd_ldm.c': '4a2bef3612ae4a9e15a78a55541a152091fa8ba160410db24d495bd6dd57249e',
 'zstd-sys/zstd/lib/compress/zstd_ldm.h': 'bd0cb86041d3a72fd7846e141a41aadae229c8e331a2117df7f6b70a1b4cb08c',
 'zstd-sys/zstd/lib/compress/zstd_ldm_geartab.h': '7285ac8ba1f0fa57a8cd8158bd427ded4e70e32a58e73b57c420b83a76e5e6e3',
 'zstd-sys/zstd/lib/compress/zstd_opt.c': '625936ee3fb02d789abb894c1fbc2918ee47582434094c963b630da13532c876',
 'zstd-sys/zstd/lib/compress/zstd_opt.h': '9b677efbae28909034d3c24f37dc5c4295d267cf477bf04e14653579f90d8b6e',
 'zstd-sys/zstd/lib/compress/zstd_preSplit.c': '8d7da15de31318ebb7dfe2f2e2c79f05b0d0563dcb92d4525814d05a279d0d64',
 'zstd-sys/zstd/lib/compress/zstd_preSplit.h': '910facb2e3442e42a6952c514223c4502d95ed957bf7dc73982b3b03785b361b',
 'zstd-sys/zstd/lib/compress/zstdmt_compress.c': 'c83db699b4041bf4db89c5558db490dd295635f1eeefd60b643fbc862bbac7b5',
 'zstd-sys/zstd/lib/compress/zstdmt_compress.h': '033fb25d96b4295c3d0158edf4d1bd4067f42aac39427706754e6c034b20f80f',
 'zstd-sys/zstd/lib/decompress/huf_decompress.c': '710a88d877d5dc5c83a4f839392996f337ec81cf38006c4a5be6042de5b54828',
 'zstd-sys/zstd/lib/decompress/huf_decompress_amd64.S': '7fe53316261517a8f877b793f3bfba8305a3f9ff05c5947f1d709aaff346f1dc',
 'zstd-sys/zstd/lib/decompress/zstd_ddict.c': '38bf812283d61f4cd1f6aea30f84b43317418257c4e6fc198b09b436bd484397',
 'zstd-sys/zstd/lib/decompress/zstd_ddict.h': 'a97a250fd2f956e3ae419ddde68ae1f9fe4025bc0373254058d31bbd352fa71c',
 'zstd-sys/zstd/lib/decompress/zstd_decompress.c': '029580818b7e9cd38d9d07c63516b00ceaa943ffcfbd099fc5ccbe3628fa362f',
 'zstd-sys/zstd/lib/decompress/zstd_decompress_block.c': '9cb8bcb07aeb87e4717a9c8a86d20832dc525dde2c1b72efdaaab7c937f43b87',
 'zstd-sys/zstd/lib/decompress/zstd_decompress_block.h': '62ee0c6ae3f7353020538397f436b59743da8acb922068841b346f480f7078ac',
 'zstd-sys/zstd/lib/decompress/zstd_decompress_internal.h': '73217b49a644c0fcf567b70677cae31449ba9f50a235ef3b681001c3f36dc56d',
 'zstd-sys/zstd/lib/deprecated/zbuff.h': 'fcd83d7f05dc7bc6e52da6200878e05d7dc4906cf2ccfa70ce4c753e1fdc2422',
 'zstd-sys/zstd/lib/dictBuilder/cover.c': '2419631b20b0f4867d0f3c178fc4a3006d05f58e8ce3ce6657133f85ed40d683',
 'zstd-sys/zstd/lib/dictBuilder/cover.h': '6e2906e7e5c486a5b7f479f60210ff67d068b2c58744ad6a936ba5a9f4a23755',
 'zstd-sys/zstd/lib/dictBuilder/divsufsort.c': '2081acb08865f623857d2c0dcb0e79fce9489f01416528c30cfee7097915c616',
 'zstd-sys/zstd/lib/dictBuilder/divsufsort.h': '14c16c2f67019875a2ea6cd07a4e803adfadce448e0b80351c51e5f910230d38',
 'zstd-sys/zstd/lib/dictBuilder/fastcover.c': '0b8b511e6370e89cb257d1a4b4d8e77add5537ffb999cfe5e7e59e454c93f38f',
 'zstd-sys/zstd/lib/dictBuilder/zdict.c': 'dbe57910c9d446bbf2195f31bdba5cffef2c80b93c0325e9df8c9ffa744b0aa3',
 'zstd-sys/zstd/lib/legacy/zstd_legacy.h': '90ecc3816d28da0e0537c610fe99209fb9603b1ecdb764bd1228a5226f5c7484',
 'zstd-sys/zstd/lib/legacy/zstd_v01.c': '8f439ec4d83f13caaa4ee86d8de74660a38cade89b48bedf3943a9315b98e167',
 'zstd-sys/zstd/lib/legacy/zstd_v01.h': '51e06e92b87abe35f617d9e65c6f26b4b1b50485cda6bf34f5b23bb6e1727b13',
 'zstd-sys/zstd/lib/legacy/zstd_v02.c': 'ffedd2b0e4dae744b8592656050f2c56eb4c8ccad1795fb3efeceb7924808712',
 'zstd-sys/zstd/lib/legacy/zstd_v02.h': '341dd36148b7080eae742bb21196740482c4d94a97024f47ad1e7c0f2bb20e6b',
 'zstd-sys/zstd/lib/legacy/zstd_v03.c': '1d69626197b9c76d28012b78fc66bcc3077bf8d796ff6f82457666d3271e7f3d',
 'zstd-sys/zstd/lib/legacy/zstd_v03.h': 'e9d7ccc971055129a14e3b81f0e9494a638e3d9c0b4b36413252d6b3adc90cf2',
 'zstd-sys/zstd/lib/legacy/zstd_v04.c': 'a80d9591ff3fc387a05e8e075713af9ad34596dc5c8681666346a716013a6e14',
 'zstd-sys/zstd/lib/legacy/zstd_v04.h': 'c8c31b9db45c1b559688738f280331a5f3bce5b275cf4c71e8a41282f406134e',
 'zstd-sys/zstd/lib/legacy/zstd_v05.c': '62170472e18505b3347e563cdb1eba439312bb53385663730ac58cd92f3c4678',
 'zstd-sys/zstd/lib/legacy/zstd_v05.h': '6fb3be7f31544cee69cfb421d5bc6e34e72f4e2612b38cb38d47032a8531acdf',
 'zstd-sys/zstd/lib/legacy/zstd_v06.c': 'd2cadc9e2906fe50e9c9564bdcb291582532060fdcec9c4bc5f0ee4370182b75',
 'zstd-sys/zstd/lib/legacy/zstd_v06.h': '138728fdc9de7ebd5c7ffdf471b8b324cafa8e0ab638c6f4d9c048296cfe23ab',
 'zstd-sys/zstd/lib/legacy/zstd_v07.c': 'ae9f3c0a440b0f61d0ce44b06ee7ed3d256e10f3cb7dbbd9834d6c13625ac944',
 'zstd-sys/zstd/lib/legacy/zstd_v07.h': 'b682d3dffc64564fdf6f19d67baece362a1b7075c0140a80ce54e17f79961b49',
 'zstd-sys/zstd/lib/zdict.h': 'abacadb94e3f79e591f4b1648e839b0160fbf4291211fd01bdba1380269b245c',
 'zstd-sys/zstd/lib/zstd.h': '9b4bc8245565c98ccfc61c07749928b57e7c0f6fddb0530c4f6aa1971893d88b',
 'zstd-sys/zstd/lib/zstd_errors.h': '66a8c3f71d12ea6e797e4f622f31f3f8f81c41b36f48cad4f5de7d8bfb6aac0a'}
FOREIGN_FLAG_LITERAL_DIGEST = "65d2fad425e300ae19bd229a513aa7564086a7f0feb9405f806c70b2366ad59a"
FOREIGN_FLAG_LITERAL_LENGTH = 28


def foreign_e_only_declaration(inv, cwd, args, env, session):
    """Closed source-first negative ownership, never permission to call a tool."""
    name = next((n for n in FOREIGN_E_ONLY_PACKAGES if cwd == session / "vendor" / n
                 and cwd.resolve() == cwd and not cwd.is_symlink()), None)
    flag = None
    if name is None:
        out = Path(env.get("OUT_DIR", ""))
        if (cwd == out and out.is_absolute() and out.resolve() == out and not out.is_symlink()
                and out.name == "out" and out.parent.parent == session / "target" / TARGET / "debug/build"
                and re.fullmatch(r"zstd-sys-[0-9a-f]{16}", out.parent.name)
                and args[-2:] == ["-c", str(out / "flag_check.c")]):
            name = "zstd-sys"; flag = str(out / "flag_check.c")
    if name is None: return None
    version = FOREIGN_E_ONLY_PACKAGES[name][0]
    package = {"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
               "tree": "vendor", "manifest": name + "/Cargo.toml"}
    require(inv["packages"].count(package) == 1, "ForeignEOnlyDeclarationPackage")
    sources = {member: FOREIGN_E_ONLY_SOURCE_PINS[member] for member in FOREIGN_E_ONLY_INPUTS[name]}
    if flag is not None: sources[flag] = FOREIGN_FLAG_LITERAL_DIGEST
    return {"state": "DeclaredOnly", "source_sha256": sources}


def foreign_e_only_context(env, cwd, session, inv, name):
    version, components, features, links = FOREIGN_E_ONLY_PACKAGES[name]
    package, root, out = fixed_source_package(name, version, cwd, env, session, inv, ())
    require(env.get("CARGO_MANIFEST_PATH") == str(root / "Cargo.toml")
            and re.fullmatch(re.escape(name) + r"-[0-9a-f]{16}", out.parent.name), "ForeignManifestOutDir")
    # These four literals are observed Cargo fields, including the empty PRE;
    # full package VERSION retains build metadata and is checked independently.
    require(all(env.get("CARGO_PKG_VERSION_" + key) == value for key, value in
                zip(("MAJOR", "MINOR", "PATCH", "PRE"), components)), "ForeignVersion")
    require({key: value for key, value in env.items() if key.startswith("CARGO_FEATURE_")} ==
            {"CARGO_FEATURE_" + f.upper().replace("-", "_"): "1" for f in features}
            and env.get("CARGO_CFG_FEATURE") == ",".join(features)
            and env.get("CARGO_MANIFEST_LINKS") == links, "ForeignFeaturesLinks")
    expected = {"HOST": TARGET, "TARGET": TARGET, "CARGO_CFG_TARGET_ARCH": "x86_64",
        "CARGO_CFG_TARGET_OS": "macos", "CARGO_CFG_TARGET_ENV": "", "CARGO_CFG_TARGET_ENDIAN": "little",
        "CARGO_CFG_TARGET_VENDOR": "apple", "CARGO_CFG_TARGET_POINTER_WIDTH": "64",
        "CARGO_CFG_TARGET_FAMILY": "unix", "CARGO_CFG_TARGET_ABI": "", "CARGO_CFG_UNIX": "",
        "CARGO_CFG_TARGET_FEATURE": "cmpxchg16b,fxsr,sse,sse2,sse3,sse4.1,ssse3",
        "CARGO_CFG_TARGET_HAS_ATOMIC": "128,16,32,64,8,ptr", "CARGO_CFG_DEBUG_ASSERTIONS": "",
        "CARGO_CFG_PANIC": "unwind", "PROFILE": "debug", "DEBUG": "true", "OPT_LEVEL": "0"}
    require(all(env.get(key) == value for key, value in expected.items()), "ForeignConfiguration")
    require(not any(key in env for key in ("CARGO_CFG_MIRI", "CARGO_CFG_WINDOWS", "ZSTD_SYS_USE_PKG_CONFIG",
                                           "RING_PREGENERATE_ASM"))
            and not (root / ".git").exists() and not (root / ".git").is_symlink(), "ForeignSourceBranch")
    require(inv["packages"].count({"id": CC_PACKAGE, "tree": "vendor", "manifest": "cc/Cargo.toml"}) == 1,
            "ForeignCcPackage")
    members = {member for member in FOREIGN_E_ONLY_SOURCE_PINS if member.startswith(name + "/") or member.startswith("cc/")}
    sources = {}
    for member in sorted(members):
        path = session / "vendor" / member; regular(path)
        expected_sha = FOREIGN_E_ONLY_SOURCE_PINS[member]
        require(path.resolve() == path and path.stat().st_nlink == 1
                and inv["vendor"]["files"].get(member) == expected_sha == file_hash(path), "ForeignEOnlySourcePin")
        sources[member] = expected_sha
    require(sources[PROBE_LITERAL] == PROBE_DIGEST
            and (session / "vendor" / PROBE_LITERAL).stat().st_size == 206, "ForeignProbeLiteral")
    return {"lane": FOREIGN_LANE, "package_id": package["id"], "manifest": str(root),
            "out_dir": str(out), "source_sha256": sources}


def foreign_e_only_prior(session, context, current_call):
    """New-package pending calls reserve identities without successful authority."""
    rows = []; namespace = session / "foreign-native-invocations"
    if not namespace.exists(): return rows
    current_request = strict_json((current_call / "request.json").read_bytes()) if current_call else None
    current_source = (os.fsdecode(bytes.fromhex(current_request["args_hex"][-1])) if current_request else None)
    for call in namespace.iterdir():
        if call == current_call: continue
        require(ID.fullmatch(call.name) and call.is_dir() and call.resolve() == call
                and not call.is_symlink(), "ForeignCallAlias")
        request_path = call / "request.json"; receipt_path = call / "receipt.json"
        if not request_path.exists() and not request_path.is_symlink():
            require(not receipt_path.exists() and not receipt_path.is_symlink()
                    and all(re.fullmatch(r"request\.json\.pending-[0-9a-f]{32}", p.name)
                            and p.is_file() and not p.is_symlink() for p in call.iterdir()), "ForeignPendingRequest")
            # Empty/atomic-publish windows are not evidence. Final seal still
            # requires a complete regular request/receipt for every call.
            continue
        regular(request_path)
        require(request_path.resolve() == request_path and request_path.stat().st_nlink == 1, "ForeignReceiptBinding")
        request = strict_json(request_path.read_bytes())
        require(isinstance(request, dict), "ForeignPendingRequest")
        keys(request, {"schema", "state", "lane", "role", "args_hex", "cwd_hex", "environment_hex", "owner_issued_inspector"})
        require(request["schema"] == NATIVE_SCHEMA and request["state"] == "RecordingOnly"
                and request["lane"] == FOREIGN_LANE and request["owner_issued_inspector"] is False, "ForeignReceiptBinding")
        cwd = os.fsdecode(bytes.fromhex(request["cwd_hex"]))
        if cwd != context["manifest"]: continue
        args = [os.fsdecode(bytes.fromhex(value)) for value in request["args_hex"]]
        if not (args[:1] == ["-E"] and len(args) in (2, 3)): continue
        source = str((Path(cwd) / args[-1]).resolve())
        rejected = {"context": context, "protocol_state": "ProtocolRefused", "failures": ["ForeignPendingRequest"],
                    "operation": {"class": "CompilerFamilyFileProbe", "source": source, "retry": len(args) == 3}}
        if not receipt_path.exists():
            require(not receipt_path.is_symlink(), "ForeignReceiptBinding"); rows.append((call, rejected)); continue
        regular(receipt_path)
        require(receipt_path.resolve() == receipt_path and receipt_path.stat().st_nlink == 1, "ForeignReceiptBinding")
        receipt = strict_json(receipt_path.read_bytes())
        require(isinstance(receipt, dict) and receipt.get("operation_id") == call.name
                and all(receipt.get(key) == value for key, value in request.items()), "ForeignReceiptBinding")
        if receipt.get("context") != context:
            rows.append((call, rejected)); continue
        if source != current_source or receipt.get("protocol_state") != "Completed":
            rows.append((call, rejected)); continue
        if receipt.get("protocol_state") == "Completed":
            # A same-file retry needs genuine raw/control/stream/state evidence;
            # another label or a swallowed fault cannot authorize its child.
            policy = strict_json(POLICY.read_bytes()); owner = strict_json((session / "owner.json").read_bytes())
            inv = native_control(session, policy, owner)
            env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
            require(request.get("lane") == FOREIGN_LANE and request.get("role") == "cc"
                    and request.get("owner_issued_inspector") is False and args == ["-E", source]
                    and foreign_context(env, Path(cwd), session, inv) == context, "ForeignEOnlyPredecessor")
            foreign_environment(env, session, inv)
            controls = foreign_controls(session, policy, owner, context)
            require(all(receipt.get(key) == controls for key in ("controls_pre", "controls_post", "controls_return"))
                    and receipt.get("failures") == [] and type(receipt.get("tool_result")) is int
                    and receipt.get("tool_sha256") == inv["generators"]["CC"]["sha256"]
                    and receipt.get("argv_hex") == [os.fsencode(inv["generators"]["CC"]["path"]).hex(), *request["args_hex"]],
                    "ForeignEOnlyPredecessor")
            foreign_jobserver_binding(receipt["jobserver_identity"], env)
            require(receipt["jobserver_return"] == receipt["jobserver_identity"], "ForeignJobserverChanged")
            require(receipt.get("operation") == {"class": "CompilerFamilyFileProbe", "source": source,
                    "retry": False, "predecessor": None, "context_group": context["out_dir"]}, "ForeignEOnlyPredecessor")
            for stream in ("stdout", "stderr"):
                path = call / (stream + ".raw"); regular(path)
                require(path.resolve() == path and path.stat().st_nlink == 1
                        and file_hash(path) == receipt[stream + "_sha256"], "ForeignStreamChanged")
            require(all(isinstance(receipt.get(key), dict) for key in ("input_pre", "input_post"))
                    and native_state_key(receipt["input_pre"]) == native_state_key(receipt["input_post"])
                    and receipt["input_pre"]["exists"] is True and receipt["input_pre"]["path"] == source
                    and receipt["input_pre"]["length"] == 206 and receipt["input_pre"]["sha256"] == PROBE_DIGEST,
                    "ForeignInputChanged")
            native_state_check(call, receipt["input_pre"]); native_state_check(call, receipt["input_post"])
            require(foreign_semantics(call, receipt) == receipt.get("source_semantics"), "ForeignSourceSemantics")
        rows.append((call, receipt))
    return rows


def foreign_context(env, cwd, session, inv):
    # Source recognition precedes labels, flags and OUT_DIR. No generic fallback.
    require(cwd.is_absolute() and cwd.resolve() == cwd and not cwd.is_symlink(), "ForeignSourceAlias")
    selected = next((n for n in FOREIGN_E_ONLY_PACKAGES if cwd == session / "vendor" / n), None)
    if selected is not None: return foreign_e_only_context(env, cwd, session, inv, selected)
    name = next((n for n in FOREIGN_PACKAGES if cwd == session / "vendor" / n), None)
    require(name is not None, "ForeignSourceContext")
    version, features, links = FOREIGN_PACKAGES[name]
    package, root, out = fixed_source_package(name, version, cwd, env, session, inv, ())
    require(env.get("CARGO_MANIFEST_PATH") == str(root / "Cargo.toml")
            and re.fullmatch(re.escape(name) + r"-[0-9a-f]{16}", out.parent.name), "ForeignManifestOutDir")
    components = version.split(".")
    require(all(env.get("CARGO_PKG_VERSION_" + k) == v for k, v in
                zip(("MAJOR", "MINOR", "PATCH"), components))
            and env.get("CARGO_PKG_VERSION_PRE") == "", "ForeignVersion")
    require({k: v for k, v in env.items() if k.startswith("CARGO_FEATURE_")} ==
            {"CARGO_FEATURE_" + f.upper().replace("-", "_"): "1" for f in features}
            and env.get("CARGO_CFG_FEATURE") == ",".join(features)
            and (env.get("CARGO_MANIFEST_LINKS") == links if links is not None
                 else "CARGO_MANIFEST_LINKS" not in env), "ForeignFeaturesLinks")
    expected = {"HOST": TARGET, "TARGET": TARGET, "CARGO_CFG_TARGET_ARCH": "x86_64",
                "CARGO_CFG_TARGET_OS": "macos", "CARGO_CFG_TARGET_ENV": "",
                "CARGO_CFG_TARGET_ENDIAN": "little", "CARGO_CFG_TARGET_VENDOR": "apple",
                "CARGO_CFG_TARGET_POINTER_WIDTH": "64", "CARGO_CFG_TARGET_FAMILY": "unix",
                "CARGO_CFG_TARGET_ABI": "", "CARGO_CFG_UNIX": "", "PROFILE": "debug",
                "CARGO_CFG_TARGET_FEATURE": "cmpxchg16b,fxsr,sse,sse2,sse3,sse4.1,ssse3",
                "CARGO_CFG_TARGET_HAS_ATOMIC": "128,16,32,64,8,ptr", "CARGO_CFG_DEBUG_ASSERTIONS": "",
                "CARGO_CFG_PANIC": "unwind",
                "DEBUG": "true", "OPT_LEVEL": "0"}
    require(all(env.get(k) == v for k, v in expected.items()), "ForeignConfiguration")
    require("CARGO_CFG_MIRI" not in env and "RING_PREGENERATE_ASM" not in env
            and not (root / ".git").exists() and not (root / ".git").is_symlink(), "ForeignSourceBranch")
    if name == "ring":
        pregenerated = root / "pregenerated"
        require(pregenerated.is_dir() and not pregenerated.is_symlink()
                and pregenerated.resolve() == pregenerated, "ForeignSourceBranch")
    cc_package = {"id": CC_PACKAGE, "tree": "vendor", "manifest": "cc/Cargo.toml"}
    require(inv["packages"].count(cc_package) == 1, "ForeignCcPackage")
    sources = {}
    for member in (name + "/Cargo.toml", name + "/build.rs", *FOREIGN_CC_MEMBERS):
        path = session / "vendor" / member
        regular(path)
        require(path.resolve() == path and path.stat().st_nlink == 1
                and inv["vendor"]["files"].get(member) == file_hash(path), "ForeignSourcePin")
        sources[member] = file_hash(path)
    require(sources[PROBE_LITERAL] == PROBE_DIGEST
            and (session / "vendor" / PROBE_LITERAL).stat().st_size == 206, "ForeignProbeLiteral")
    return {"lane": FOREIGN_LANE, "package_id": package["id"], "manifest": str(root),
            "out_dir": str(out), "source_sha256": sources}


def foreign_environment(env, session, inv):
    native_environment(env, session, inv)
    require("LC_ALL" not in env and env.get("LC_CTYPE") == "C.UTF-8"
            and "ZERO_AR_DATE" not in env, "ForeignProbeEnvironment")
    require(env.get("CARGO") == inv["cargo"]["path"]
            and env.get("HOME") == str(session / "home")
            and env.get("TMPDIR") == str(session / "tmp")
            and env.get("CARGO_HOME") == str(session / "cargo-home"), "ForeignSessionEnvironment")


def foreign_controls(session, policy, owner, context):
    inv = native_control(session, policy, owner)
    return {"owner_source_sha256": file_hash(Path(__file__)), "policy_sha256": digest(POLICY.read_bytes()),
            "session_owner_sha256": file_hash(session / "owner.json"),
            "launchers": owner["native_launchers"], "tools": {r: inv["generators"][r] for r in ("CC", "AR")},
            "source_sha256": context["source_sha256"]}


def foreign_prior(session, context, current_call=None):
    if Path(context["manifest"]).name in FOREIGN_E_ONLY_PACKAGES:
        return foreign_e_only_prior(session, context, current_call)
    rows = []
    root = session / "foreign-native-invocations"
    if not root.exists(): return rows
    for call in root.iterdir():
        if call == current_call: continue
        if (call / "receipt.json").is_file():
            receipt = strict_json((call / "receipt.json").read_bytes())
            if receipt.get("context") == context: rows.append((call, receipt))
        else:
            # A pending/request-only first E reserves its source identity too.
            # Concurrent duplicate requests may both refuse; neither gains entry.
            request = strict_json((call / "request.json").read_bytes())
            cwd = os.fsdecode(bytes.fromhex(request["cwd_hex"]))
            args = [os.fsdecode(bytes.fromhex(v)) for v in request["args_hex"]]
            if cwd == context["manifest"] and args[:1] == ["-E"] and len(args) in (2, 3):
                rows.append((call, {"context": context, "protocol_state": "ProtocolRefused",
                    "failures": ["ForeignPendingRequest"], "operation": {"class": "CompilerFamilyFileProbe",
                    "source": str((Path(cwd) / args[-1]).resolve()), "retry": len(args) == 3}}))
    return rows


def foreign_probe_classify(role, args, context, env, session, inv, *, history=None, current_call=None):
    require(role == "cc", "ForeignRole")
    if Path(context["manifest"]).name in FOREIGN_E_ONLY_PACKAGES:
        require((len(args) == 2 and args[:1] == ["-E"])
                or (len(args) == 3 and args[:2] == ["-E", "--"]), "ForeignEOnlyArgv")
    # Fresh Command::new calls have no Tool.args or compile locale adjustment.
    if args == ["-?"]:
        return {"class": "CompilerFamilyHelpProbe", "context_group": context["out_dir"],
                "probe_predecessor": "not_observed"}
    if args == ["--version"]:
        return {"class": "CompilerFamilyVersionProbe", "context_group": context["out_dir"],
                "probe_predecessor": "not_observed"}
    retry = len(args) == 3 and args[:2] == ["-E", "--"]
    require((len(args) == 2 and args[:1] == ["-E"]) or retry, "ForeignStageAArgv")
    path = Path(args[-1]); out = Path(context["out_dir"])
    match = re.fullmatch(r"(0|[1-9][0-9]{0,19})detect_compiler_family\.c", path.name)
    require(match and int(match[1]) <= 2**64 - 1 and path.parent == out
            and path.is_absolute() and str(path) == args[-1] and path.resolve() == path, "ForeignProbePath")
    rows = foreign_prior(session, context, current_call) if history is None else history
    prior = [(c, r) for c, r in rows if r.get("operation", {}).get("source") == str(path)]
    require((not retry and not prior) or (retry and len(prior) == 1), "ForeignProbePredecessor")
    predecessor = None
    if retry:
        call, receipt = prior[0]
        require(receipt["protocol_state"] == "Completed" and not receipt["failures"]
                and receipt["operation"]["class"] == "CompilerFamilyFileProbe"
                and not receipt["operation"]["retry"]
                and any(b"-Wslash-u-filename" in (call / (s + ".raw")).read_bytes()
                        for s in ("stdout", "stderr")), "ForeignProbePredecessor")
        native_state_check(call, receipt["input_post"], live=history is None)
        predecessor = call.name
    return {"class": "CompilerFamilyFileProbe", "source": str(path), "retry": retry,
            "predecessor": predecessor, "context_group": context["out_dir"]}


def foreign_semantics(call, receipt):
    code = receipt["tool_result"]
    require(type(code) is int, "ForeignToolStatus")
    kind = receipt["operation"]["class"]
    stdout = (call / "stdout.raw").read_bytes().decode("utf-8", errors="replace")
    stderr = (call / "stderr.raw").read_bytes().decode("utf-8", errors="replace")
    if kind == "CompilerFamilyHelpProbe":
        return {"accepts_cl_style_flags": code == 0,
                "source_branch": "MsvcArmCandidate" if code == 0 else "ClStyleRejected"}
    if kind == "CompilerFamilyVersionProbe":
        return {"zig_cc": code == 0 and "ziglang" in stdout,
                "source_nonzero_default": code != 0}
    if kind == "CompilerObjectCompile":
        return {"compiler_success": code == 0, "object_observed": receipt["output_post"]["exists"]}
    if kind in {"ArchiverFirstAppend", "ArchiverRemainingAppend"}:
        return {"append_success": code == 0, "archive_observed": receipt["archive_post"]["exists"],
                "fallback_requested": receipt["operation"]["mode"] == "cqD" and code != 0}
    if kind == "ArchiverIndex":
        return {"index_success": code == 0, "archive_observed": receipt["archive_post"]["exists"]}
    warning = "-Wslash-u-filename" in stdout or "-Wslash-u-filename" in stderr
    effective = code == 0 and (receipt["operation"]["retry"] or not warning)
    return {"warning_retry_requested": warning and not receipt["operation"]["retry"],
            "effective_stdout": effective,
            "markers": {m: ('"' + m + '"') in stdout for m in ("clang", "gcc", "emscripten", "VxWorks")}}


def foreign_compile_environment(env, session, inv):
    native_environment(env, session, inv)
    require(env.get("LC_ALL") == "C" and "LC_CTYPE" not in env and "ZERO_AR_DATE" not in env,
            "ForeignCompileEnvironment")
    require(env.get("CARGO") == inv["cargo"]["path"] and env.get("HOME") == str(session / "home")
            and env.get("TMPDIR") == str(session / "tmp") and env.get("CARGO_HOME") == str(session / "cargo-home"),
            "ForeignSessionEnvironment")


def foreign_compile_inputs(inv, name):
    """Negative-only declaration for the closed ring/psm input set; no observation."""
    require(name in FOREIGN_COMPILE_SOURCES, "ForeignCompileSource")
    sources = {name + "/" + member for member in FOREIGN_COMPILE_SOURCES[name]}
    if name == "ring":
        include = {m for m in inv["vendor"]["files"] if m.startswith("ring/include/")}
        sources.update(include)
        sources.update(m for m in inv["vendor"]["files"] if m.startswith("ring/pregenerated/"))
        # Closed private header leaves from the existing ring0.17.14 policy.
        # They are native inputs, with the same pins/quarantine as public includes.
        sources.update(('ring/crypto/curve25519/curve25519_tables.h', 'ring/crypto/curve25519/internal.h', 'ring/crypto/fipsmodule/bn/internal.h', 'ring/crypto/fipsmodule/ec/ecp_nistz.h', 'ring/crypto/fipsmodule/ec/ecp_nistz384.h', 'ring/crypto/fipsmodule/ec/ecp_nistz384.inl', 'ring/crypto/fipsmodule/ec/p256-nistz-table.h', 'ring/crypto/fipsmodule/ec/p256-nistz.h', 'ring/crypto/fipsmodule/ec/p256_shared.h', 'ring/crypto/fipsmodule/ec/p256_table.h', 'ring/crypto/fipsmodule/ec/util.h', 'ring/crypto/internal.h', 'ring/crypto/limbs/limbs.h', 'ring/crypto/limbs/limbs.inl', 'ring/third_party/fiat/curve25519_32.h', 'ring/third_party/fiat/curve25519_64.h', 'ring/third_party/fiat/curve25519_64_adx.h', 'ring/third_party/fiat/curve25519_64_msvc.h', 'ring/third_party/fiat/p256_32.h', 'ring/third_party/fiat/p256_64.h', 'ring/third_party/fiat/p256_64_msvc.h'))
    return {"state": "DeclaredOnly", "source_sha256":
            {member: inv["vendor"]["files"].get(member) for member in sorted(sources)}}


def foreign_compile_pins(session, inv, context):
    name = Path(context["manifest"]).name
    if name == "ring":
        require(any(m.startswith("ring/include/") for m in inv["vendor"]["files"]), "ForeignCompileInclude")
    sources = foreign_compile_inputs(inv, name)["source_sha256"]
    helpers = set(FOREIGN_COMPILE_CC_MEMBERS)
    source_pins, helper_pins = {}, {}
    for members, pins in ((sources, source_pins), (helpers, helper_pins)):
        for member in sorted(members):
            path = session / "vendor" / member; regular(path)
            require(path.resolve() == path and path.stat().st_nlink == 1
                    and inv["vendor"]["files"].get(member) == file_hash(path), "ForeignCompileSourcePin")
            pins[member] = file_hash(path)
    require(inv["rustc"]["sha256"] == FOREIGN_MAP_RUSTC_SHA256
            and context["source_sha256"]["cc/src/command_helpers.rs"] == FOREIGN_MAP_CC_SHA256,
            "ForeignObjectMapBinding")
    return {"source_sha256": source_pins, "helper_sha256": helper_pins, "rustc": inv["rustc"],
            "object_map_cc_sha256": FOREIGN_MAP_CC_SHA256}


def foreign_compile_classify(role, args, context, session, *, current_call=None):
    require(role == "cc", "ForeignRole")
    root = Path(context["manifest"]); name = root.name
    require(len(args) >= 5 and args[-4] == "-o" and args[-2] == "-c", "ForeignCompileArgv")
    raw_source = args[-1]
    member = next((m for m in FOREIGN_COMPILE_SOURCES[name]
                   if raw_source == (str(root / m) if name == "ring" else m)), None)
    require(member is not None, "ForeignCompileSource")
    # Match cc's literal dirname strip_prefix and extension writes; no Path.hash.
    dirname = str(Path(raw_source).parent)
    if dirname.startswith(str(root)): dirname = dirname[len(str(root)):]
    extension = Path(raw_source).suffix[1:]
    prefix = FOREIGN_OBJECT_PREFIXES.get((dirname, extension)); require(prefix is not None, "ForeignObjectMap")
    output = Path(context["out_dir"]) / (prefix + "-" + Path(raw_source).with_suffix(".o").name)
    flags = list(FOREIGN_COMPILE_COMMON)
    flags += ["-I", str(root / "include"), "-I", str(root / "pregenerated"), *FOREIGN_COMPILE_RING] if name == "ring" else list(FOREIGN_COMPILE_PSM)
    require(args == [*flags, "-o", str(output), "-c", raw_source], "ForeignCompileArgv")
    require(output.resolve() == output and output.is_absolute(), "ForeignCompileOutputAlias")
    # An immutable request reserves its output even if execution/receipt failed.
    namespace = session / "foreign-native-invocations"
    for other in namespace.iterdir():
        if other == current_call: continue
        if not (other / "request.json").exists() and not (other / "receipt.json").exists(): continue
        request = strict_json((other / "request.json").read_bytes())
        other_cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
        values = [os.fsdecode(bytes.fromhex(v)) for v in request["args_hex"]]
        for index, value in enumerate(values[:-1]):
            if value == "-o":
                require((other_cwd / values[index + 1]).resolve() != output, "ForeignCompileOwnership")
    return {"class": "CompilerObjectCompile", "source": str(root / member), "raw_source": raw_source,
            "output": str(output), "object_derivation": {"dirname": dirname, "extension": extension, "prefix": prefix},
            "context_group": context["out_dir"]}


def foreign_compile_family(session, policy, owner, context, *, current_call=None, allow_archive=False):
    inv = native_control(session, policy, owner); controls = foreign_controls(session, policy, owner, context)
    rows = []; prior_compiles = []; kinds = {"CompilerFamilyFileProbe", "CompilerFamilyHelpProbe", "CompilerFamilyVersionProbe"}
    namespace = session / "foreign-native-invocations"
    for call in namespace.iterdir():
        if call == current_call: continue
        require(ID.fullmatch(call.name) and call.is_dir() and call.resolve() == call and not call.is_symlink(), "ForeignFamilyEvidenceAlias")
        request_path = call / "request.json"
        # A not-yet-published concurrent call has no evidence/ownership authority.
        # A retained receipt with a missing request is a fault, never skipped.
        if not request_path.exists() and not (call / "receipt.json").exists(): continue
        regular(request_path)
        require(request_path.resolve() == request_path and request_path.stat().st_nlink == 1, "ForeignFamilyEvidenceAlias")
        request = strict_json(request_path.read_bytes())
        env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
        cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
        if str(cwd) != context["manifest"] or env.get("OUT_DIR") != context["out_dir"]: continue
        keys(request, {"schema", "state", "lane", "role", "args_hex", "cwd_hex", "environment_hex", "owner_issued_inspector"})
        require(request["schema"] == NATIVE_SCHEMA and request["state"] == "RecordingOnly" and request["lane"] == FOREIGN_LANE
                and request["owner_issued_inspector"] is False, "ForeignFamilyReceiptBinding")
        args = [os.fsdecode(bytes.fromhex(v)) for v in request["args_hex"]]
        if allow_archive and request["role"] == "ar":
            # Archive callers separately validate every skipped request/receipt.
            continue
        receipt_path = call / "receipt.json"
        if not receipt_path.exists():
            # Concurrent fixed object requests are not probe predecessors or successes.
            # Their eventual faults remain sticky in the aggregate, including request-only cuts.
            require("-c" in args, "ForeignFamilyPending")
            foreign_compile_environment(env, session, inv)
            require(foreign_context(env, cwd, session, inv) == context, "ForeignFamilyControl")
            foreign_compile_classify(request["role"], args, context, session, current_call=call)
            continue
        regular(receipt_path)
        require(receipt_path.resolve() == receipt_path and receipt_path.stat().st_nlink == 1, "ForeignFamilyEvidenceAlias")
        receipt = strict_json(receipt_path.read_bytes())
        require(request["schema"] == NATIVE_SCHEMA and request["state"] == "RecordingOnly" and request["lane"] == FOREIGN_LANE
                and request["owner_issued_inspector"] is False and receipt["operation_id"] == call.name
                and all(receipt[k] == v for k, v in request.items()), "ForeignFamilyReceiptBinding")
        require(receipt["protocol_state"] == "Completed" and receipt["failures"] == [], "ForeignFamilySticky")
        computed_context = foreign_context(env, cwd, session, inv)
        require(computed_context == context and receipt["context"] == context
                and all(receipt.get(k) == controls for k in ("controls_pre", "controls_post", "controls_return")), "ForeignFamilyControl")
        args = [os.fsdecode(bytes.fromhex(v)) for v in request["args_hex"]]
        require(receipt["tool_sha256"] == inv["generators"]["CC"]["sha256"]
                and receipt["argv_hex"] == [os.fsencode(inv["generators"]["CC"]["path"]).hex(), *request["args_hex"]], "ForeignFamilyTool")
        foreign_jobserver_binding(receipt["jobserver_identity"], env)
        require(receipt["jobserver_return"] == receipt["jobserver_identity"], "ForeignJobserverChanged")
        for stream in ("stdout", "stderr"):
            path = call / (stream + ".raw"); regular(path)
            require(path.resolve() == path and path.stat().st_nlink == 1
                    and file_hash(path) == receipt[stream + "_sha256"], "ForeignFamilyStream")
        semantics = foreign_semantics(call, receipt)
        require(semantics == receipt["source_semantics"], "ForeignFamilySemantics")
        kind = receipt["operation"]["class"]
        if kind == "CompilerObjectCompile":
            foreign_compile_environment(env, session, inv)
            require(semantics["compiler_success"], "ForeignFamilySticky")
            operation = foreign_compile_classify(receipt["role"], args, context, session, current_call=call)
            require(operation == receipt["operation"], "ForeignFamilyClassification")
            pins = foreign_compile_pins(session, inv, context)
            require(all(receipt.get(k) == pins for k in ("compile_pins_pre", "compile_pins_post", "compile_pins_return")), "ForeignCompilePinChanged")
            require(all(isinstance(receipt.get(k), dict) for k in ("input_pre", "input_post", "output_pre", "output_post")), "ForeignSnapshotFields")
            require(native_state_key(receipt["input_pre"]) == native_state_key(receipt["input_post"])
                    and receipt["input_pre"]["exists"] is True
                    and receipt["input_pre"]["path"] == receipt["input_post"]["path"] == operation["source"]
                    and receipt["input_pre"]["sha256"] == pins["source_sha256"][str(Path(operation["source"]).relative_to(session / "vendor"))]
                    and receipt["output_pre"] == {"exists": False, "path": operation["output"]}
                    and receipt["output_post"]["path"] == operation["output"]
                    and receipt["output_post"]["exists"] is True and receipt["output_post"]["length"] > 0, "ForeignCompileSnapshot")
            native_state_check(call, receipt["input_pre"]); native_state_check(call, receipt["input_post"], live=True)
            native_state_check(call, receipt["output_post"], live=True)
            prior_compiles.append(receipt)
            continue
        require(kind in kinds, "ForeignFamilyClass"); foreign_environment(env, session, inv)
        rows.append((call, receipt, args))
    files = [(c, r) for c, r, _ in rows if r["operation"]["class"] == "CompilerFamilyFileProbe"]
    effective = []; references = {}
    for call, receipt, args in rows:
        predecessor = receipt["operation"].get("predecessor")
        history = [(c, r) for c, r in files if c.name == predecessor] if predecessor else []
        operation = foreign_probe_classify(receipt["role"], args, context, {}, session, inv, history=history)
        require(operation == receipt["operation"], "ForeignFamilyClassification")
        kind = operation["class"]
        if kind == "CompilerFamilyFileProbe":
            pre, post = receipt.get("input_pre"), receipt.get("input_post")
            require(isinstance(pre, dict) and isinstance(post, dict), "ForeignSnapshotFields")
            require(native_state_key(pre) == native_state_key(post) and pre["exists"] is True
                    and pre["path"] == post["path"] == operation["source"] and pre["length"] == 206
                    and pre["sha256"] == PROBE_DIGEST, "ForeignFamilyLiteral")
            native_state_check(call, pre); native_state_check(call, post, live=True, retire=True)
            if receipt["source_semantics"]["effective_stdout"]: effective.append((call, receipt))
        else:
            require(not any(k in receipt for k in ("input_pre", "input_post", "output_pre", "output_post", "archive_pre", "archive_post")), "ForeignOutputFreeClass")
        reference = {"operation_id": call.name, "request_sha256": file_hash(call / "request.json"),
                     "receipt_sha256": file_hash(call / "receipt.json"),
                     "stdout_sha256": receipt["stdout_sha256"], "stderr_sha256": receipt["stderr_sha256"]}
        references.setdefault(kind, []).append(reference)
    require(len(files) in (1, 2) and len(effective) == 1, "ForeignFamilyUnique")
    first = [(c, r) for c, r in files if not r["operation"]["retry"]]
    require(len(first) == 1 and all(not r["operation"]["retry"] or r["operation"]["predecessor"] == first[0][0].name for _, r in files), "ForeignFamilyUnique")
    for kind in ("CompilerFamilyHelpProbe", "CompilerFamilyVersionProbe"):
        require(len(references.get(kind, [])) == 1, "ForeignFamilyUnique")
    _, e = effective[0]
    help_row = next(r for _, r, _ in rows if r["operation"]["class"] == "CompilerFamilyHelpProbe")
    version = next(r for _, r, _ in rows if r["operation"]["class"] == "CompilerFamilyVersionProbe")
    require(e["tool_result"] == 0 and e["source_semantics"]["markers"]["clang"]
            and not e["source_semantics"]["warning_retry_requested"] and help_row["tool_result"] == 1
            and help_row["source_semantics"] == {"accepts_cl_style_flags": False, "source_branch": "ClStyleRejected"}
            and version["tool_result"] == 0 and version["source_semantics"] == {"zig_cc": False, "source_nonzero_default": False}, "ForeignFamilyClang")
    evidence = {"family": "Clang", "effective_e_operation_id": effective[0][0].name,
                "context_group": context["out_dir"], "evidence": {k: sorted(v, key=lambda r: r["operation_id"]) for k, v in references.items()}}
    require(all(r["family_pre"] == r["family_return"] == evidence for r in prior_compiles), "ForeignFamilyChanged")
    return evidence


# Finite bundled3 append requests only. Remaining core/index/test are declarations,
# not an archiver/provider fallback or completed archive/consumer qualification.
FOREIGN_ARCHIVE_FIRST_MEMBERS = {
    "ring": ("25ac62e5b3c53843-curve25519.o", "0bbbd18bda93c05b-aes_nohw.o",
        "00c879ee3285a50d-montgomery.o", "00c879ee3285a50d-montgomery_inv.o",
        "a0330e891e733f4e-ecp_nistz.o", "a0330e891e733f4e-gfp_p256.o",
        "a0330e891e733f4e-gfp_p384.o", "a0330e891e733f4e-p256.o", "aaa1ba3e455ee2e1-limbs.o",
        "a4019cc0736b0423-mem.o", "d5a9841f3dc6e253-poly1305.o", "a4019cc0736b0423-crypto.o",
        "a4019cc0736b0423-cpu_intel.o", "25ac62e5b3c53843-curve25519_64_adx.o",
        "e165cd818145c705-fiat_curve25519_adx_mul.o", "e165cd818145c705-fiat_curve25519_adx_square.o"),
    "psm": ("4f9a91766097c4c5-x86_64.o",)}


FOREIGN_ARCHIVE_REMAINING_MEMBERS = (
    "a0330e891e733f4e-p256-nistz.o", "c322a0bcc369f531-chacha-x86_64-macosx.o",
    "c322a0bcc369f531-aes-gcm-avx2-x86_64-macosx.o", "c322a0bcc369f531-aesni-gcm-x86_64-macosx.o",
    "c322a0bcc369f531-aesni-x86_64-macosx.o", "c322a0bcc369f531-ghash-x86_64-macosx.o",
    "c322a0bcc369f531-vpaes-x86_64-macosx.o", "c322a0bcc369f531-x86_64-mont-macosx.o",
    "c322a0bcc369f531-x86_64-mont5-macosx.o", "c322a0bcc369f531-p256-x86_64-asm-macosx.o",
    "c322a0bcc369f531-sha512-x86_64-macosx.o", "c322a0bcc369f531-chacha20_poly1305_x86_64-macosx.o",
    "c322a0bcc369f531-sha256-x86_64-macosx.o")


def foreign_archive_environment(env, session, inv):
    native_environment(env, session, inv)
    require("LC_ALL" not in env and env.get("LC_CTYPE") == "C.UTF-8"
            and env.get("ZERO_AR_DATE") == "1", "ForeignArchiveEnvironment")
    require(not any(k in env for k in ("ARFLAGS", "RANLIB", "RANLIBFLAGS")), "ForeignArchiveEnvironment")
    require(env.get("CARGO") == inv["cargo"]["path"] and env.get("HOME") == str(session / "home")
            and env.get("TMPDIR") == str(session / "tmp") and env.get("CARGO_HOME") == str(session / "cargo-home"),
            "ForeignSessionEnvironment")


def foreign_archive_classify(role, args, context):
    require(role == "ar", "ForeignArchiveRole")
    name = Path(context["manifest"]).name; out = Path(context["out_dir"])
    archive = out / (RING_ARCHIVES[0] if name == "ring" else PSM_ARCHIVE)
    members = [str(out / n) for n in FOREIGN_ARCHIVE_FIRST_MEMBERS[name]]
    kind = "ArchiverFirstAppend"
    if name == "ring" and args == ["cq", str(archive), *[str(out / n) for n in FOREIGN_ARCHIVE_REMAINING_MEMBERS]]:
        # Legacy first-only observations do not assert a complete cc partition.
        # This new remaining-stage template alone requires the genuine byte rule.
        foreign_archive_partition(context)
        members = [str(out / n) for n in FOREIGN_ARCHIVE_REMAINING_MEMBERS]
        kind = "ArchiverRemainingAppend"
    elif name == "psm" and args == ["s", str(archive)]:
        # The original one-member ledger is semantic input, not members argv.
        kind = "ArchiverIndex"
    else:
        require(args[:1] in (["cqD"], ["cq"]) and args == [args[0], str(archive), *members], "ForeignArchiveTemplate")
    require(archive.resolve() == archive and all(Path(p).resolve() == Path(p) for p in members), "ForeignArchiveAlias")
    return {"class": kind, "mode": args[0], "archive": str(archive),
            "members": members, "context_group": context["out_dir"]}


def foreign_archive_partition(context):
    out = Path(context["out_dir"])
    require(out.is_absolute() and out.resolve() == out and not out.is_symlink(), "ForeignArchiveAlias")
    names = (*FOREIGN_ARCHIVE_FIRST_MEMBERS["ring"], *FOREIGN_ARCHIVE_REMAINING_MEMBERS)
    batches = []; batch = []; remaining = 4000
    for name in names:
        path = str(out / name); length = len(os.fsencode(path))
        if batch and length > remaining:
            batches.append(batch); batch = []; remaining = 4000
        batch.append(path); remaining = max(0, remaining - length)
    if batch: batches.append(batch)
    require(batches == [[str(out / n) for n in FOREIGN_ARCHIVE_FIRST_MEMBERS["ring"]],
                        [str(out / n) for n in FOREIGN_ARCHIVE_REMAINING_MEMBERS]], "ForeignArchiveBatchGeometry")
    return batches


def foreign_archive_stage(operation):
    stages = {("ArchiverFirstAppend", "cqD"): "FirstD", ("ArchiverFirstAppend", "cq"): "FirstFallbackCQ",
              ("ArchiverRemainingAppend", "cq"): "RingRemainingCQ", ("ArchiverIndex", "s"): "PSMIndexS"}
    stage = stages.get((operation["class"], operation["mode"]))
    require(stage is not None, "ForeignArchiveTemplate")
    return stage


def foreign_archive_producers(session, context, operation, *, current_call=None):
    rows = {}
    for call in (session / "foreign-native-invocations").iterdir():
        if call == current_call or not (call / "receipt.json").exists(): continue
        receipt = strict_json((call / "receipt.json").read_bytes())
        require(isinstance(receipt, dict), "ForeignArchiveReceiptFields")
        if receipt.get("context") != context: continue
        request_path = call / "request.json"; regular(request_path)
        require(request_path.resolve() == request_path and request_path.stat().st_nlink == 1, "ForeignArchiveEvidenceAlias")
        request = strict_json(request_path.read_bytes())
        # Role is read from raw request, never guessed from a nullable operation.
        # The caller must independently validate ALL same-context AR history.
        require(request["role"] == receipt.get("role"), "ForeignArchiveReceiptBinding")
        if request["role"] == "ar": continue
        operation_row = receipt.get("operation")
        require(isinstance(operation_row, dict), "ForeignArchiveOperationFields")
        if operation_row.get("class") != "CompilerObjectCompile": continue
        output = operation_row["output"]
        if output not in operation["members"]: continue
        require(output not in rows, "ForeignArchiveMemberUnique")
        require(receipt["protocol_state"] == "Completed" and receipt["tool_result"] == 0
                and receipt["failures"] == [], "ForeignArchiveMemberProducer")
        receipt_path = call / "receipt.json"; regular(receipt_path)
        require(receipt_path.resolve() == receipt_path and receipt_path.stat().st_nlink == 1, "ForeignArchiveEvidenceAlias")
        rows[output] = {"path": output, "operation_id": call.name,
            "request_sha256": file_hash(request_path), "receipt_sha256": file_hash(receipt_path),
            "sha256": receipt["output_post"]["sha256"], "length": receipt["output_post"]["length"]}
    require(set(rows) == set(operation["members"]), "ForeignArchiveMemberProducer")
    return [rows[path] for path in operation["members"]]


def foreign_archive_predecessor(operation, chain):
    stage = foreign_archive_stage(operation)
    if stage == "FirstD": require(not chain, "ForeignArchivePredecessor")
    elif stage == "FirstFallbackCQ":
        require(len(chain) == 1 and foreign_archive_stage(chain[0][1]["operation"]) == "FirstD"
                and chain[0][1]["tool_result"] != 0, "ForeignArchivePredecessor")
    else:
        require(len(chain) == 2 and foreign_archive_stage(chain[0][1]["operation"]) == "FirstD"
                and chain[0][1]["tool_result"] != 0
                and foreign_archive_stage(chain[1][1]["operation"]) == "FirstFallbackCQ"
                and chain[1][1]["tool_result"] == 0, "ForeignArchivePredecessor")
    require(not chain or all(r["operation"]["archive"] == operation["archive"] for _, r in chain), "ForeignArchivePredecessor")
    if stage in ("RingRemainingCQ", "PSMIndexS"):
        name = "ring" if stage == "RingRemainingCQ" else "psm"
        expected = [str(Path(operation["archive"]).parent / n) for n in FOREIGN_ARCHIVE_FIRST_MEMBERS[name]]
        require([p["path"] for p in foreign_archive_ledger(chain)["producers"]] == expected,
                "ForeignArchiveLedger")


def foreign_archive_ledger(chain):
    producers = []
    for _, receipt in chain:
        if receipt["operation"]["class"] != "ArchiverIndex" and receipt["tool_result"] == 0:
            producers.extend(receipt["archive_member_producers_pre"])
    require(len({p["path"] for p in producers}) == len(producers), "ForeignArchiveLedgerUnique")
    # Successful append operands are recorded; binary archive inventory is not.
    return {"state": "RecordingOnly", "archive_member_inventory": "not_observed", "producers": producers}


def foreign_archive_members(session, policy, owner, context, operation, *, current_call=None):
    # The archive-only family exception supplies no AR success evidence.
    # Complete independent history validates every skipped AR request/receipt.
    family = foreign_compile_family(session, policy, owner, context, current_call=current_call, allow_archive=True)
    return family, foreign_archive_producers(session, context, operation, current_call=current_call)


def foreign_archive_states(call, receipt, operation, *, live_archive=False):
    before, after = receipt.get("archive_members_pre"), receipt.get("archive_members_post")
    require(isinstance(before, list) and isinstance(after, list)
            and len(before) == len(after) == len(operation["members"]), "ForeignArchiveSnapshotFields")
    references = receipt["archive_member_producers_pre"]
    require(isinstance(references, list) and len(references) == len(before), "ForeignArchiveSnapshotFields")
    for index, (path, pre, post, reference) in enumerate(zip(operation["members"], before, after, references)):
        require(isinstance(pre, dict) and isinstance(post, dict), "ForeignArchiveSnapshotFields")
        require(pre["exists"] is True and native_state_key(pre) == native_state_key(post)
                and pre["path"] == post["path"] == path and pre["sha256"] == reference["sha256"]
                and pre["length"] == reference["length"] and pre["snapshot"] == "archive-member-pre-" + str(index) + ".raw"
                and post["snapshot"] == "archive-member-post-" + str(index) + ".raw", "ForeignArchiveInputChanged")
        native_state_check(call, pre); native_state_check(call, post, live=True)
    for key, leaf in (("archive_pre", "archive-pre.raw"), ("archive_post", "archive-post.raw")):
        state = receipt.get(key); require(isinstance(state, dict), "ForeignArchiveSnapshotFields")
        require(state["path"] == operation["archive"] and (not state["exists"] or state["snapshot"] == leaf), "ForeignArchiveSnapshot")
        native_state_check(call, state, live=live_archive and key == "archive_post")
    require(receipt["tool_result"] != 0 or (receipt["archive_post"]["exists"] is True
            and receipt["archive_post"]["length"] > 0), "ForeignArchiveMissing")


def foreign_archive_history(session, policy, owner, context, family, *, current_call=None, live_archive=False):
    inv = native_control(session, policy, owner); controls = foreign_controls(session, policy, owner, context)
    rows = {}
    for call in (session / "foreign-native-invocations").iterdir():
        if call == current_call: continue
        require(ID.fullmatch(call.name) and call.is_dir() and call.resolve() == call and not call.is_symlink(), "ForeignArchiveEvidenceAlias")
        if not (call / "request.json").exists() and not (call / "receipt.json").exists(): continue
        request = strict_json((call / "request.json").read_bytes())
        receipt_path = call / "receipt.json"
        receipt = strict_json(receipt_path.read_bytes()) if receipt_path.exists() else None
        env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
        cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
        raw_group = str(cwd) == context["manifest"] and env.get("OUT_DIR") == context["out_dir"]
        retained_context = receipt.get("context") if isinstance(receipt, dict) else None
        retained_group = (isinstance(retained_context, dict)
            and retained_context.get("manifest") == context["manifest"]
            and retained_context.get("out_dir") == context["out_dir"])
        retained_raw_group = False
        if isinstance(receipt, dict):
            try:
                retained_env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v))
                                for k, v in receipt["environment_hex"].items()}
                retained_cwd = os.fsdecode(bytes.fromhex(receipt["cwd_hex"]))
                retained_raw_group = retained_cwd == context["manifest"] and retained_env.get("OUT_DIR") == context["out_dir"]
            except (KeyError, TypeError, ValueError, AttributeError):
                # Invalid retained fields are not evidence of an unrelated group.
                # Any matching raw/context view still enters the strict gate below.
                pass
        # Retained views supply negative relevance only, never predecessor success.
        if not (raw_group or retained_group or retained_raw_group): continue
        operation_row = receipt.get("operation") if isinstance(receipt, dict) else None
        retained_ar = isinstance(receipt, dict) and (receipt.get("role") == "ar"
            or (isinstance(operation_row, dict) and operation_row.get("class") in
                {"ArchiverFirstAppend", "ArchiverRemainingAppend", "ArchiverIndex"}))
        if request["role"] != "ar" and not retained_ar: continue
        require(raw_group and request["role"] == "ar", "ForeignArchiveReceiptBinding")
        for leaf in ("request.json", "receipt.json"):
            path = call / leaf; regular(path)
            require(path.resolve() == path and path.stat().st_nlink == 1, "ForeignArchiveEvidenceAlias")
        keys(request, {"schema", "state", "lane", "role", "args_hex", "cwd_hex", "environment_hex", "owner_issued_inspector"})
        receipt = strict_json((call / "receipt.json").read_bytes())
        require(isinstance(receipt, dict), "ForeignArchiveReceiptFields")
        require(request["schema"] == NATIVE_SCHEMA and request["state"] == "RecordingOnly" and request["lane"] == FOREIGN_LANE
                and request["owner_issued_inspector"] is False and receipt["operation_id"] == call.name
                and all(receipt.get(k) == v for k, v in request.items()), "ForeignArchiveReceiptBinding")
        require(receipt.get("context") == context, "ForeignArchiveControl")
        require(receipt["protocol_state"] == "Completed" and receipt["failures"] == []
                and type(receipt["tool_result"]) is int, "ForeignArchiveSticky")
        require(isinstance(receipt.get("operation"), dict), "ForeignArchiveOperationFields")
        foreign_archive_environment(env, session, inv)
        require(foreign_context(env, cwd, session, inv) == context and receipt["context"] == context
                and all(receipt.get(k) == controls for k in ("controls_pre", "controls_post", "controls_return")), "ForeignArchiveControl")
        args = [os.fsdecode(bytes.fromhex(v)) for v in request["args_hex"]]
        operation = foreign_archive_classify(request["role"], args, context)
        require(operation == receipt["operation"], "ForeignArchiveClassification")
        require(receipt["tool_sha256"] == inv["generators"]["AR"]["sha256"]
                and receipt["argv_hex"] == [os.fsencode(inv["generators"]["AR"]["path"]).hex(), *request["args_hex"]], "ForeignArchiveTool")
        foreign_jobserver_binding(receipt["jobserver_identity"], env)
        require(receipt["jobserver_return"] == receipt["jobserver_identity"], "ForeignJobserverChanged")
        for stream in ("stdout", "stderr"):
            path = call / (stream + ".raw"); regular(path)
            require(path.resolve() == path and path.stat().st_nlink == 1
                    and file_hash(path) == receipt[stream + "_sha256"], "ForeignArchiveStream")
        require(foreign_semantics(call, receipt) == receipt["source_semantics"], "ForeignArchiveSemantics")
        pins = foreign_compile_pins(session, inv, context)
        require(all(receipt.get(k) == pins for k in ("compile_pins_pre", "compile_pins_post", "compile_pins_return")), "ForeignCompilePinChanged")
        specific = foreign_archive_producers(session, context, operation, current_call=call)
        require(receipt["family_pre"] == receipt["family_return"] == family
                and receipt["archive_member_producers_pre"] == receipt["archive_member_producers_return"] == specific,
                "ForeignArchiveMemberChanged")
        foreign_archive_states(call, receipt, operation)
        stage = foreign_archive_stage(operation)
        require(stage not in rows, "ForeignArchiveOnce"); rows[stage] = (call, receipt)
    chain = []
    stages = ("FirstD", "FirstFallbackCQ", "RingRemainingCQ" if Path(context["manifest"]).name == "ring" else "PSMIndexS")
    for stage in stages:
        if stage not in rows: continue
        call, receipt = rows[stage]; operation = receipt["operation"]
        foreign_archive_predecessor(operation, chain)
        require(receipt["archive_predecessor"] == (chain[-1][0].name if chain else None), "ForeignArchivePredecessor")
        if not chain: require(receipt["archive_pre"] == {"exists": False, "path": operation["archive"]}, "ForeignArchiveInitial")
        else: require(native_state_key(receipt["archive_pre"]) == native_state_key(chain[-1][1]["archive_post"]), "ForeignArchivePredecessor")
        expected = [{"operation_id": c.name, "request_sha256": file_hash(c / "request.json"),
                     "receipt_sha256": file_hash(c / "receipt.json")} for c, _ in chain]
        require(receipt.get("archive_history_pre") == receipt.get("archive_history_return") == expected, "ForeignArchivePredecessor")
        require(receipt.get("archive_operand_ledger_pre") == foreign_archive_ledger(chain)
                and receipt.get("archive_operand_ledger_return") == foreign_archive_ledger([*chain, (call, receipt)]), "ForeignArchiveLedger")
        chain.append((call, receipt))
    require(len(rows) == len(chain), "ForeignArchivePredecessor")
    if chain and live_archive: native_state_check(chain[-1][0], chain[-1][1]["archive_post"], live=True)
    return chain


def foreign_operation(session, policy, owner, role, args, cwd, env):
    namespace = session / "foreign-native-invocations"
    namespace.mkdir(mode=0o700, exist_ok=True)
    require(namespace.resolve() == namespace and not namespace.is_symlink(), "ForeignNamespaceAlias")
    call = namespace / uuid.uuid4().hex; call.mkdir(mode=0o700)
    request = {"schema": NATIVE_SCHEMA, "state": "RecordingOnly", "lane": FOREIGN_LANE,
               "role": role, "args_hex": [os.fsencode(a).hex() for a in args],
               "cwd_hex": os.fsencode(str(cwd)).hex(),
               "environment_hex": {os.fsencode(k).hex(): os.fsencode(v).hex() for k, v in env.items()},
               "owner_issued_inspector": False}
    atomic_json(call / "request.json", request)
    receipt = dict(request, operation_id=call.name, protocol_state="ProtocolRefused", tool_result=None, failures=[])
    try:
        inv = native_control(session, policy, owner)
        declaration = foreign_e_only_declaration(inv, cwd, args, env, session)
        if declaration is not None: receipt["e_only_input_declaration"] = declaration
        context = foreign_context(env, cwd, session, inv); receipt["context"] = context
        if Path(context["manifest"]).name in FOREIGN_E_ONLY_PACKAGES:
            require(role == "cc" and ((len(args) == 2 and args[:1] == ["-E"])
                    or (len(args) == 3 and args[:2] == ["-E", "--"])), "ForeignEOnlyArgv")
        archiving = role == "ar"; compiling = "-c" in args and not archiving
        if archiving:
            # Declarations are negative-only, retained before environment/pin refusal.
            receipt["compile_input_declaration"] = foreign_compile_inputs(inv, Path(context["manifest"]).name)
            names = RING_ARCHIVES if Path(context["manifest"]).name == "ring" else (PSM_ARCHIVE,)
            receipt["archive_output_declaration"] = [str(Path(context["out_dir"]) / n) for n in names]
        (foreign_archive_environment if archiving else foreign_compile_environment if compiling else foreign_environment)(env, session, inv)
        controls = foreign_controls(session, policy, owner, context); receipt["controls_pre"] = controls
        operation = (foreign_archive_classify(role, args, context) if archiving
                     else foreign_compile_classify(role, args, context, session, current_call=call) if compiling
                     else foreign_probe_classify(role, args, context, env, session, inv, current_call=call))
        receipt["operation"] = operation
        if compiling:
            # Retain expected input ownership before any pin validation may refuse.
            receipt["compile_input_declaration"] = foreign_compile_inputs(inv, Path(context["manifest"]).name)
            receipt["compile_pins_pre"] = foreign_compile_pins(session, inv, context)
            receipt["family_pre"] = foreign_compile_family(session, policy, owner, context, current_call=call)
        if archiving:
            receipt["compile_pins_pre"] = foreign_compile_pins(session, inv, context)
            family, members = foreign_archive_members(session, policy, owner, context, operation, current_call=call)
            receipt["family_pre"] = family; receipt["archive_member_producers_pre"] = members
            chain = foreign_archive_history(session, policy, owner, context, family, current_call=call, live_archive=True)
            foreign_archive_predecessor(operation, chain)
            receipt["archive_operand_ledger_pre"] = foreign_archive_ledger(chain)
            receipt["archive_predecessor"] = chain[-1][0].name if chain else None
            receipt["archive_history_pre"] = [{"operation_id": c.name, "request_sha256": file_hash(c / "request.json"),
                "receipt_sha256": file_hash(c / "receipt.json")} for c, _ in chain]
        tool_role = "AR" if archiving else "CC"
        tool = pinned_file(inv["generators"][tool_role])
        receipt["tool_sha256"] = inv["generators"][tool_role]["sha256"]
        receipt["argv_hex"] = [os.fsencode(v).hex() for v in [tool, *args]]
        if operation["class"] == "CompilerFamilyFileProbe":
            receipt["input_pre"] = native_snapshot(call, operation["source"], "input-pre.raw", bound=206)
            require(receipt["input_pre"]["length"] == 206 and receipt["input_pre"]["sha256"] == PROBE_DIGEST,
                    "ForeignProbeLiteral")
        if compiling:
            receipt["input_pre"] = native_snapshot(call, operation["source"], "input-pre.raw")
            receipt["output_pre"] = native_snapshot(call, operation["output"], "output-pre.raw", absent=True)
            require(not receipt["output_pre"]["exists"], "ForeignCompileOwnership")
        if archiving:
            receipt["archive_pre"] = native_snapshot(call, operation["archive"], "archive-pre.raw", absent=True)
            if operation["mode"] == "cqD": require(not receipt["archive_pre"]["exists"], "ForeignArchiveInitial")
            else: require(native_state_key(receipt["archive_pre"]) == native_state_key(chain[-1][1]["archive_post"]), "ForeignArchivePredecessor")
            receipt["archive_members_pre"] = []
            for index, (path, reference) in enumerate(zip(operation["members"], members)):
                state = native_snapshot(call, path, "archive-member-pre-" + str(index) + ".raw")
                receipt["archive_members_pre"].append(state)
                require(state["exists"] is True and state["sha256"] == reference["sha256"]
                        and state["length"] == reference["length"], "ForeignArchiveMemberChanged")
        fds = inherited_jobserver_fds(env)
        require(len(fds) == 2, "ForeignJobserverRequired")
        receipt["jobserver_identity"] = native_jobserver_identity(fds)
        code, faults = native_streamed([tool, *args], cwd, env, call, fds, {})
        receipt["tool_result"] = code; receipt["failures"].extend(faults)
        if compiling:
            # Capture even a failed compiler's partial output before input/control checks.
            receipt["output_post"] = native_snapshot(call, operation["output"], "output-post.raw", absent=True)
        if archiving:
            # Preserve partial archive and each observed member before any check.
            receipt["archive_post"] = native_snapshot(call, operation["archive"], "archive-post.raw", absent=True)
            receipt["archive_members_post"] = []
            for index, path in enumerate(operation["members"]):
                receipt["archive_members_post"].append(native_snapshot(call, path, "archive-member-post-" + str(index) + ".raw"))
            foreign_archive_states(call, receipt, operation, live_archive=True)
        if "input_pre" in receipt:
            receipt["input_post"] = native_snapshot(call, operation["source"], "input-post.raw", bound=receipt["input_pre"]["length"] if compiling else 206)
            require(native_state_key(receipt["input_pre"]) == native_state_key(receipt["input_post"]), "ForeignInputChanged")
        if compiling:
            require(code != 0 or (receipt["output_post"]["exists"] and receipt["output_post"]["length"] > 0), "ForeignCompileObjectMissing")
        post_context = foreign_context(env, cwd, session, inv)
        receipt["controls_post"] = foreign_controls(session, policy, owner, post_context)
        require(receipt["controls_post"] == controls, "ForeignControlChanged")
        if compiling or archiving:
            receipt["compile_pins_post"] = foreign_compile_pins(session, inv, post_context)
            require(receipt["compile_pins_post"] == receipt["compile_pins_pre"], "ForeignCompilePinChanged")
        require(not receipt["failures"], "ForeignCaptureSticky")
        receipt["source_semantics"] = foreign_semantics(call, receipt)
        receipt["protocol_state"] = "Completed"
    except (Refusal, OSError, ValueError, KeyError, TypeError) as error:
        receipt["failures"].append(str(error))
    for stream in ("stdout", "stderr"):
        path = call / (stream + ".raw")
        try:
            if path.is_file():
                receipt[stream + "_sha256"] = file_hash(path)
                with open(path, "rb") as source:
                    for block in iter(lambda: source.read(65536), b""):
                        view = memoryview(block)
                        while view:
                            written = os.write(1 if stream == "stdout" else 2, view)
                            require(written > 0, "ForeignForwardZero:" + stream)
                            view = view[written:]
                require(file_hash(path) == receipt[stream + "_sha256"], "ForeignStreamChanged")
        except (OSError, Refusal) as error:
            receipt["protocol_state"] = "ProtocolRefused"
            receipt["failures"].append("ForeignForward:" + stream + ":" + str(error))
    # A fault swallowed by cc stays in this receipt and in the aggregate seal.
    if receipt["protocol_state"] == "Completed":
        try:
            context = foreign_context(env, cwd, session, inv)
            receipt["controls_return"] = foreign_controls(session, policy, owner, context)
            require(receipt["controls_return"] == receipt["controls_pre"], "ForeignControlChanged")
            receipt["jobserver_return"] = native_jobserver_identity(fds)
            require(receipt["jobserver_return"] == receipt["jobserver_identity"], "ForeignJobserverChanged")
            if "input_post" in receipt: native_state_check(call, receipt["input_post"], live=True)
            if compiling:
                native_state_check(call, receipt["output_post"], live=True)
                receipt["compile_pins_return"] = foreign_compile_pins(session, inv, context)
                require(receipt["compile_pins_return"] == receipt["compile_pins_pre"], "ForeignCompilePinChanged")
                receipt["family_return"] = foreign_compile_family(session, policy, owner, context, current_call=call)
                require(receipt["family_return"] == receipt["family_pre"], "ForeignFamilyChanged")
            if archiving:
                foreign_archive_states(call, receipt, operation, live_archive=True)
                receipt["compile_pins_return"] = foreign_compile_pins(session, inv, context)
                require(receipt["compile_pins_return"] == receipt["compile_pins_pre"], "ForeignCompilePinChanged")
                family, members = foreign_archive_members(session, policy, owner, context, operation, current_call=call)
                receipt["family_return"] = family; receipt["archive_member_producers_return"] = members
                require(family == receipt["family_pre"] and members == receipt["archive_member_producers_pre"], "ForeignArchiveMemberChanged")
                chain = foreign_archive_history(session, policy, owner, context, family, current_call=call)
                receipt["archive_history_return"] = [{"operation_id": c.name, "request_sha256": file_hash(c / "request.json"),
                    "receipt_sha256": file_hash(c / "receipt.json")} for c, _ in chain]
                require(receipt["archive_history_return"] == receipt["archive_history_pre"], "ForeignArchivePredecessor")
                receipt["archive_operand_ledger_return"] = foreign_archive_ledger([*chain, (call, receipt)])
        except (Refusal, OSError, KeyError, TypeError, ValueError) as error:
            receipt["protocol_state"] = "ProtocolRefused"; receipt["failures"].append(str(error))
    atomic_json(call / "receipt.json", receipt)
    return call, receipt


def foreign_evidence_namespace(session):
    paths, probes, hashes, blockers = set(), set(), set(), []
    namespace = session / "foreign-native-invocations"
    if not namespace.exists() and not namespace.is_symlink(): return paths, probes, hashes, blockers
    if not (namespace.is_dir() and namespace.resolve() == namespace and not namespace.is_symlink()):
        return paths, probes, hashes, ["ForeignNamespaceAlias"]
    for call in namespace.iterdir():
        try:
            require(ID.fullmatch(call.name) and call.is_dir() and call.resolve() == call
                    and not call.is_symlink(), "ForeignCallAlias")
        except (Refusal, OSError, ValueError, KeyError, TypeError) as error:
            blockers.append("ForeignNamespace:" + call.name + ":" + str(error)); continue
        try:
            # Snapshot and stream bytes stay denied even if annotations vanish.
            # Empty hashes conservatively deny every 0B ordinary-file copy too.
            for snapshot in call.glob("*.raw"):
                paths.add(str(snapshot.resolve()))
                if snapshot.is_file() and not snapshot.is_symlink(): hashes.add(file_hash(snapshot))
        except (Refusal, OSError, ValueError, KeyError, TypeError) as error:
            blockers.append("ForeignNamespace:" + call.name + ":" + str(error))
        receipt = None
        try:
            receipt_path = call / "receipt.json"
            if receipt_path.is_file() and not receipt_path.is_symlink():
                receipt = strict_json(receipt_path.read_bytes())
                require(isinstance(receipt, dict), "ForeignSnapshotFields")
                # Retained stream ownership is independent of request validity.
                for stream in ("stdout", "stderr"):
                    sha = receipt.get(stream + "_sha256")
                    if isinstance(sha, str) and HEX.fullmatch(sha): hashes.add(sha)
                # The new negative-only declaration cannot lose later valid SHA
                # because an earlier member is malformed. Old compile maps stay unchanged.
                declaration = receipt.get("e_only_input_declaration")
                if isinstance(declaration, dict) and isinstance(declaration.get("source_sha256"), dict):
                    members = declaration["source_sha256"]
                    hashes.update(sha for sha in members.values() if isinstance(sha, str) and HEX.fullmatch(sha))
                    for member in members:
                        if isinstance(member, str):
                            try: paths.add(str((session / "vendor" / member).resolve()))
                            except (OSError, ValueError) as error:
                                blockers.append("ForeignNamespace:" + call.name + ":ForeignEOnlyDeclarationPath:" + type(error).__name__)
                # Native compile inputs/includes keep retained ownership before request parsing.
                for key in ("compile_input_declaration", "compile_pins_pre", "compile_pins_post", "compile_pins_return"):
                    pins = receipt.get(key)
                    if isinstance(pins, dict) and isinstance(pins.get("source_sha256"), dict):
                        for member, sha in pins["source_sha256"].items():
                            if isinstance(sha, str) and HEX.fullmatch(sha): hashes.add(sha)
                            if isinstance(member, str): paths.add(str((session / "vendor" / member).resolve()))
                for key in ("archive_members_pre", "archive_members_post", "archive_member_producers_pre", "archive_member_producers_return"):
                    states = receipt.get(key)
                    if isinstance(states, list):
                        for state in states:
                            if isinstance(state, dict):
                                sha = state.get("sha256")
                                if isinstance(sha, str) and HEX.fullmatch(sha): hashes.add(sha)
                                value = state.get("path")
                                if isinstance(value, str) and Path(value).is_absolute(): paths.add(str(Path(value).resolve()))
                declared_archives = receipt.get("archive_output_declaration")
                if isinstance(declared_archives, list):
                    paths.update(str(Path(value).resolve()) for value in declared_archives
                                 if isinstance(value, str) and Path(value).is_absolute())
                # Harvest every valid digest; one bad state cannot hide another.
                # Shape and cwd-dependent path checks remain in the request phase.
                for key in ("input_pre", "input_post", "output_pre", "output_post", "archive_pre", "archive_post"):
                    state = receipt.get(key)
                    if isinstance(state, dict):
                        sha = state.get("sha256")
                        if isinstance(sha, str) and HEX.fullmatch(sha): hashes.add(sha)
        except (Refusal, OSError, ValueError, KeyError, TypeError) as error:
            receipt = None
            blockers.append("ForeignNamespace:" + call.name + ":" + str(error))
        try:
            request = strict_json((call / "request.json").read_bytes())
            cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
            require(cwd.is_absolute(), "ForeignSourceAlias")
            args = [os.fsdecode(bytes.fromhex(v)) for v in request["args_hex"]]
            declared = []
            if args[:1] == ["-E"] and len(args) >= 2:
                declared.append(args[-1]); probes.add(str((cwd / args[-1]).resolve()))
            for index, arg in enumerate(args[:-1]):
                if arg in {"-o", "-c"}: declared.append(args[index + 1])
            if request["role"] == "ar" and len(args) >= 2: declared.extend(args[1:])
            paths.update(str((cwd / value).resolve()) for value in declared)
            # Old ring/psm namespace paths do not acquire this new policy/env read.
            selected = any(cwd == session / "vendor" / n for n in FOREIGN_E_ONLY_PACKAGES)
            flag_out = (cwd.name == "out" and cwd.parent.parent == session / "target" / TARGET / "debug/build"
                        and re.fullmatch(r"zstd-sys-[0-9a-f]{16}", cwd.parent.name))
            if selected or flag_out:
                inv = strict_json(POLICY.read_bytes())["inventory"]
                declaration = foreign_e_only_declaration(inv, cwd, args,
                    {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}, session)
                if declaration is not None:
                    for member, sha in declaration["source_sha256"].items():
                        paths.add(str((session / "vendor" / member).resolve())); hashes.add(sha)
            # A receiptless fixed Compile request owns its closed inputs negatively.
            # Raw cwd/source and pinned package identity suffice for declaration only;
            # labels, environment, flags and receipts cannot grant execution authority.
            if args.count("-c") == 1:
                name = next((n for n in FOREIGN_COMPILE_SOURCES
                             if cwd == session / "vendor" / n and cwd.resolve() == cwd
                             and not cwd.is_symlink()), None)
                index = args.index("-c")
                if name is not None and index + 1 < len(args) and any(
                        args[index + 1] == (str(cwd / m) if name == "ring" else m)
                        for m in FOREIGN_COMPILE_SOURCES[name]):
                    inv = strict_json(POLICY.read_bytes())["inventory"]
                    package = {"id": RING_PACKAGE if name == "ring" else PSM_PACKAGE,
                               "tree": "vendor", "manifest": name + "/Cargo.toml"}
                    require(inv["packages"].count(package) == 1, "ForeignCompileDeclarationPackage")
                    inputs = foreign_compile_inputs(inv, name)["source_sha256"]
                    for member, sha in inputs.items():
                        paths.add(str((session / "vendor" / member).resolve()))
                        if isinstance(sha, str) and HEX.fullmatch(sha): hashes.add(sha)
            # Source-first declaration also closes receiptless/refused AR inputs;
            # package cwd and finite canonical OUT shape grant only negative ownership.
            if request["role"] == "ar":
                name = next((n for n in FOREIGN_PACKAGES if cwd == session / "vendor" / n
                             and cwd.resolve() == cwd and not cwd.is_symlink()), None)
                env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
                out = Path(env.get("OUT_DIR", ""))
                if name is not None and out.is_absolute() and out.resolve() == out and not out.is_symlink() and out.name == "out" and out.parent.parent == session / "target" / TARGET / "debug/build" and re.fullmatch(re.escape(name) + r"-[0-9a-f]{16}", out.parent.name):
                    inv = strict_json(POLICY.read_bytes())["inventory"]
                    package = {"id": RING_PACKAGE if name == "ring" else PSM_PACKAGE, "tree": "vendor", "manifest": name + "/Cargo.toml"}
                    require(inv["packages"].count(package) == 1, "ForeignArchiveDeclarationPackage")
                    for member, sha in foreign_compile_inputs(inv, name)["source_sha256"].items():
                        paths.add(str((session / "vendor" / member).resolve()))
                        if isinstance(sha, str) and HEX.fullmatch(sha): hashes.add(sha)
                    names = RING_ARCHIVES if name == "ring" else (PSM_ARCHIVE,)
                    paths.update(str(out / n) for n in (*names, *FOREIGN_ARCHIVE_FIRST_MEMBERS[name],
                                 *(FOREIGN_ARCHIVE_REMAINING_MEMBERS if name == "ring" else ())))
            if receipt is not None:
                for key in ("input_pre", "input_post", "output_pre", "output_post", "archive_pre", "archive_post"):
                    state = receipt.get(key, {})
                    require(isinstance(state, dict), "ForeignSnapshotFields")
                    if "path" in state: paths.add(str((cwd / state["path"]).resolve()))
                    if "snapshot" in state: paths.add(str((call / state["snapshot"]).resolve()))
        except (Refusal, OSError, ValueError, KeyError, TypeError) as error:
            blockers.append("ForeignNamespace:" + call.name + ":" + str(error))
    for value in paths:
        path = Path(value)
        if path.is_file() and not path.is_symlink(): hashes.add(file_hash(path))
    return paths, probes, hashes, blockers


def foreign_owned(value, cwd, paths, hashes):
    path = (Path(cwd) / value).resolve()
    return str(path) in paths or (bool(hashes) and path.is_file() and not path.is_symlink()
                                  and file_hash(path) in hashes)


def foreign_borrowed_cc_literal(value, receipt, session, inv, artifacts):
    """Root's sole provenance exception: cc's pinned include_bytes dep-info edge."""
    try:
        literal = session / "vendor" / PROBE_LITERAL
        require(os.path.join(receipt["cwd"], value) == str(literal), "ForeignBorrowedLiteralPath")
        regular(literal)
        require(literal.resolve() == literal and literal.stat().st_nlink == 1
                and file_hash(literal) == PROBE_DIGEST == inv["vendor"]["files"][PROBE_LITERAL], "ForeignBorrowedLiteralPin")
        call = session / "invocations" / receipt["invocation_id"]
        request = strict_json((call / "request.json").read_bytes())
        initial = strict_json((call / "invocation.json").read_bytes())
        raw = [os.fsdecode(bytes.fromhex(v)) for v in request["argv_hex"]]
        env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
        cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
        root = session / "vendor/cc"; source = root / "src/lib.rs"
        package = {"id": CC_PACKAGE, "tree": "vendor", "manifest": "cc/Cargo.toml"}
        require(inv["packages"].count(package) == 1 and cwd == root and cwd.resolve() == cwd
                and env.get("CARGO_MANIFEST_DIR") == str(root) and env.get("CARGO_MANIFEST_PATH") == str(root / "Cargo.toml")
                and env.get("CARGO_PKG_NAME") == "cc" and env.get("CARGO_PKG_VERSION") == "1.2.59"
                and raw[0] == pinned_file(inv["rustc"]), "ForeignBorrowedCcOrigin")
        parsed = parse_rustc(raw[1:]); context = invocation_context(raw, parsed, env, cwd, session, inv)
        compiler_environment(env, session, inv["sysroot"], probe=parsed["probe"], context=context)
        require(not parsed["probe"] and "--target" not in parsed["options"]
                and parsed["inputs"] and len(parsed["inputs"]) == 1 and cwd / parsed["inputs"][0] == source
                and parsed["options"].get("--crate-name") == ["cc"]
                and parsed["options"].get("--crate-type") == ["lib"]
                and context == {"kind": "DirectCargoCompile"}, "ForeignBorrowedCcCompile")
        regular(source)
        require(source.resolve() == source and source.stat().st_nlink == 1
                and file_hash(source) == inv["vendor"]["files"]["cc/src/lib.rs"], "ForeignBorrowedCcSource")
        fields = ("argv_hex", "environment_hex", "cwd", "parsed", "context", "package", "source",
                  "role", "kind", "compiler_sha256", "declared_outputs")
        require(all(initial[k] == receipt[k] for k in fields)
                and receipt["argv_hex"] == request["argv_hex"]
                and receipt["environment_hex"] == request["environment_hex"] and receipt["cwd"] == str(cwd)
                and receipt["parsed"] == parsed and receipt["context"] == context
                and receipt["package"] == package and receipt["source"] == str(source)
                and receipt["role"] == "Host" and receipt["kind"] == "Compile"
                and receipt["compiler_sha256"] == inv["rustc"]["sha256"]
                and receipt["declared_outputs"] == selected_outputs(parsed, cwd, session / "target")
                and receipt["exit_code"] == 0 and receipt["blockers"] == [], "ForeignBorrowedCcBinding")
        matches = [a for a in artifacts if a["invocation_id"] == receipt["invocation_id"]
                   and a["package_id"] == CC_PACKAGE and a["target"].get("kind") == ["lib"]
                   and a["target"].get("crate_types") == ["lib"] and a["target"].get("name") == "cc"
                   and a["target"].get("src_path") == str(source)]
        require(len(matches) == 1, "ForeignBorrowedCcArtifact")
        return True
    except (Refusal, OSError, KeyError, TypeError, ValueError, IndexError):
        return False


def foreign_jobserver_binding(identity, env):
    match = re.fullmatch(r"-j --jobserver-fds=([1-9][0-9]*),([1-9][0-9]*) --jobserver-auth=\1,\2",
                         env.get("CARGO_MAKEFLAGS", ""))
    require(match is not None and "MAKEFLAGS" not in env and "MFLAGS" not in env, "ForeignJobserverRequired")
    require(identity["state"] == "RecordingOnly" and identity["platform"] == "darwin"
            and identity["library"] == "/usr/lib/libproc.dylib" and identity["symbol"] == "proc_pidfdinfo"
            and identity["flavor"] == 6 and identity["buffer_bytes"] == 184
            and identity["pipe_info_offset"] == 24 and identity["handle_offset"] == 160
            and identity["peer_offset"] == 168 and type(identity["pid"]) is int and identity["pid"] > 0,
            "ForeignJobserverBinding")
    ends = identity["endpoints"]; fds = [int(v) for v in match.groups()]
    require(len(ends) == 2 and fds[0] != fds[1]
            and all(3 <= fd <= 2**31 - 1 for fd in fds), "ForeignJobserverBinding")
    for end, fd, access in zip(ends, fds, (os.O_RDONLY, os.O_WRONLY)):
        require(end["fd"] == fd and end["access"] == access
                and end["flags"] & os.O_ACCMODE == access and end["file_type"] == stat.S_IFIFO
                and end["returned_bytes"] == 184 and end["handle"] == end["inode"]
                and end["handle"] != 0 and end["peer"] != 0, "ForeignJobserverBinding")
    require(ends[0]["handle"] == ends[1]["peer"] and ends[1]["handle"] == ends[0]["peer"]
            and ends[0]["handle"] != ends[1]["handle"], "ForeignJobserverBinding")


def foreign_seal(session, policy):
    namespace = session / "foreign-native-invocations"
    owner = strict_json((session / "owner.json").read_bytes()); inv = native_control(session, policy, owner)
    result = {"schema": NATIVE_SCHEMA, "state": "RecordingOnly", "lane": FOREIGN_LANE,
              "operations": [], "context_groups": [], "blockers": [],
              "stage": "StageAIncomplete", "native_producer_qualification": "not_issued",
              "artifact_selection": "not_observed",
              "unclosed": ["compiler-family-successors", "family-to-compile", "object", "archive", "builder-run", "consumer"]}
    paths, probes, hashes, failures = foreign_evidence_namespace(session)
    calls = {}; groups = {}; outdirs = {}; compile_requested = False; archive_requested = False
    if namespace.exists() and namespace.is_dir() and namespace.resolve() == namespace and not namespace.is_symlink():
        for call in sorted(namespace.iterdir()):
            item = {"operation_id": call.name}; result["operations"].append(item)
            try:
                item["retained_raw_sha256"] = {}
                for raw in call.glob("*.raw"):
                    regular(raw)
                    require(raw.resolve() == raw and raw.stat().st_nlink == 1, "ForeignEvidenceAlias")
                    item["retained_raw_sha256"][raw.name] = file_hash(raw)
                early_request = strict_json((call / "request.json").read_bytes())
                compile_requested = compile_requested or any(os.fsdecode(bytes.fromhex(v)) == "-c" for v in early_request["args_hex"])
                archive_requested = archive_requested or early_request.get("role") == "ar"
                for leaf in ("request", "receipt"):
                    path = call / (leaf + ".json"); regular(path)
                    require(path.resolve() == path and path.stat().st_nlink == 1, "ForeignEvidenceAlias")
                    item[leaf + "_sha256"] = file_hash(path)
                request = strict_json((call / "request.json").read_bytes()); receipt = strict_json((call / "receipt.json").read_bytes())
                keys(request, {"schema", "state", "lane", "role", "args_hex", "cwd_hex", "environment_hex", "owner_issued_inspector"})
                require(request["schema"] == NATIVE_SCHEMA and request["state"] == "RecordingOnly"
                        and request["lane"] == FOREIGN_LANE and request["owner_issued_inspector"] is False
                        and receipt["operation_id"] == call.name and all(receipt[k] == v for k, v in request.items()), "ForeignReceiptBinding")
                item["protocol_state"] = receipt["protocol_state"]; item["tool_result"] = receipt["tool_result"]
                compile_requested = compile_requested or any(os.fsdecode(bytes.fromhex(v)) == "-c" for v in request["args_hex"])
                require(receipt["protocol_state"] == "Completed" and receipt["failures"] == [], "ForeignProtocolSticky")
                env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
                cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"]))); args = [os.fsdecode(bytes.fromhex(v)) for v in request["args_hex"]]
                archiving = request["role"] == "ar"; archive_requested = archive_requested or archiving
                compiling = "-c" in args and not archiving; compile_requested = compile_requested or compiling
                context = foreign_context(env, cwd, session, inv)
                (foreign_archive_environment if archiving else foreign_compile_environment if compiling else foreign_environment)(env, session, inv)
                group = context["out_dir"]; package = context["package_id"]
                require(group not in outdirs or outdirs[group] == package, "ForeignOutDirCollision"); outdirs[group] = package
                controls = foreign_controls(session, policy, owner, context)
                require(receipt["context"] == context and all(receipt.get(k) == controls
                        for k in ("controls_pre", "controls_post", "controls_return")), "ForeignControlChanged")
                tool_role = "AR" if archiving else "CC"
                require(receipt["tool_sha256"] == inv["generators"][tool_role]["sha256"]
                        and receipt["argv_hex"] == [os.fsencode(inv["generators"][tool_role]["path"]).hex(), *request["args_hex"]], "ForeignToolBinding")
                foreign_jobserver_binding(receipt["jobserver_identity"], env)
                require(receipt["jobserver_return"] == receipt["jobserver_identity"], "ForeignJobserverChanged")
                for stream in ("stdout", "stderr"):
                    path = call / (stream + ".raw"); regular(path)
                    require(path.resolve() == path and path.stat().st_nlink == 1
                            and file_hash(path) == receipt[stream + "_sha256"], "ForeignStreamChanged")
                require(foreign_semantics(call, receipt) == receipt["source_semantics"], "ForeignSourceSemantics")
                calls[call.name] = (call, receipt, env, args)
                groups.setdefault(group, []).append(call.name)
            except (Refusal, OSError, KeyError, TypeError, ValueError) as error:
                failures.append("ForeignOperation:" + call.name + ":" + str(error))
        for ident, (call, receipt, env, args) in calls.items():
            try:
                operation = receipt["operation"]; predecessor = operation.get("predecessor")
                history = []
                if predecessor is not None:
                    require(predecessor in calls, "ForeignProbePredecessor")
                    history = [calls[predecessor][:2]]
                compiling = operation["class"] == "CompilerObjectCompile"
                archiving = operation["class"] in {"ArchiverFirstAppend", "ArchiverRemainingAppend", "ArchiverIndex"}
                computed = (foreign_archive_classify(receipt["role"], args, receipt["context"]) if archiving
                            else foreign_compile_classify(receipt["role"], args, receipt["context"], session, current_call=call) if compiling
                            else foreign_probe_classify(receipt["role"], args, receipt["context"], env, session, inv, history=history))
                require(computed == operation, "ForeignRawClassification")
                if operation["class"] == "CompilerFamilyFileProbe":
                    require(isinstance(receipt.get("input_pre"), dict)
                            and isinstance(receipt.get("input_post"), dict), "ForeignSnapshotFields")
                    require(native_state_key(receipt["input_pre"]) == native_state_key(receipt["input_post"])
                            and receipt["input_pre"]["exists"] is True
                            and receipt["input_pre"]["path"] == receipt["input_post"]["path"] == operation["source"]
                            and receipt["input_pre"]["length"] == 206
                            and receipt["input_pre"]["sha256"] == PROBE_DIGEST, "ForeignInputChanged")
                    native_state_check(call, receipt["input_pre"])
                    item = next(i for i in result["operations"] if i["operation_id"] == ident)
                    item["input_final_state"] = native_state_check(call, receipt["input_post"], live=True, retire=True)
                elif compiling:
                    pins = foreign_compile_pins(session, inv, receipt["context"])
                    require(all(receipt.get(k) == pins for k in ("compile_pins_pre", "compile_pins_post", "compile_pins_return")), "ForeignCompilePinChanged")
                    # Sealing validates ALL published AR rows independently below,
                    # so its family scan is explicit and independent of UUID/request order.
                    family = foreign_compile_family(session, policy, owner, receipt["context"], current_call=call, allow_archive=True)
                    require(receipt["family_pre"] == receipt["family_return"] == family, "ForeignFamilyChanged")
                    require(all(isinstance(receipt.get(k), dict) for k in ("input_pre", "input_post", "output_pre", "output_post")), "ForeignSnapshotFields")
                    require(native_state_key(receipt["input_pre"]) == native_state_key(receipt["input_post"])
                            and receipt["input_pre"]["exists"] is True
                            and receipt["input_pre"]["path"] == receipt["input_post"]["path"] == operation["source"]
                            and receipt["input_pre"]["sha256"] == pins["source_sha256"][str(Path(operation["source"]).relative_to(session / "vendor"))]
                            and receipt["output_pre"] == {"exists": False, "path": operation["output"]}
                            and receipt["output_post"]["path"] == operation["output"], "ForeignCompileSnapshot")
                    native_state_check(call, receipt["input_pre"]); native_state_check(call, receipt["input_post"], live=True)
                    native_state_check(call, receipt["output_post"], live=True)
                    require(receipt["tool_result"] == 0 and receipt["output_post"]["exists"] is True
                            and receipt["output_post"]["length"] > 0, "ForeignCompileNonzeroOrMissing")
                elif archiving:
                    family, members = foreign_archive_members(session, policy, owner, receipt["context"], operation, current_call=call)
                    chain = foreign_archive_history(session, policy, owner, receipt["context"], family, live_archive=True)
                    require(any(c.name == ident for c, _ in chain), "ForeignArchivePredecessor")
                    if chain[-1][1]["tool_result"] != 0: failures.append("ForeignArchiveUnresolvedNonzero:" + chain[-1][0].name)
                else:
                    require(not any(k in receipt for k in ("input_pre", "input_post", "output_pre", "output_post",
                                                           "archive_pre", "archive_post")), "ForeignOutputFreeClass")
            except (Refusal, OSError, KeyError, TypeError, ValueError) as error:
                failures.append("ForeignOperation:" + ident + ":" + str(error))
        sources = {}
        for ident, (_, receipt, _, _) in calls.items():
            operation = receipt["operation"]
            if operation["class"] == "CompilerFamilyFileProbe": sources.setdefault(operation["source"], []).append((ident, operation))
        for source, rows in sources.items():
            first = [i for i, op in rows if not op["retry"]]
            if len(first) != 1 or len(rows) > 2 or any(op["retry"] and op["predecessor"] != first[0] for _, op in rows):
                failures.append("ForeignProbeHistory:" + source)
    for out, members in sorted(groups.items()):
        observed = {calls[i][1]["operation"]["class"] for i in members}
        missing = sorted({"CompilerFamilyFileProbe", "CompilerFamilyHelpProbe", "CompilerFamilyVersionProbe"} - observed)
        result["context_groups"].append({"out_dir": out, "package_id": outdirs[out],
            "operations": sorted(members), "per_probe_family_predecessor": "not_observed",
            "observed_classes": sorted(observed), "missing_observation_classes": missing,
            "family_pairing": "ambiguous_context_group", "family_qualification": "not_issued"})
    if compile_requested:
        result["stage"] = "StageBIncomplete"
        result["unclosed"] = ["archive", "builder-run", "consumer"]
    if archive_requested:
        result["stage"] = "StageCIncomplete"
        result["unclosed"] = ["archive-chain/index", "builder-run", "consumer"]
    if result["operations"]:
        failures.extend("ForeignSeal:" + result["stage"] + ":" + missing for missing in result["unclosed"])
    result["quarantine"] = {"paths": sorted(paths), "probe_paths": sorted(probes), "sha256": sorted(hashes)}
    result["blockers"] = sorted(set(failures)); native_control(session, policy, owner)
    atomic_json(session / "foreign-native-record.json", result)
    return result


def native_wrapper(session_id, role, args):
    require(ID.fullmatch(session_id),"SessionId")
    session=SESSIONS/("pending-"+session_id)
    require(session.is_dir() and session.resolve()==session and not session.is_symlink(),"MissingSession")
    policy=strict_json(POLICY.read_bytes());owner=strict_json((session/"owner.json").read_bytes())
    cwd = Path.cwd(); env = dict(os.environ)
    # Only the actual canonical SQLite tree uses its unchanged native graph.
    operation = native_operation if cwd == session / "vendor/libsqlite3-sys" else foreign_operation
    call,r=operation(session,policy,owner,role,args,cwd,env)
    if r["protocol_state"]!="Completed":
        raise Refusal("NativeProtocolRefused:"+call.name+":"+r["failures"][0])
    code=r["tool_result"]
    return code if code>=0 else 128-code


def sqlite_rust_context(args, env, cwd, session, inv):
    raw=args[1:]
    if env.get("CARGO_PKG_NAME")!="libsqlite3-sys":return None
    native=any(a.startswith("-l") for a in raw)
    if not native:
        parsed=parse_rustc(raw)
        if parsed["probe"] or parsed["options"].get("--target")!=[TARGET] or parsed["options"].get("--crate-type")!=["lib"]:
            return None
    require(raw[-2:]==["-l","static=sqlite3"] and sum(a.startswith("-l") for a in raw)==1,"SqliteStaticTemplate")
    parsed=parse_rustc(raw[:-2]);o=parsed["options"]
    package,root,out=fixed_source_package("libsqlite3-sys","0.28.0",cwd,env,session,inv,SQLITE_MEMBERS)
    require(package["id"]==SQLITE_PACKAGE and not parsed["probe"] and parsed["inputs"]==[str(root/"src/lib.rs")]
            and o.get("--crate-name")==["libsqlite3_sys"] and o.get("--crate-type")==["lib"]
            and o.get("--target")==[TARGET] and o.get("--out-dir")==[str(session/"target"/TARGET/"debug/deps")]
            and o.get("--emit")==["dep-info,metadata,link"]
            and sorted(o.get("--cfg",[]))==sorted('feature="'+f+'"' for f in SQLITE_FEATURES)
            and [v for v in o.get("-L",[]) if v.startswith("native=")]==["native="+str(out)]
            and all(v in {"native="+str(out),"dependency="+str(session/"target"/TARGET/"debug/deps"),
                         "dependency="+str(session/"target/debug/deps")} for v in o.get("-L",[]))
            and "--test" not in o and not any(k in parsed["codegen"] for k in ("link-arg","linker")),"SqliteRustContext")
    require(env.get("CC")==str(session/"native-cc") and env.get("AR")==str(session/"native-ar")
            and env.get("CARGO_MANIFEST_PATH")==str(root/"Cargo.toml"),"SqliteRustEnvironment")
    o["-l"]=["static=sqlite3"]
    return parsed,{"kind":"DirectCargoCompile","sqlite_static_declaration":"static=sqlite3",
                   "package_id":SQLITE_PACKAGE,"manifest":str(root),"out_dir":str(out)}


def native_evidence_paths(session):
    """Quarantine declared native identities, including retired/request-only probes.

    This collection denies ordinary ownership; it does not validate or grant a
    native producer. Native seal separately validates every request/receipt.
    """
    paths, probes, blockers = set(), set(), []
    for call in (session / "native-invocations").iterdir():
        try:
            request = strict_json((call / "request.json").read_bytes())
            cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
            require(cwd.is_absolute(), "NativePathAlias")
            args = [os.fsdecode(bytes.fromhex(v)) for v in request["args_hex"]]
            declared = []
            if request["role"] == "cc" and args[:1] == ["-E"] and len(args) in (2, 3):
                value = str((cwd / args[-1]).resolve())
                probes.add(value); declared.append(value)
            elif request["role"] == "cc":
                declared.extend(args[i + 1] for i, arg in enumerate(args[:-1]) if arg == "-o")
            elif request["role"] == "ar" and args:
                if args[0] in {"cqD", "cq"}: declared.extend(args[1:])
                elif args[0] in {"sD", "s", "t", "p"} and len(args) >= 2: declared.append(args[1])
            paths.update(str((cwd / value).resolve()) for value in declared)
            receipt_path = call / "receipt.json"
            if receipt_path.is_file():
                receipt = strict_json(receipt_path.read_bytes())
                for key in ("input_pre", "input_post", "output_pre", "output_post", "archive_pre", "archive_post"):
                    state = receipt.get(key, {})
                    if "snapshot" in state: paths.add(str((call / state["snapshot"]).resolve()))
        except (Refusal, OSError, KeyError, TypeError, ValueError) as error:
            blockers.append("NativeNamespace:" + call.name + ":" + str(error))
    foreign_paths, foreign_probes, _, foreign_blockers = foreign_evidence_namespace(session)
    paths.update(foreign_paths); probes.update(foreign_probes); blockers.extend(foreign_blockers)
    return paths, probes, blockers


def native_path_identity(value, receipt):
    return str((Path(receipt["cwd"]) / value).resolve())


def native_seal(session, policy, rust_receipts, associations, artifacts, events, edges):
    owner=strict_json((session/"owner.json").read_bytes());inv=native_control(session,policy,owner)
    calls={};failures=[];result={"schema":NATIVE_SCHEMA,"state":"RecordingOnly","profile":BUNDLED_PROFILE,
                               "operations":[],"blockers":[],"artifact_selection":"not_observed"}
    for call in (session/"native-invocations").iterdir():
        try:
            regular(call/"request.json");regular(call/"receipt.json")
            request=strict_json((call/"request.json").read_bytes());r=strict_json((call/"receipt.json").read_bytes())
            require(r["operation_id"]==call.name and all(r[k]==v for k,v in request.items()),"NativeReceiptBinding")
            require(r["protocol_state"]=="Completed" and not r["failures"],"NativeProtocolSticky")
            require(not request["owner_issued_inspector"],"NativeUnexpectedInspector")
            env={os.fsdecode(bytes.fromhex(k)):os.fsdecode(bytes.fromhex(v)) for k,v in request["environment_hex"].items()}
            cwd=Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
            context=native_context(env,cwd,session,inv);native_environment(env,session,inv)
            require(context==r["context"] and r["tool_sha256"]==inv["generators"][r["role"].upper()]["sha256"],"NativeReceiptBinding")
            require(r["argv_hex"]==[os.fsencode(inv["generators"][r["role"].upper()]["path"]).hex(),*r["args_hex"]],"NativeReceiptBinding")
            for stream in ("stdout","stderr"):
                require(file_hash(call/(stream+".raw"))==r[stream+"_sha256"],"NativeStreamChanged")
            for key in ("input_pre","input_post","output_pre","output_post","archive_pre","archive_post"):
                if key in r:native_state_check(call,r[key])
            calls[call.name]=(call,r,env,cwd)
        except (Refusal,OSError,KeyError,ValueError,TypeError) as error:
            failures.append("NativeOperation:"+call.name+":"+str(error))
    try:
        require(not failures,"NativeIncompleteOrProtocolRefused")
        require(calls,"NativeMissingOperations")
        contexts={json.dumps(r["context"],sort_keys=True) for _,r,_,_ in calls.values()}
        require(len(contexts)==1,"NativeOriginAmbiguous")
        context=next(iter(calls.values()))[1]["context"];out=Path(context["out_dir"])
        objects=[v for v in calls.values() if v[1]["operation"]["class"]=="SqliteObjectCompile"]
        require(len(objects)==1 and objects[0][1]["tool_result"]==0,"NativeObjectProducer")
        oc,obj,_,_=objects[0];native_state_check(oc,obj["output_post"],live=True)
        mutators={k:v for k,v in calls.items() if v[1]["operation"]["class"] in {"ArchiveAppend","ArchiveIndex"}}
        predecessors={v[1]["operation"]["predecessor"] for v in mutators.values()}
        tails=[k for k in mutators if k not in predecessors]
        require(len(tails)==1,"NativeArchiveChain")
        chain=[];cursor=tails[0]
        while cursor is not None:
            require(cursor in mutators and cursor not in chain,"NativeArchiveChain")
            chain.append(cursor);cursor=mutators[cursor][1]["operation"]["predecessor"]
        chain.reverse();require(set(chain)==set(mutators) and len(chain) in (2,3),"NativeArchiveChain")
        history=[(oc,obj)]
        for ident in chain:
            call,r,env,cwd=calls[ident]
            args=[os.fsdecode(bytes.fromhex(v)) for v in r["args_hex"]]
            computed=native_classify(r["role"],args,context,env,session,inv,history=history)
            require(computed==r["operation"],"NativeArchiveChain")
            previous=r["operation"]["predecessor"]
            if previous is None:require(not r["archive_pre"]["exists"],"NativeArchiveInitial")
            else:require(native_state_key(r["archive_pre"])==native_state_key(calls[previous][1]["archive_post"]),"NativeArchivePredecessor")
            history.append((call,r))
        final_call,final,final_env,_=calls[chain[-1]]
        require(final["operation"]["class"]=="ArchiveIndex" and final["tool_result"]==0,"NativeArchiveIndex")
        for ident,(call,r,env,cwd) in calls.items():
            kind=r["operation"]["class"]
            if kind in {"ArchiveAppend","ArchiveIndex"}:continue
            require(kind in {"SqliteObjectCompile","CompilerFamilyFileProbe"},"NativeOperationClass")
            preceding=[];previous=r["operation"].get("predecessor")
            if previous is not None:
                require(previous in calls,"NativeProbePredecessor")
                preceding=[calls[previous][:2]]
            computed=native_classify(r["role"],[os.fsdecode(bytes.fromhex(v)) for v in r["args_hex"]],context,env,session,inv,history=preceding)
            require(computed==r["operation"] and native_state_key(r["input_pre"])==native_state_key(r["input_post"]),"NativeInputChanged")
            if kind=="CompilerFamilyFileProbe":
                require(r["input_pre"]["length"]==206 and r["input_pre"]["sha256"]==PROBE_DIGEST,"NativeProbeLiteral")
                r["input_final_state"]=native_state_check(call,r["input_post"],live=True,retire=True)
            else:
                require(r["input_pre"]["sha256"]==inv["vendor"]["files"]["libsqlite3-sys/sqlite3/sqlite3.c"],"NativeCompileSource")
                native_state_check(call,r["input_post"],live=True)
        probes=[(c,r) for c,r,_,_ in calls.values() if r["operation"]["class"]=="CompilerFamilyFileProbe"]
        for source in {r["operation"]["source"] for _,r in probes}:
            group=[(c,r) for c,r in probes if r["operation"]["source"]==source]
            first=[c.name for c,r in group if not r["operation"]["retry"]]
            require(len(first)==1 and len(group)<=2 and all(not r["operation"]["retry"]
                    or r["operation"]["predecessor"]==first[0] for _,r in group),"NativeProbePredecessor")
        origin=[a for a in associations if a["package_id"]==SQLITE_PACKAGE and a["out_dir"]==str(out)]
        executed=[e for e in events if e["reason"]=="build-script-executed" and e.get("package_id")==SQLITE_PACKAGE]
        require(len(origin)==len(executed)==1 and origin[0]["cargo_event"]==executed[0],"NativeCargoOrigin")
        by_id={r["invocation_id"]:r for r in rust_receipts};builder=by_id[origin[0]["producer_invocation"]]
        require(builder["role"]=="Host" and builder["kind"]=="Compile" and builder["exit_code"]==0
                and not builder["blockers"] and builder["source"]==str(Path(context["manifest"])/"build.rs")
                and sorted(builder["parsed"]["options"].get("--cfg",[]))==sorted('feature="'+f+'"' for f in SQLITE_FEATURES),"NativeBuilder")
        builder_events=[e for e in events if e["reason"]=="compiler-artifact" and e.get("package_id")==SQLITE_PACKAGE
                        and e.get("target",{}).get("kind")==["custom-build"]]
        require(len(builder_events)==1 and sorted(builder_events[0].get("features",[]))==sorted(SQLITE_FEATURES),"NativeBuilder")
        cc=[e for e in edges if e["consumer"]==builder["invocation_id"] and e["name"]=="cc"]
        require(len(cc)==1 and len(cc[0]["producers"])==1,"NativeCcProducer")
        cp=by_id[cc[0]["producers"][0]]
        require(cp["package"]["id"]==CC_PACKAGE and cp["role"]=="Host" and cp["exit_code"]==0 and not cp["blockers"],"NativeCcProducer")
        binding=out/"bindgen.rs";regular(binding)
        require(file_hash(binding)==inv["vendor"]["files"]["libsqlite3-sys/sqlite3/bindgen_bundled_version.rs"]
                and origin[0]["generated_files"].get("bindgen.rs")==file_hash(binding),"NativeBindings")
        event=executed[0]
        require(event["linked_libs"]==["static=sqlite3"] and event["linked_paths"]==["native="+str(out)],"NativeLinkDeclaration")
        consumers=[r for r in rust_receipts if r.get("package",{}).get("id")==SQLITE_PACKAGE and r["role"]=="Target" and r["kind"]=="Compile"]
        require(len(consumers)==1,"NativeRustConsumer")
        consumer=consumers[0];rc=session/"invocations"/consumer["invocation_id"]
        raw=strict_json((rc/"request.json").read_bytes());initial=strict_json((rc/"invocation.json").read_bytes())
        raw_env={os.fsdecode(bytes.fromhex(k)):os.fsdecode(bytes.fromhex(v)) for k,v in raw["environment_hex"].items()}
        special=sqlite_rust_context([os.fsdecode(bytes.fromhex(v)) for v in raw["argv_hex"]],raw_env,
                 Path(os.fsdecode(bytes.fromhex(raw["cwd_hex"]))),session,inv)
        require(special is not None and special[0]==consumer["parsed"] and special[1]==consumer["context"]
                and all(initial[k]==consumer[k] for k in ("context","parsed","argv_hex","environment_hex","source","package","role","kind"))
                and raw["argv_hex"]==consumer["argv_hex"] and raw["environment_hex"]==consumer["environment_hex"]
                and consumer["exit_code"]==0 and not consumer["blockers"],"NativeRustConsumer")
        ca=[a for a in artifacts if a["invocation_id"]==consumer["invocation_id"]]
        ce=[e for e in events if e["reason"]=="compiler-artifact" and e.get("package_id")==SQLITE_PACKAGE and e.get("target",{}).get("kind")==["lib"]]
        require(len(ca)==len(ce)==1 and sorted(ce[0].get("features",[]))==sorted(SQLITE_FEATURES)
                and ce[0]["target"].get("src_path")==consumer["source"] and ca[0]["files"]==ce[0]["filenames"],"NativeRustConsumer")
        for driver in ("registry+https://github.com/rust-lang/crates.io-index#diesel@2.3.7",
                       "registry+https://github.com/rust-lang/crates.io-index#rusqlite@0.31.0"):
            joined = [e for e in edges if e["producers"] == [consumer["invocation_id"]]
                      and by_id[e["consumer"]].get("package", {}).get("id") == driver]
            require(len(joined) == 1 and by_id[joined[0]["consumer"]]["exit_code"] == 0
                    and not by_id[joined[0]["consumer"]]["blockers"], "NativeRustExtern")
        forbidden_paths, _, namespace_blockers = native_evidence_paths(session)
        require(not namespace_blockers, "NativeOutputRole")
        require(not any(isinstance(r.get("source"), str) and native_path_identity(r["source"], r) in forbidden_paths
                        for r in rust_receipts)
                and not any(native_path_identity(o["path"], r) in forbidden_paths
                        for r in rust_receipts for o in r["declared_outputs"] + r["outputs"])
                and not any(native_path_identity(e["path"], by_id[e["consumer"]]) in forbidden_paths for e in edges)
                and not any(native_path_identity(path, by_id[a["invocation_id"]]) in forbidden_paths
                            for a in artifacts for path in a["files"])
                and not any(native_path_identity(path, r) in forbidden_paths for r in rust_receipts for o in r["outputs"]
                            for path in o.get("dep_info",{}).get("paths",[])), "NativeOutputRole")
        native_state_check(final_call,final["archive_post"],live=True)
        require(origin[0]["generated_files"].get("libsqlite3.a")==final["archive_post"]["sha256"],"NativeArchiveBinding")
        # Inspector is an owner request after Cargo: never replay expired builder FDs.
        inspector_env=dict(final_env)
        for key in ("CARGO_MAKEFLAGS","MAKEFLAGS","MFLAGS"):inspector_env.pop(key,None)
        atomic_json(session/"native-inspector-owner.json",{"state":"RecordingOnly","environment":inspector_env,
                    "archive_version":chain[-1],"tool":inv["generators"]["AR"]})
        name=Path(obj["operation"]["output"]).name
        inspectors=[]
        for args,limit in ((["t",str(out/"libsqlite3.a")],len(name.encode("ascii"))+1),
                           (["p",str(out/"libsqlite3.a"),name],obj["output_post"]["length"])):
            native_state_check(final_call,final["archive_post"],live=True)
            ic,ir=native_operation(session,policy,owner,"ar",args,Path(context["manifest"]),inspector_env,
                                   inspector=True,bounds={"stdout":limit,"stderr":0})
            require(ir["protocol_state"]=="Completed" and ir["tool_result"]==0,"NativeInspectorFailed")
            require((ic/"stderr.raw").stat().st_size==0,"NativeInspectorDiagnostic")
            if args[0]=="t":require((ic/"stdout.raw").read_bytes()==name.encode("ascii")+b"\n","NativeInspectorMembers")
            else:require((ic/"stdout.raw").stat().st_size==obj["output_post"]["length"]
                         and file_hash(ic/"stdout.raw")==obj["output_post"]["sha256"],"NativeInspectorObject")
            native_state_check(final_call,final["archive_post"],live=True);inspectors.append(ic.name)
        native_state_check(oc,obj["output_post"],live=True)
        result.update({"context":context,"builder":builder["invocation_id"],"cc_producer":cp["invocation_id"],
            "object_producer":oc.name,"archive_chain":chain,"final_archive_writer":chain[-1],"inspectors":inspectors,
            "rust_consumer":consumer["invocation_id"],"static_declaration":"static=sqlite3",
            "native_producer_qualification":"not_issued"})
    except (Refusal,OSError,KeyError,TypeError,ValueError,IndexError) as error:
        failures.append("NativeSeal:"+str(error))
    for call in sorted((session/"native-invocations").iterdir()):
        item={"operation_id":call.name}
        for leaf in ("request","receipt"):
            try:item[leaf+"_sha256"]=file_hash(call/(leaf+".json"))
            except (OSError,Refusal):failures.append("NativeIncomplete:"+call.name+":"+leaf)
        if call.name in calls and "input_final_state" in calls[call.name][1]:item["input_final_state"]=calls[call.name][1]["input_final_state"]
        result["operations"].append(item)
    result["blockers"]=sorted(set(failures));native_control(session,policy,owner)
    atomic_json(session/"native-record.json",result)
    return result


def wrapper(session_id, args):
    require(ID.fullmatch(session_id), "SessionId")
    session = SESSIONS / ("pending-" + session_id)
    require(session.is_dir() and not session.is_symlink(), "MissingSession")
    # The owner already validated/materialized the full policy. Bind this child
    # to that exact fixed-file policy; avoid rehashing Cargo/generators per crate.
    regular(POLICY)
    policy_raw = POLICY.read_bytes()
    policy, policy_hash = strict_json(policy_raw), digest(policy_raw)
    require(policy["inventory"] is not None, "MissingInventory")
    owner = strict_json((session / "owner.json").read_bytes())
    require(owner["policy_sha256"] == policy_hash and owner["owner_sha256"] == file_hash(Path(__file__))
            and owner["session"] == session_id and owner["state"] == "RecordingOnly", "SessionIdentity")
    inv = policy["inventory"]
    call = session / "invocations" / uuid.uuid4().hex
    call.mkdir(mode=0o700)
    atomic_json(call / "request.json", {"state": "RecordingOnly",
                "argv_hex": [os.fsencode(value).hex() for value in args],
                "cwd_hex": os.fsencode(os.getcwd()).hex(),
                "environment_hex": {os.fsencode(k).hex(): os.fsencode(v).hex() for k, v in os.environ.items()}})
    require(args and args[0] == pinned_file(inv["rustc"]), "WrongCompiler")
    cwd = Path.cwd().resolve()
    require(inside(cwd, session), "CompilerCwd")
    special = anyhow_context(args, os.environ, cwd, session, inv)
    if special is None:
        special = record10_context(args, os.environ, cwd, session, inv)
    if special is None:
        special = rustix_context(args, os.environ, cwd, session, inv)
    if special is None and policy["profile"] == BUNDLED_PROFILE:
        native_control(session, policy, owner)
        special = sqlite_rust_context(args, os.environ, cwd, session, inv)
    if special is None:
        special = ring_static_context(args, os.environ, cwd, session, inv)
    if special is None:
        special = psm_static_context(args, os.environ, cwd, session, inv)
    if special is None:
        special = zstd_static_context(args, os.environ, cwd, session, inv)
    if special is None:
        special = framework_context(args, os.environ, cwd, session, inv)
    if special is None:
        special = autocfg_context(args, os.environ, cwd, session, inv)
    if special is None:
        parsed = parse_rustc(args[1:])
        context = invocation_context(args, parsed, os.environ, cwd, session, inv)
    else:
        parsed, context = special
    compiler_environment(os.environ, session, inv["sysroot"], probe=parsed["probe"], context=context)
    jobserver_fds = inherited_jobserver_fds(os.environ)
    options, codegen = parsed["options"], parsed["codegen"]
    if "--target" in options:
        require(options["--target"] == [TARGET], "WrongTarget")
    if "--sysroot" in options:
        require(options["--sysroot"] == [inv["sysroot"]["root"]], "WrongSysroot")
    target = session / "target"
    externs, package, source, outputs = [], None, None, []
    sysroot_declarations = []
    if parsed["probe"] and raw_bare_externs(args):
        bare_proc_macro_declarations(args, parsed, context, cwd, session, inv, None, None, os.environ)
    stdin_bytes, stdin_evidence = None, None
    autocfg_stdin = context["kind"] == "NumTraitsAutocfgStdinProbe"
    rustix_stdin = context["kind"] == RUSTIX_KIND
    if rustix_stdin:
        stdin_bytes, stdin_evidence = capture_rustix_stdin(call)
        outputs = [{"path": str(Path(context["out_dir"]) / "rustix_test_can_compile"), "kind": "metadata"}]
        pre = metadata_state(call, Path(outputs[0]["path"]), "pre")
        predecessor = rustix_predecessor(session, call, context, stdin_evidence["template_index"], pre)
    elif autocfg_stdin:
        stdin_bytes, stdin_evidence = capture_autocfg_stdin(call, context)
        outputs = [{"path": str(Path(context["out_dir"]) / (context["crate_name"] + ".ll")),
                    "kind": "llvm-ir"}]
    elif not parsed["probe"]:
        source = (cwd / parsed["inputs"][0]).resolve()
        manifest_dir = Path(os.environ.get("CARGO_MANIFEST_DIR", ""))
        matches = []
        for candidate in inv["packages"]:
            tree = session / candidate["tree"]
            manifest = tree / candidate["manifest"]
            if manifest.parent == manifest_dir and inside(source, manifest.parent):
                matches.append(candidate)
        require(len(matches) == 1, "UnresolvedSourcePackage")
        package = matches[0]
        regular(source)
        tree = session / package["tree"]
        name = source.relative_to(tree).as_posix()
        require(inv[package["tree"]]["files"].get(name) == file_hash(source), "SourceMismatch")
        sysroot_declarations = bare_proc_macro_declarations(
            args, parsed, context, cwd, session, inv, package, source, os.environ)
        for value in options.get("--extern", []):
            if value == "proc_macro" and sysroot_declarations:
                continue
            name, separator, path = value.partition("=")
            require(separator and re.fullmatch(r"[A-Za-z0-9_]+", name), "UnresolvedExtern")
            path = (cwd / path).resolve()
            require(inside(path, target), "ExternEscape")
            externs.append({"name": name, "path": str(path)})
        for value in options.get("-L", []):
            _, separator, path = value.partition("=")
            path = path if separator else value
            require(inside(cwd / path, target), "SearchPathEscape")
        for value in codegen.get("incremental", []):
            require(inside(cwd / value, target), "IncrementalEscape")
        for value in codegen.get("linker", []):
            require(value in {v["path"] for v in inv["generators"].values()}, "UnpinnedLinker")
        outputs = selected_outputs(parsed, cwd, target)
    transient = context["kind"] in {"ProcMacro2FeatureProbe", "ThiserrorStaticFeatureProbe", ANYHOW_KIND} or autocfg_stdin or rustix_stdin
    record = {"state": "RecordingOnly", "kind": "TransientProbe" if transient else (
                  "Probe" if parsed["probe"] else "Compile"), "context": context,
              "argv_hex": [os.fsencode(value).hex() for value in args], "parsed": parsed,
              "cwd": str(cwd), "source": str(source) if source else None, "package": package,
              "role": "Probe" if parsed["probe"] else ("Target" if "--target" in options else "Host"),
              "environment_hex": {os.fsencode(k).hex(): os.fsencode(v).hex() for k, v in os.environ.items()},
              "externs": externs, "declared_outputs": outputs, "compiler_sha256": file_hash(Path(args[0]))}
    if sysroot_declarations:
        record["sysroot_extern_declarations"] = sysroot_declarations
    if stdin_evidence is not None:
        record["stdin"] = stdin_evidence
    if rustix_stdin:
        record.update(metadata_pre=pre, predecessor_invocation=predecessor)
    if "ring_static_declarations" in context:
        record["ring_archive_pre"] = ring_archive_observation(call, context, "pre")
    if "psm_static_declaration" in context:
        record["psm_archive_pre"] = psm_archive_observation(call, context, "pre")
    if "zstd_static_declaration" in context:
        record["zstd_archive_pre"] = static_archive_observation(call, context, "pre", "zstd", "libzstd.a", "ZstdArchiveEvidence")
    atomic_json(call / "invocation.json", record)
    stdin_failures = []
    code = run_streamed(args, cwd, dict(os.environ), call / "stdout.raw", call / "stderr.raw",
                        echo=True, pass_fds=jobserver_fds, stdin_bytes=stdin_bytes,
                        stdin_failures=stdin_failures)
    psm_post, psm_post_blockers = None, []
    if "psm_static_declaration" in context:
        try:
            psm_post = psm_archive_observation(call, context, "post")
            require(all(record["psm_archive_pre"][k] == psm_post[k] for k in ("path", "length", "sha256")), "PsmArchiveChanged")
        except (Refusal, OSError) as error:
            psm_post_blockers.append(str(error))
        try:
            require(psm_static_context(args, os.environ, cwd, session, inv) == (parsed, context), "PsmPostSource")
        except (Refusal, OSError) as error:
            psm_post_blockers.append("PsmPostSource:" + str(error))
    zstd_post, zstd_post_blockers = None, []
    if "zstd_static_declaration" in context:
        try:
            zstd_post = static_archive_observation(call, context, "post", "zstd", "libzstd.a", "ZstdArchiveEvidence")
            require(all(record["zstd_archive_pre"][k] == zstd_post[k] for k in ("path", "length", "sha256")), "ZstdArchiveChanged")
        except (Refusal, OSError) as error:
            zstd_post_blockers.append(str(error))
        try:
            require(zstd_static_context(args, os.environ, cwd, session, inv) == (parsed, context), "ZstdPostSource")
        except (Refusal, OSError) as error:
            zstd_post_blockers.append("ZstdPostSource:" + str(error))
    record["exit_code"] = code
    record["stdout_sha256"] = file_hash(call / "stdout.raw")
    record["stderr_sha256"] = file_hash(call / "stderr.raw")
    record["outputs"], record["blockers"] = [], []
    if "ring_static_declarations" in context:
        try:
            record["ring_archive_post"] = ring_archive_observation(call, context, "post")
            if any(a["sha256"] != b["sha256"] or a["length"] != b["length"]
                   for a, b in zip(record["ring_archive_pre"], record["ring_archive_post"])):
                record["blockers"].append("RingArchiveChanged")
        except Refusal as error:
            record["blockers"].append(str(error))
    if psm_post is not None:
        record["psm_archive_post"] = psm_post
    record["blockers"].extend(psm_post_blockers)
    if zstd_post is not None:
        record["zstd_archive_post"] = zstd_post
    record["blockers"].extend(zstd_post_blockers)
    if transient:
        try:
            if rustix_stdin:
                post = metadata_state(call, Path(outputs[0]["path"]), "post")
                record["metadata_post"] = post
                if post["exists"]:
                    record["outputs"] = [dict(outputs[0], sha256=post["sha256"], snapshot=post["snapshot"], observation_only=True)]
                elif code == 0:
                    record["blockers"].append("MissingDeclaredOutput:" + outputs[0]["path"])
            elif context["kind"] in {"ThiserrorStaticFeatureProbe", ANYHOW_KIND}:
                record["outputs"], record["blockers"] = record10_capture_outputs(call, outputs, code)
            else:
                record["outputs"], record["blockers"] = capture_transient_outputs(call, outputs, code)
        except (Refusal, OSError) as error:
            # Preserve the actual compiler status even when evidence retention fails.
            record["blockers"].append("TransientEvidence:" + str(error))
        record["probe_outcome"] = {0: "Supported", 1: "Unsupported"}.get(code, "CompilerFailure")
        if code not in (0, 1):
            record["blockers"].append("CompilerFailed")
    elif code == 0:
        for item in outputs:
            path = Path(item["path"])
            if not path.is_file() or path.is_symlink():
                record["blockers"].append("MissingDeclaredOutput:" + str(path)); continue
            output = dict(item, sha256=file_hash(path))
            if item["kind"] == "dep-info":
                try:
                    output["dep_info"] = dep_info(path.read_bytes())
                except Refusal as error:
                    record["blockers"].append(str(error))
            record["outputs"].append(output)
    else:
        record["blockers"].append("CompilerFailed")
    if context["kind"] in RECORD10_KINDS:
        try:
            require(record10_context(args, os.environ, cwd, session, inv) == (parsed, context), "Record10PostSource")
        except (Refusal, OSError) as error:
            record["blockers"].append("Record10PostSource:" + str(error))
    if context["kind"] == ANYHOW_KIND:
        try:
            require(anyhow_context(args, os.environ, cwd, session, inv) == (parsed, context), "AnyhowPostSource")
        except (Refusal, OSError) as error:
            record["blockers"].append("AnyhowPostSource:" + str(error))
    record["blockers"].extend(stdin_failures)
    atomic_json(call / "receipt.json", record)
    return code if code != 0 else (0 if not record["blockers"] else 2)


def cargo_events(raw_path):
    events, blockers = [], []
    known = {
        "compiler-artifact": {"reason", "package_id", "manifest_path", "target", "profile", "features", "filenames", "executable", "fresh"},
        "build-script-executed": {"reason", "package_id", "linked_libs", "linked_paths", "cfgs", "env", "out_dir"},
        "compiler-message": {"reason", "package_id", "manifest_path", "target", "message"},
        "build-finished": {"reason", "success"},
    }
    with open(raw_path, "rb") as stream:
        for number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            try:
                event = strict_json(line)
            except (Refusal, ValueError, UnicodeError):
                blockers.append("UnknownCargoBytes:" + str(number)); continue
            if not isinstance(event, dict) or event.get("reason") not in known:
                blockers.append("UnknownCargoEvent:" + str(number)); continue
            if set(event) - known[event["reason"]]:
                blockers.append("UnknownCargoFields:" + str(number))
            # All original fields/bytes remain in stdout.raw. Only explicit
            # fields participate in associations; unknowns never mean Matched.
            events.append(event)
    return events, blockers


def custom_build_producer(event, receipt, hashes):
    """Use structured filename plus actual link bytes; null executable is Cargo data."""
    target = event.get("target", {})
    require(target.get("kind") == ["custom-build"] and target.get("crate_types") == ["bin"],
            "CustomBuildTarget")
    require(receipt["kind"] == "Compile" and receipt["exit_code"] == 0 and not receipt["blockers"],
            "CustomBuildReceipt")
    require(len(hashes) == 1 and len(event.get("filenames", [])) == 1, "CustomBuildFilenames")
    alias, sha = next(iter(hashes.items()))
    require("executable" in event and event["executable"] in (None, alias), "CustomBuildExecutable")
    links = [o for o in receipt["outputs"] if o["kind"] == "link" and o["sha256"] == sha]
    require(len(links) == 1, "CustomBuildLinkAlias")
    return {"path": alias, "sha256": sha, "link_path": links[0]["path"]}


def autocfg_graph_blockers(receipts, associations, edges):
    """Final observed branch and actual helper edge, never inferred from directory suffixes."""
    blockers = []
    calls = [r for r in receipts if r.get("context", {}).get("kind") in AUTOCFG_KINDS]
    groups = {}
    for call in calls:
        context = call["context"]
        groups.setdefault((context["package_id"], context["out_dir"]), []).append(call)
    by_id = {r["invocation_id"]: r for r in receipts}
    for (package, out), group in groups.items():
        try:
            origins = [a for a in associations if a["package_id"] == package and a["out_dir"] == out]
            require(len(origins) == 1, "AutocfgOrigin")
            builder = origins[0]["producer_invocation"]
            helper_edges = [e for e in edges if e["consumer"] == builder and e["name"] == "autocfg"]
            require(len(helper_edges) == 1 and len(helper_edges[0]["producers"]) == 1, "AutocfgHelperEdge")
            helper = by_id[helper_edges[0]["producers"][0]]
            require(helper["kind"] == "Compile" and helper["role"] == "Host"
                    and helper["exit_code"] == 0 and not helper["blockers"]
                    and helper["package"] == {"id": "registry+https://github.com/rust-lang/crates.io-index#autocfg@1.5.0",
                                              "tree": "vendor", "manifest": "autocfg/Cargo.toml"}
                    and helper["source"] == str(Path(group[0]["context"]["manifest"]).parent / "autocfg/src/lib.rs"),
                    "AutocfgHelperProducer")
            versions = [r for r in group if r["context"]["kind"] == "NumTraitsAutocfgVersion"]
            probes = [r for r in group if r["context"]["kind"] == "NumTraitsAutocfgStdinProbe"]
            require(len(versions) == 1 and versions[0]["exit_code"] == 0 and not versions[0]["blockers"],
                    "AutocfgVersion")
            require(len(probes) in (2, 3) and len({r["context"]["prefix"] for r in probes}) == 1,
                    "AutocfgBranch")
            indexed = {r["context"]["index"]: r for r in probes}
            require(len(indexed) == len(probes) and set(indexed) == set(range(len(probes)))
                    and all(r["exit_code"] in (0, 1) and not r["blockers"] for r in probes), "AutocfgBranch")
            require(indexed[0]["stdin"]["template"] == "EmptyStd", "AutocfgBranch")
            if indexed[0]["exit_code"] == 0:
                require(len(probes) == 2 and indexed[1]["stdin"]["template"] == "TotalCmp", "AutocfgBranch")
            else:
                require(len(probes) == 3 and indexed[1]["stdin"]["template"] == "NoStd", "AutocfgBranch")
                last = "NoStdTotalCmp" if indexed[1]["exit_code"] == 0 else "TotalCmp"
                require(indexed[2]["stdin"]["template"] == last, "AutocfgBranch")
        except (Refusal, KeyError, TypeError) as error:
            blockers.append("AutocfgGraph:" + str(error))
    return blockers


def seal_record(session, policy, cargo_exit):
    """Resolve observed graph after Cargo; this is a diagnostic seal, never issuance."""
    events, blockers = cargo_events(session / "cargo.stdout.raw")
    receipts = []
    sysroot_declarations = {}
    record10_transient_paths = set()
    record10_invalid_calls = set()
    psm_paths, psm_hashes, psm_invalid_calls = set(), set(), set()
    zstd_paths, zstd_hashes, anyhow_paths, anyhow_hashes, tools12_invalid_calls = set(), set(), set(), set(), set()
    for event in events:
        if (event["reason"] == "build-script-executed" and event.get("package_id") == ZSTD_PACKAGE
                and isinstance(event.get("out_dir"), str)):
            paths, hashes = tools12_namespace("zstd-sys", [], {"OUT_DIR": event["out_dir"]}, session, session, session)
            zstd_paths.update(paths); zstd_hashes.update(hashes)
    # Fixed event names only strengthen exclusion if mutable consumer annotations vanish.
    for event in events:
        if (event["reason"] != "build-script-executed" or event.get("package_id") != PSM_PACKAGE
                or not isinstance(event.get("out_dir"), str)): continue
        out = Path(event["out_dir"]).resolve()
        if (out.name == "out"
                and out.parent.parent == session / "target" / TARGET / "debug/build"
                and re.fullmatch(r"psm-[0-9a-f]{16}", out.parent.name)):
            archive = out / PSM_ARCHIVE; psm_paths.add(str(archive))
            if archive.is_file() and not archive.is_symlink(): psm_hashes.add(file_hash(archive))
    for directory in sorted((session / "invocations").iterdir()):
        # Reconstruct closed names even for request-only failures, before any
        # full qualification or mutable receipt annotation can be consulted.
        try:
            request = strict_json((directory / "request.json").read_bytes())
            raw = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
            env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
            cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
            for name, paths, hashes in (("zstd-sys", zstd_paths, zstd_hashes), ("anyhow", anyhow_paths, anyhow_hashes)):
                names, values = tools12_namespace(name, raw, env, cwd, directory, session)
                paths.update(names); hashes.update(values)
            try:
                zstd_static_context(raw, env, cwd, session, policy["inventory"])
                anyhow_context(raw, env, cwd, session, policy["inventory"])
            except (Refusal, OSError) as error:
                blockers.append("Tools12Evidence:" + str(error)); tools12_invalid_calls.add(directory.name)
            record10_transient_paths.update(record10_transient_namespace(raw, env, cwd, session))
            paths, hashes = psm_archive_namespace(raw, env, cwd, directory, session)
            psm_paths.update(paths); psm_hashes.update(hashes)
            try:
                psm_static_context(raw, env, cwd, session, policy["inventory"])
            except (Refusal, OSError) as error:
                blockers.append("PsmEvidence:" + str(error)); psm_invalid_calls.add(directory.name)
        except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError) as error:
            blockers.append("Record10Namespace:" + str(error))
            record10_invalid_calls.add(directory.name)
        # Retained observations strengthen exclusion even if files or annotations disappear.
        for leaf in ("invocation.json", "receipt.json"):
            path = directory / leaf
            if not path.is_file() or path.is_symlink(): continue
            try:
                observation = strict_json(path.read_bytes())
                for phase in ("pre", "post"):
                    row = observation.get("psm_archive_" + phase, {})
                    sha = row.get("sha256") if isinstance(row, dict) else None
                    if isinstance(sha, str) and re.fullmatch(r"[0-9a-f]{64}", sha): psm_hashes.add(sha)
            except (Refusal, OSError, TypeError, ValueError, AttributeError) as error:
                blockers.append("PsmArchiveNamespace:" + str(error)); psm_invalid_calls.add(directory.name)
        if not (directory / "receipt.json").is_file():
            blockers.append("IncompleteInvocation:" + directory.name); continue
        receipt = strict_json((directory / "receipt.json").read_bytes())
        if "psm_static_declaration" in receipt.get("context", {}) or any(k.startswith("psm_archive_") for k in receipt):
            for phase in ("pre", "post"):
                snapshot = directory / ("psm-archive-" + phase + ".raw"); psm_paths.add(str(snapshot))
                if snapshot.is_file() and not snapshot.is_symlink(): psm_hashes.add(file_hash(snapshot))
        receipt["invocation_id"] = directory.name
        receipt["receipt_sha256"] = file_hash(directory / "receipt.json")
        receipt["request_sha256"] = file_hash(directory / "request.json")
        receipt["invocation_sha256"] = file_hash(directory / "invocation.json")
        for stream in ("stdout", "stderr"):
            if file_hash(directory / (stream + ".raw")) != receipt[stream + "_sha256"]:
                blockers.append("ChangedInvocationBytes:" + directory.name)
        receipts.append(receipt)
        blockers.extend(receipt["blockers"])
        try:
            declared = verify_sysroot_extern_declaration(directory, receipt, session, policy["inventory"])
            if declared:
                sysroot_declarations[directory.name] = declared
        except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError):
            blockers.append("ChangedSysrootExternDeclaration:" + directory.name)
        initial = strict_json((directory / "invocation.json").read_bytes())
        try:
            zstd_evidence(receipt, session, policy["inventory"])
            anyhow_evidence(receipt, session, policy["inventory"])
        except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError) as error:
            blockers.append("Tools12Evidence:" + str(error)); tools12_invalid_calls.add(directory.name)
        try:
            request = strict_json((directory / "request.json").read_bytes())
            raw = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
            env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
            cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
            record10_context(raw, env, cwd, session, policy["inventory"])
            record10_evidence(receipt, directory, session, policy["inventory"])
        except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError) as error:
            blockers.append("Record10Evidence:" + str(error))
            record10_invalid_calls.add(directory.name)
        if "framework_declaration" in initial.get("context", {}) or "framework_declaration" in receipt.get("context", {}):
            try:
                request = strict_json((directory / "request.json").read_bytes())
                require(all(initial[k] == receipt[k] for k in ("context", "argv_hex", "environment_hex", "parsed", "source", "package", "role", "kind"))
                        and request["argv_hex"] == receipt["argv_hex"]
                        and request["environment_hex"] == receipt["environment_hex"], "FrameworkInvocationEvidence")
                require([os.fsdecode(bytes.fromhex(a)) for a in receipt["argv_hex"]][-2:] == ["-l", FRAMEWORK_LITERAL],
                        "FrameworkInvocationEvidence")
            except (Refusal, KeyError, TypeError, ValueError) as error:
                blockers.append("ChangedFrameworkEvidence:" + directory.name + ":" + str(error))
        if receipt.get("context", {}).get("kind") == RUSTIX_KIND:
            try:
                verify_rustix_evidence(directory, receipt)
            except (Refusal, OSError, KeyError, TypeError, ValueError) as error:
                blockers.append("ChangedRustixEvidence:" + directory.name + ":" + str(error))
        if receipt.get("context", {}).get("kind") == "NumTraitsAutocfgStdinProbe":
            try:
                evidence = receipt["stdin"]
                path = directory / "stdin.raw"
                regular(path)
                require(path.resolve() == path and path.stat().st_size <= 60, "ChangedAutocfgStdin")
                body = path.read_bytes()
                metadata = directory / "stdin.json"
                regular(metadata)
                require(metadata.resolve() == metadata, "ChangedAutocfgStdin")
                require(evidence == strict_json(metadata.read_bytes())
                        and evidence["snapshot"] == "stdin.raw" and evidence["eof"] is True
                        and evidence["truncated"] is False and evidence["length"] == len(body)
                        and evidence["sha256"] == digest(body)
                        and AUTOCFG_BODIES.get(evidence["template"]) == body,
                        "ChangedAutocfgStdin")
            except (Refusal, OSError, KeyError, TypeError, ValueError):
                blockers.append("ChangedAutocfgStdin:" + directory.name)
    # Collect all declared transient paths before registering any ordinary output.
    # An unsupported probe may have no output file; its declared namespace is still excluded.
    transient_paths = {str(Path(o["path"]).resolve()) for r in receipts if r["kind"] == "TransientProbe"
                       for o in r["declared_outputs"] + r["outputs"]} | record10_transient_paths | anyhow_paths
    native_paths, native_probes, foreign_paths, native_hashes = set(), set(), set(), set()
    if policy["profile"] == BUNDLED_PROFILE:
        native_paths, native_probes, namespace_blockers = native_evidence_paths(session)
        blockers.extend(namespace_blockers)
        foreign_paths, _, native_hashes, foreign_blockers = foreign_evidence_namespace(session)
        blockers.extend(foreign_blockers)
    output_owners, transient_collisions = {}, set()
    for receipt in receipts:
        if receipt["invocation_id"] in record10_invalid_calls | psm_invalid_calls | tools12_invalid_calls:
            transient_collisions.add(receipt["invocation_id"])
            continue
        denied = next((reason for paths, hashes, reason in ((zstd_paths, zstd_hashes, "ZstdArchiveOwnership:Output"),
                      (anyhow_paths, anyhow_hashes, "AnyhowTransientOwnership:Output"))
                       if receipt["kind"] != "TransientProbe" and any(tools12_owned(o["path"], o.get("sha256"), paths, hashes)
                         for o in receipt["declared_outputs"] + receipt["outputs"])), None)
        if denied:
            blockers.append(denied); transient_collisions.add(receipt["invocation_id"]); continue
        if any(str(Path(o["path"]).resolve()) in psm_paths or o.get("sha256") in psm_hashes
               or (Path(o["path"]).is_file() and not Path(o["path"]).is_symlink()
                   and file_hash(Path(o["path"])) in psm_hashes)
               for o in receipt["declared_outputs"] + receipt["outputs"]):
            blockers.append("PsmArchiveOwnership:Output"); transient_collisions.add(receipt["invocation_id"]); continue
        if (native_paths or native_hashes) and ((isinstance(receipt.get("source"), str)
                and foreign_owned(receipt["source"], receipt["cwd"], native_paths, native_hashes))
                or any(foreign_owned(o["path"], receipt["cwd"], native_paths, native_hashes)
                       or o.get("sha256") in native_hashes
                       for o in receipt["declared_outputs"] + receipt["outputs"])):
            blockers.append("NativeOutputRole:" + receipt["invocation_id"])
            transient_collisions.add(receipt["invocation_id"])
            continue
        if receipt["kind"] != "TransientProbe":
            collisions = {str(Path(o["path"]).resolve()) for o in receipt["declared_outputs"] + receipt["outputs"]} & transient_paths
            if collisions:
                blockers.extend("TransientOrdinaryOutput:" + path for path in sorted(collisions))
                transient_collisions.add(receipt["invocation_id"])
                continue
        for output in receipt["outputs"]:
            path = Path(output["path"])
            if receipt["kind"] == "TransientProbe":
                snapshot = session / "invocations" / receipt["invocation_id"] / output["snapshot"]
                if (snapshot.parent != session / "invocations" / receipt["invocation_id"]
                        or snapshot.resolve() != snapshot or not snapshot.is_file()
                        or snapshot.is_symlink() or file_hash(snapshot) != output["sha256"]):
                    blockers.append("ChangedTransientEvidence:" + receipt["invocation_id"])
                continue
            if not path.is_file() or path.is_symlink() or file_hash(path) != output["sha256"]:
                blockers.append("ChangedOutput:" + str(path))
            output_owners.setdefault(str(path), []).append(receipt["invocation_id"])
    artifacts, associations, selected = [], [], []
    for event in events:
        if event["reason"] != "compiler-artifact":
            continue
        target = event.get("target", {})
        root = target.get("src_path")
        filename_list = event.get("filenames")
        if not isinstance(root, str) or not isinstance(filename_list, list) or not all(isinstance(f, str) for f in filename_list):
            blockers.append("IncompleteArtifactEvent"); continue
        denied = next((reason for paths, hashes, reason in ((zstd_paths, zstd_hashes, "ZstdArchiveOwnership:CargoArtifact"),
                      (anyhow_paths, anyhow_hashes, "AnyhowTransientOwnership:CargoArtifact"))
                       if any(tools12_owned(f, None, paths, hashes) for f in filename_list)), None)
        if denied:
            blockers.append(denied); continue
        forbidden = [f for f in filename_list if str(Path(f).resolve()) in transient_paths]
        if forbidden:
            blockers.extend("TransientCargoArtifact:" + f for f in forbidden)
            continue
        # A relative Cargo filename has only the actual requesting receipt's cwd.
        # No candidate may claim a native namespace through an alternate spelling.
        if native_paths or native_hashes:
            native_candidates = [r for r in receipts if isinstance(r.get("source"), str)
                                 and native_path_identity(r["source"], r) == native_path_identity(root, r)
                                 and package_id(r["package"], session) == event.get("package_id")]
            if (any(foreign_owned(f, r["cwd"], native_paths, native_hashes) for r in native_candidates for f in filename_list)
                    or any(foreign_owned(f, session, native_paths, native_hashes) for f in filename_list)):
                blockers.append("NativeOutputRole:CargoArtifact")
                continue
        if any(str(Path(f).resolve()) in psm_paths or (Path(f).is_file() and not Path(f).is_symlink()
               and file_hash(Path(f)) in psm_hashes) for f in filename_list):
            blockers.append("PsmArchiveOwnership:CargoArtifact"); continue
        hashes = {}
        for filename in filename_list:
            path = Path(filename)
            if not inside(path, session / "target") or not path.is_file() or path.is_symlink():
                blockers.append("InvalidCargoArtifactPath:" + filename)
            else:
                hashes[filename] = file_hash(path)
        candidates = [r for r in receipts if r["kind"] == "Compile"
                      and r["invocation_id"] not in transient_collisions and r["source"] == str(Path(root).resolve())
                      and package_id(r["package"], session) == event.get("package_id")
                      and len(hashes) == len(filename_list) and hashes
                      and set(hashes.values()) <= {o["sha256"] for o in r["outputs"]}]
        # Cargo may copy/rename a build-script executable. Its structured event,
        # exact source/package and output bytes establish that alias, not a basename.
        if len(candidates) != 1:
            blockers.append("UnresolvedCargoArtifact:" + root); continue
        record = {"package_id": event["package_id"], "target": target,
                  "invocation_id": candidates[0]["invocation_id"], "files": filename_list, "file_sha256": hashes,
                  "executable": event.get("executable")}
        if target.get("kind") == ["custom-build"]:
            try:
                record["builder_alias"] = custom_build_producer(event, candidates[0], hashes)
            except Refusal as error:
                blockers.append(str(error)); continue
        artifacts.append(record)
        for filename in filename_list:
            owners = output_owners.setdefault(filename, [])
            if record["invocation_id"] not in owners:
                owners.append(record["invocation_id"])
        if root == str(session / "application/src/lib.rs") and "lib" in target.get("kind", []):
            selected.append(record)
    bound_sysroot_declarations = []
    for receipt in receipts:
        call_id = receipt["invocation_id"]
        if call_id not in sysroot_declarations:
            continue
        requesting = [a for a in artifacts if a["invocation_id"] == call_id]
        if (receipt["exit_code"] != 0 or receipt["blockers"] or len(requesting) != 1
                or requesting[0]["target"].get("kind") != ["proc-macro"]
                or requesting[0]["target"].get("crate_types") != ["proc-macro"]
                or requesting[0]["target"].get("name") != receipt["parsed"]["options"]["--crate-name"][0]):
            blockers.append("UnresolvedSysrootExternConsumer:" + call_id)
            continue
        bound_sysroot_declarations.extend(dict(d, consumer=call_id) for d in sysroot_declarations[call_id])
    for event in events:
        if event["reason"] != "build-script-executed":
            continue
        out_dir = event.get("out_dir")
        producers = [a for a in artifacts if a["package_id"] == event.get("package_id")
                     and "builder_alias" in a]
        mapping = None
        if event.get("package_id") == SERDE_PACKAGE:
            try:
                producers, mapping = serde_core_mapping(event, producers, receipts, artifacts, events, session, policy["inventory"])
            except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError, StopIteration) as error:
                blockers.append("SerdeCoreMapping:" + (str(error) if isinstance(error, Refusal) else "SerdeCoreEvidence")); continue
        if (not isinstance(out_dir, str) or not inside(Path(out_dir), session / "target")
                or str(Path(out_dir)) != out_dir or Path(out_dir).resolve() != Path(out_dir)
                or not Path(out_dir).is_dir() or len(producers) != 1):
            blockers.append("UnresolvedBuildScriptProducer"); continue
        if any(a["out_dir"] == out_dir for a in associations):
            blockers.append("DuplicateBuildScriptOutDir:" + out_dir); continue
        generated = {}
        for path in Path(out_dir).rglob("*"):
            if path.is_symlink() or (not path.is_dir() and not path.is_file()):
                blockers.append("GeneratedNonregular:" + str(path)); continue
            if path.is_file() and (str(path.resolve()) in foreign_paths
                    or native_hashes and file_hash(path) in native_hashes):
                blockers.append("NativeOutputRole:GeneratedFile:" + str(path)); continue
            if (path.is_file() and str(path.resolve()) not in transient_paths | native_probes
                    and not tools12_owned(str(path), None, anyhow_paths, anyhow_hashes)):
                generated[path.relative_to(out_dir).as_posix()] = file_hash(path)
        associations.append({"out_dir": str(Path(out_dir).resolve()), "package_id": event["package_id"],
                             "producer_invocation": producers[0]["invocation_id"],
                             "generated_files": generated, "cargo_event": event})
        if mapping is not None:
            associations[-1]["recording_only_mapping"] = mapping
    nested_origins = []
    for receipt in receipts:
        context = receipt.get("context", {})
        if context.get("kind") in {"LibcBuildVersion", "ProcMacro2FeatureProbe", RUSTIX_KIND, ANYHOW_KIND} | AUTOCFG_KINDS | RECORD10_KINDS:
            matches = [a for a in associations if a["package_id"] == context["package_id"]
                       and a["out_dir"] == context["out_dir"]]
            if len(matches) != 1:
                blockers.append("UnresolvedNestedOrigin:" + receipt["invocation_id"])
            else:
                nested_origins.append({"invocation_id": receipt["invocation_id"],
                                       "producer_invocation": matches[0]["producer_invocation"],
                                       "out_dir": context["out_dir"], "package_id": context["package_id"]})
    edges = []
    for receipt in receipts:
        for edge in receipt["externs"]:
            denied = next((reason for paths, hashes, reason in ((zstd_paths, zstd_hashes, "ZstdArchiveOwnership:Extern"),
                          (anyhow_paths, anyhow_hashes, "AnyhowTransientOwnership:Extern"))
                           if tools12_owned(edge["path"], None, paths, hashes)), None)
            if denied:
                blockers.append(denied); edges.append(dict(edge, consumer=receipt["invocation_id"], producers=[])); continue
            if str(Path(edge["path"]).resolve()) in psm_paths or (Path(edge["path"]).is_file()
                    and not Path(edge["path"]).is_symlink() and file_hash(Path(edge["path"])) in psm_hashes):
                blockers.append("PsmArchiveOwnership:Extern"); edges.append(dict(edge, consumer=receipt["invocation_id"], producers=[])); continue
            if (native_paths or native_hashes) and foreign_owned(edge["path"], receipt["cwd"], native_paths, native_hashes):
                blockers.append("NativeOutputRole:" + edge["path"])
                edges.append(dict(edge, consumer=receipt["invocation_id"], producers=[]))
                continue
            producers = output_owners.get(edge["path"], [])
            if str(Path(edge["path"]).resolve()) in transient_paths:
                blockers.append("TransientExtern:" + edge["path"])
                producers = []
            if len(producers) != 1:
                blockers.append("UnresolvedExternProducer:" + edge["path"])
            edges.append(dict(edge, consumer=receipt["invocation_id"], producers=producers))
    blockers.extend(autocfg_graph_blockers(receipts, associations, edges))
    blockers.extend(record10_graph_blockers(receipts, associations, artifacts, events, session, policy["inventory"]))
    new_blockers, declarations = new_role_graphs(receipts, associations)
    blockers.extend(new_blockers)
    ring_blockers, ring_declarations = ring_static_graph(receipts, associations, artifacts, events, edges,
                                                       session, policy["inventory"])
    blockers.extend(ring_blockers)
    declarations.extend(ring_declarations)
    psm_blockers, psm_declarations = psm_static_graph(receipts, associations, artifacts, events, edges, session, policy["inventory"])
    blockers.extend(psm_blockers); declarations.extend(psm_declarations)
    zstd_blockers, zstd_declarations = zstd_graph(receipts, associations, artifacts, events, edges, session, policy["inventory"], zstd_paths, zstd_hashes)
    blockers.extend(zstd_blockers); declarations.extend(zstd_declarations)
    blockers.extend(anyhow_graph(receipts, associations, artifacts, events, session, policy["inventory"]))
    consumed = []
    for receipt in receipts:
        if receipt["invocation_id"] in transient_collisions:
            continue
        for output in receipt["outputs"]:
            for value in output.get("dep_info", {}).get("paths", []):
                path = (Path(receipt["cwd"]) / value).resolve()
                denied = next((reason for paths, hashes, reason in ((zstd_paths, zstd_hashes, "ZstdArchiveOwnership:ConsumedSource"),
                              (anyhow_paths, anyhow_hashes, "AnyhowTransientOwnership:ConsumedSource"))
                               if tools12_owned(str(path), None, paths, hashes)), None)
                if denied:
                    blockers.append(denied); continue
                if str(path) in psm_paths or (path.is_file() and not path.is_symlink() and file_hash(path) in psm_hashes):
                    blockers.append("PsmArchiveOwnership:ConsumedSource"); continue
                if (native_paths or native_hashes) and foreign_owned(value, receipt["cwd"], native_paths, native_hashes) and not (
                        str(path) not in native_paths and foreign_borrowed_cc_literal(value, receipt, session, policy["inventory"], artifacts)):
                    blockers.append("NativeOutputRole:" + str(path)); continue
                if str(path) in transient_paths:
                    blockers.append("TransientConsumedSource:" + str(path)); continue
                owner = None
                for tree in ("application", "vendor"):
                    if inside(path, session / tree):
                        name = path.relative_to(session / tree).as_posix()
                        if name in policy["inventory"][tree]["files"]:
                            owner = {"tree": tree, "relative_path": name}
                if owner is None:
                    matches = [a for a in associations if inside(path, Path(a["out_dir"]))]
                    if len(matches) == 1 and receipt["kind"] != "TransientProbe":
                        association = matches[0]
                        name = path.relative_to(association["out_dir"]).as_posix()
                        expected = association["generated_files"].get(name)
                        if path.is_file() and expected == file_hash(path):
                            owner = {"generated_by": association["producer_invocation"], "out_dir": association["out_dir"]}
                            if "recording_only_mapping" in association:
                                owner["recording_only_mapping"] = association["recording_only_mapping"]
                if owner is None or not path.is_file() or path.is_symlink():
                    blockers.append("UnresolvedConsumedSource:" + str(path)); continue
                consumed.append({"path": str(path), "sha256": file_hash(path), "owner": owner})
    finishes = [e for e in events if e["reason"] == "build-finished"]
    if cargo_exit != 0 or len(finishes) != 1 or finishes[0].get("success") is not True:
        blockers.append("CargoDidNotFinishSuccessfully")
    if len(selected) != 1:
        blockers.append("UnresolvedSelectedLibrary")
    # Complete immutable inventories are checked at materialization and seal,
    # not redundantly per dependency edge. Generator reads beyond dep-info are
    # TCB inputs; this record explicitly awaits review, not exhaustive tracing.
    for tree in ("application", "vendor"):
        spec = dict(policy["inventory"][tree], root=str(session / tree))
        inventory(spec)
    inventory(policy["inventory"]["sysroot"])
    validate_config_chain(ROOT, policy["inventory"]["ancestor_configs"])
    for name in ("cargo", "rustc", "python"):
        pinned_file(policy["inventory"][name])
    for spec in policy["inventory"]["generators"].values():
        pinned_file(spec)
    require(file_hash(Path(__file__)) == policy["inventory"]["owner_sha256"], "OwnerDrift")
    seal = {"schema": SCHEMA, "state": "RecordingOnly", "review_gate": "IndependentPolicyReviewRequired",
            "cargo_exit_code": cargo_exit, "blockers": sorted(set(blockers)), "selected_library": selected,
            "extern_edges": edges, "build_script_associations": associations, "consumed_sources": consumed,
            "nested_origins": nested_origins, "native_link_declarations": declarations,
            # This binds the owner receipt; its nested owner_sha256 binds tool source.
            "owner_sha256": file_hash(session / "owner.json"),
            "policy_sha256": digest(POLICY.read_bytes()),
            "invocations": [{key: r[key] for key in ("invocation_id", "receipt_sha256", "request_sha256", "invocation_sha256", "stdout_sha256", "stderr_sha256")} for r in receipts],
            "cargo_stdout_sha256": file_hash(session / "cargo.stdout.raw"),
            "cargo_stderr_sha256": file_hash(session / "cargo.stderr.raw")}
    if bound_sysroot_declarations:
        seal["sysroot_extern_declarations"] = bound_sysroot_declarations
    if policy["profile"] == BUNDLED_PROFILE:
        native = native_seal(session, policy, receipts, associations, artifacts, events, edges)
        seal["native_record_sha256"] = file_hash(session / "native-record.json")
        seal["blockers"] = sorted(set(seal["blockers"] + native["blockers"]))
        if (session / "foreign-native-invocations").exists() or (session / "foreign-native-invocations").is_symlink():
            foreign = foreign_seal(session, policy)
            seal["foreign_native_record_sha256"] = file_hash(session / "foreign-native-record.json")
            seal["blockers"] = sorted(set(seal["blockers"] + foreign["blockers"]))
    atomic_json(session / "record.json", seal)
    return seal


def record():
    policy, policy_hash = load_policy()
    if policy["inventory"] is None:
        return {"schema": SCHEMA, "state": "RecordingOnly", "reason": "MissingInventory",
                "profile": policy["profile"], "policy_sha256": policy_hash}, 2
    inv = policy["inventory"]
    for tree in ("application", "vendor", "sysroot"):
        inventory(inv[tree])
    # A source snapshot containing Cargo configs could change source selection.
    require(not any(name in {".cargo/config", ".cargo/config.toml"}
                    for name in inv["application"]["files"]), "SnapshotCargoConfig")
    validate_config_chain(ROOT, inv["ancestor_configs"])
    loader_path = sysroot_loader_path(inv["sysroot"])
    SESSIONS.mkdir(mode=0o700, exist_ok=True)
    require(not SESSIONS.is_symlink() and SESSIONS.resolve() == SESSIONS, "SessionRoot")
    session_id = uuid.uuid4().hex
    session = SESSIONS / ("pending-" + session_id)
    session.mkdir(mode=0o700)
    for directory in ("application", "vendor", "target", "cargo-home", "home", "tmp", "invocations"):
        (session / directory).mkdir(mode=0o700)
    for tree in ("application", "vendor"):
        inventory(inv[tree], session / tree)
    inventory(inv["sysroot"])
    config = session / "cargo-home/config.toml"
    config.write_text('[source.crates-io]\nreplace-with = "reviewed-vendor"\n[source.reviewed-vendor]\ndirectory = '
                      + json.dumps(str(session / "vendor")) + '\n[net]\noffline = true\n', encoding="utf-8")
    launcher = session / "rustc-wrapper"
    launcher.write_text("#!" + inv["python"]["path"] + " -I\nimport os, sys\nos.execv("
                        + repr(inv["python"]["path"]) + ", [" + repr(inv["python"]["path"])
                        + ", '-I', " + repr(str(Path(__file__).resolve())) + ", '_wrapper', "
                        + repr(session_id) + ", *sys.argv[1:]])\n", encoding="utf-8")
    launcher.chmod(0o700)
    env = dict(inv["environment"], HOME=str(session / "home"), TMPDIR=str(session / "tmp"),
               CARGO_HOME=str(session / "cargo-home"), RUSTC=inv["rustc"]["path"], RUSTC_WRAPPER=str(launcher),
               DYLD_FALLBACK_LIBRARY_PATH=loader_path)
    argv = [inv["cargo"]["path"], "build", "--locked", "--offline", "--lib", "--target", TARGET,
            "--message-format=json-render-diagnostics", "--manifest-path", str(session / "application/Cargo.toml"),
            "--target-dir", str(session / "target")]
    native_wrappers = None
    if policy["profile"] == BUNDLED_PROFILE:
        (session / "native-invocations").mkdir(mode=0o700)
        native_wrappers = native_launchers(session, inv)
        env.update(CC=native_wrappers["cc"]["path"], AR=native_wrappers["ar"]["path"])
        argv += ["--features", "replay-sqlite-bundled-v1"]
    owner = {"schema": SCHEMA, "state": "RecordingOnly", "session": session_id, "profile": policy["profile"],
             "policy_sha256": policy_hash, "owner_sha256": file_hash(Path(__file__)), "argv": argv,
             "environment": env, "config_sha256": file_hash(config), "launcher_sha256": file_hash(launcher)}
    if native_wrappers is not None:
        owner["native_launchers"] = native_wrappers
        owner["native_underlying_tools"] = {name: inv["generators"][name] for name in ("CC", "AR")}
    atomic_json(session / "owner.json", owner)
    code = run_streamed(argv, session / "application", env, session / "cargo.stdout.raw", session / "cargo.stderr.raw")
    require(digest(POLICY.read_bytes()) == policy_hash and file_hash(config) == owner["config_sha256"]
            and file_hash(launcher) == owner["launcher_sha256"], "ControlDrift")
    seal = seal_record(session, policy, code)
    # Immutable paths in raw observations deliberately refer to this session.
    # Atomic record.json creation is the publication of a diagnostic only;
    # target products are never copied into a usable release/output directory.
    return {"schema": SCHEMA, "state": "RecordingOnly", "record_path": str(session / "record.json"),
            "record_sha256": file_hash(session / "record.json"), "blockers": seal["blockers"],
            "review_gate": "IndependentPolicyReviewRequired"}, (0 if not seal["blockers"] else 2)


def main(argv):
    try:
        if argv == ["record"]:
            result, code = record()
            print(json.dumps(result, sort_keys=True))
            return code
        if len(argv) >= 4 and argv[0] == "_native":
            return native_wrapper(argv[1], argv[2], argv[3:])
        if len(argv) >= 3 and argv[0] == "_wrapper":
            return wrapper(argv[1], argv[2:])
        raise Refusal("Usage: replay_build_owner_v1.py record")
    except (Refusal, OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"schema": SCHEMA, "state": "RecordingOnly", "reason": "Refused",
                          "detail": str(error)}, sort_keys=True), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
