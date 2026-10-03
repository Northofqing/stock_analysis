#!/usr/bin/env python3
"""Closed recording owner. No issuance, accepted include, runtime token or promotion.

Only `record` is public. The fixed adjacent manifest is reviewed source, never
caller input. Cargo and approved generators/filesystem ownership are trusted.
Recordings are diagnostic evidence requiring subsequent independent policy review.
"""
from __future__ import annotations

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
            and policy["profile"] == PROFILE, "PolicyIdentity")
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


def run_streamed(argv, cwd, env, stdout_file, stderr_file, echo=False):
    """Preserve exact child bytes while forwarding both streams without buffering a tree."""
    process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
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
    code = process.wait()
    for thread in threads:
        thread.join()
    require(not failures, "ChildOutputCapture")
    return code


def parse_rustc(args):
    """Finite Cargo call forms, exact original argv retained separately. Not rustc grammar."""
    values = {"--crate-name", "--edition", "--crate-type", "--emit", "--out-dir", "--target",
              "--extern", "--cfg", "--check-cfg", "--cap-lints", "--error-format", "--json",
              "--color", "--sysroot", "--print", "--remap-path-prefix", "--diagnostic-width"}
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


def compiler_environment(env, session, sysroot, *, probe):
    require(not any(env.get(key) for key in FORBIDDEN_ENV), "CompilerEnvironmentInjection")
    expected = sysroot_loader_path(sysroot)
    if not probe:
        prefix = str(session / "target/debug/deps")
        require(":" not in prefix, "SessionLoaderPath")
        expected = prefix + ":" + expected
    # Compare original bytes represented by Python's environment strings. Do not
    # normalize, reorder, remove empty components, or trust ambient defaults.
    require(env.get("DYLD_FALLBACK_LIBRARY_PATH") == expected, "CompilerEnvironmentInjection")


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
    parsed = parse_rustc(args[1:])
    compiler_environment(os.environ, session, inv["sysroot"], probe=parsed["probe"])
    options, codegen = parsed["options"], parsed["codegen"]
    if "--target" in options:
        require(options["--target"] == [TARGET], "WrongTarget")
    if "--sysroot" in options:
        require(options["--sysroot"] == [inv["sysroot"]["root"]], "WrongSysroot")
    cwd = Path.cwd().resolve()
    require(inside(cwd, session), "CompilerCwd")
    target = session / "target"
    externs, package, source, outputs = [], None, None, []
    if not parsed["probe"]:
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
        for value in options.get("--extern", []):
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
    record = {"state": "RecordingOnly", "kind": "Probe" if parsed["probe"] else "Compile",
              "argv_hex": [os.fsencode(value).hex() for value in args], "parsed": parsed,
              "cwd": str(cwd), "source": str(source) if source else None, "package": package,
              "role": "Probe" if parsed["probe"] else ("Target" if "--target" in options else "Host"),
              "environment_hex": {os.fsencode(k).hex(): os.fsencode(v).hex() for k, v in os.environ.items()},
              "externs": externs, "declared_outputs": outputs, "compiler_sha256": file_hash(Path(args[0]))}
    atomic_json(call / "invocation.json", record)
    code = run_streamed(args, cwd, dict(os.environ), call / "stdout.raw", call / "stderr.raw", echo=True)
    record["exit_code"] = code
    record["stdout_sha256"] = file_hash(call / "stdout.raw")
    record["stderr_sha256"] = file_hash(call / "stderr.raw")
    record["outputs"], record["blockers"] = [], []
    if code == 0:
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


