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
    if kind in {"LibcBuildVersion", "ProcMacro2FeatureProbe", RUSTIX_KIND} | AUTOCFG_KINDS | RECORD10_KINDS:
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


def native_wrapper(session_id, role, args):
    require(ID.fullmatch(session_id),"SessionId")
    session=SESSIONS/("pending-"+session_id)
    require(session.is_dir() and session.resolve()==session and not session.is_symlink(),"MissingSession")
    policy=strict_json(POLICY.read_bytes());owner=strict_json((session/"owner.json").read_bytes())
    call,r=native_operation(session,policy,owner,role,args,Path.cwd(),dict(os.environ))
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
    special = record10_context(args, os.environ, cwd, session, inv)
    if special is None:
        special = rustix_context(args, os.environ, cwd, session, inv)
    if special is None and policy["profile"] == BUNDLED_PROFILE:
        native_control(session, policy, owner)
        special = sqlite_rust_context(args, os.environ, cwd, session, inv)
    if special is None:
        special = ring_static_context(args, os.environ, cwd, session, inv)
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
    transient = context["kind"] in {"ProcMacro2FeatureProbe", "ThiserrorStaticFeatureProbe"} or autocfg_stdin or rustix_stdin
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
    atomic_json(call / "invocation.json", record)
    stdin_failures = []
    code = run_streamed(args, cwd, dict(os.environ), call / "stdout.raw", call / "stderr.raw",
                        echo=True, pass_fds=jobserver_fds, stdin_bytes=stdin_bytes,
                        stdin_failures=stdin_failures)
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
    if transient:
        try:
            if rustix_stdin:
                post = metadata_state(call, Path(outputs[0]["path"]), "post")
                record["metadata_post"] = post
                if post["exists"]:
                    record["outputs"] = [dict(outputs[0], sha256=post["sha256"], snapshot=post["snapshot"], observation_only=True)]
                elif code == 0:
                    record["blockers"].append("MissingDeclaredOutput:" + outputs[0]["path"])
            elif context["kind"] == "ThiserrorStaticFeatureProbe":
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
    for directory in sorted((session / "invocations").iterdir()):
        # Reconstruct closed names even for request-only failures, before any
        # full qualification or mutable receipt annotation can be consulted.
        try:
            request = strict_json((directory / "request.json").read_bytes())
            raw = [os.fsdecode(bytes.fromhex(a)) for a in request["argv_hex"]]
            env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in request["environment_hex"].items()}
            cwd = Path(os.fsdecode(bytes.fromhex(request["cwd_hex"])))
            record10_transient_paths.update(record10_transient_namespace(raw, env, cwd, session))
        except (Refusal, OSError, KeyError, IndexError, TypeError, ValueError) as error:
            blockers.append("Record10Namespace:" + str(error))
            record10_invalid_calls.add(directory.name)
        if not (directory / "receipt.json").is_file():
            blockers.append("IncompleteInvocation:" + directory.name); continue
        receipt = strict_json((directory / "receipt.json").read_bytes())
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
                       for o in r["declared_outputs"] + r["outputs"]} | record10_transient_paths
    native_paths, native_probes = set(), set()
    if policy["profile"] == BUNDLED_PROFILE:
        native_paths, native_probes, namespace_blockers = native_evidence_paths(session)
        blockers.extend(namespace_blockers)
    output_owners, transient_collisions = {}, set()
    for receipt in receipts:
        if receipt["invocation_id"] in record10_invalid_calls:
            transient_collisions.add(receipt["invocation_id"])
            continue
        if native_paths and ((isinstance(receipt.get("source"), str) and native_path_identity(receipt["source"], receipt) in native_paths)
                or any(native_path_identity(o["path"], receipt) in native_paths
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
        forbidden = [f for f in filename_list if str(Path(f).resolve()) in transient_paths]
        if forbidden:
            blockers.extend("TransientCargoArtifact:" + f for f in forbidden)
            continue
        # A relative Cargo filename has only the actual requesting receipt's cwd.
        # No candidate may claim a native namespace through an alternate spelling.
        if native_paths:
            native_candidates = [r for r in receipts if isinstance(r.get("source"), str)
                                 and native_path_identity(r["source"], r) == native_path_identity(root, r)
                                 and package_id(r["package"], session) == event.get("package_id")]
            if any(native_path_identity(f, r) in native_paths for r in native_candidates for f in filename_list):
                blockers.append("NativeOutputRole:CargoArtifact")
                continue
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
            if path.is_file() and str(path.resolve()) not in transient_paths | native_probes:
                generated[path.relative_to(out_dir).as_posix()] = file_hash(path)
        associations.append({"out_dir": str(Path(out_dir).resolve()), "package_id": event["package_id"],
                             "producer_invocation": producers[0]["invocation_id"],
                             "generated_files": generated, "cargo_event": event})
    nested_origins = []
    for receipt in receipts:
        context = receipt.get("context", {})
        if context.get("kind") in {"LibcBuildVersion", "ProcMacro2FeatureProbe", RUSTIX_KIND} | AUTOCFG_KINDS | RECORD10_KINDS:
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
            if native_paths and native_path_identity(edge["path"], receipt) in native_paths:
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
    consumed = []
    for receipt in receipts:
        if receipt["invocation_id"] in transient_collisions:
            continue
        for output in receipt["outputs"]:
            for value in output.get("dep_info", {}).get("paths", []):
                path = (Path(receipt["cwd"]) / value).resolve()
                if str(path) in native_paths:
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