def seal_record(session, policy, cargo_exit):
    """Resolve observed graph after Cargo; this is a diagnostic seal, never issuance."""
    events, blockers = cargo_events(session / "cargo.stdout.raw")
    receipts = []
    for directory in sorted((session / "invocations").iterdir()):
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
    output_owners = {}
    for receipt in receipts:
        for output in receipt["outputs"]:
            path = Path(output["path"])
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
        hashes = {}
        for filename in filename_list:
            path = Path(filename)
            if not inside(path, session / "target") or not path.is_file() or path.is_symlink():
                blockers.append("InvalidCargoArtifactPath:" + filename)
            else:
                hashes[filename] = file_hash(path)
        candidates = [r for r in receipts if r["kind"] == "Compile" and r["source"] == str(Path(root).resolve())
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
        artifacts.append(record)
        for filename in filename_list:
            owners = output_owners.setdefault(filename, [])
            if record["invocation_id"] not in owners:
                owners.append(record["invocation_id"])
        if root == str(session / "application/src/lib.rs") and "lib" in target.get("kind", []):
            selected.append(record)
    for event in events:
        if event["reason"] != "build-script-executed":
            continue
        out_dir = event.get("out_dir")
        producers = [a for a in artifacts if a["package_id"] == event.get("package_id")
                     and a["target"].get("kind") == ["custom-build"] and a["executable"] in a["files"]]
        if not isinstance(out_dir, str) or not inside(Path(out_dir), session / "target") or len(producers) != 1:
            blockers.append("UnresolvedBuildScriptProducer"); continue
        generated = {}
        for path in Path(out_dir).rglob("*"):
            if path.is_symlink() or (not path.is_dir() and not path.is_file()):
                blockers.append("GeneratedNonregular:" + str(path)); continue
            if path.is_file():
                generated[path.relative_to(out_dir).as_posix()] = file_hash(path)
        associations.append({"out_dir": str(Path(out_dir).resolve()), "package_id": event["package_id"],
                             "producer_invocation": producers[0]["invocation_id"],
                             "generated_files": generated, "cargo_event": event})
    edges = []
    for receipt in receipts:
        for edge in receipt["externs"]:
            producers = output_owners.get(edge["path"], [])
            if len(producers) != 1:
                blockers.append("UnresolvedExternProducer:" + edge["path"])
            edges.append(dict(edge, consumer=receipt["invocation_id"], producers=producers))
    consumed = []
    for receipt in receipts:
        for output in receipt["outputs"]:
            for value in output.get("dep_info", {}).get("paths", []):
                path = (Path(receipt["cwd"]) / value).resolve()
                owner = None
                for tree in ("application", "vendor"):
                    if inside(path, session / tree):
                        name = path.relative_to(session / tree).as_posix()
                        if name in policy["inventory"][tree]["files"]:
                            owner = {"tree": tree, "relative_path": name}
                if owner is None:
                    matches = [a for a in associations if inside(path, Path(a["out_dir"]))]
                    if len(matches) == 1:
                        owner = {"generated_by": matches[0]["producer_invocation"], "out_dir": matches[0]["out_dir"]}
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
            "owner_sha256": file_hash(session / "owner.json"),
            "policy_sha256": digest(POLICY.read_bytes()),
            "invocations": [{key: r[key] for key in ("invocation_id", "receipt_sha256", "request_sha256", "invocation_sha256", "stdout_sha256", "stderr_sha256")} for r in receipts],
            "cargo_stdout_sha256": file_hash(session / "cargo.stdout.raw"),
            "cargo_stderr_sha256": file_hash(session / "cargo.stderr.raw")}
    atomic_json(session / "record.json", seal)
    return seal


def record():
    policy, policy_hash = load_policy()
    if policy["inventory"] is None:
        return {"schema": SCHEMA, "state": "RecordingOnly", "reason": "MissingInventory",
                "profile": PROFILE, "policy_sha256": policy_hash}, 2
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
    owner = {"schema": SCHEMA, "state": "RecordingOnly", "session": session_id, "profile": PROFILE,
             "policy_sha256": policy_hash, "owner_sha256": file_hash(Path(__file__)), "argv": argv,
             "environment": env, "config_sha256": file_hash(config), "launcher_sha256": file_hash(launcher)}
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
        if len(argv) >= 3 and argv[0] == "_wrapper":
            return wrapper(argv[1], argv[2:])
        raise Refusal("Usage: replay_build_owner_v1.py record")
    except (Refusal, OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"schema": SCHEMA, "state": "RecordingOnly", "reason": "Refused",
                          "detail": str(error)}, sort_keys=True), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
