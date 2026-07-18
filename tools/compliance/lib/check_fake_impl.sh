#!/usr/bin/env bash
# AGENTS.md §2.1/§2.8: high-confidence fake implementation gate.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SRC_DIR="${FAKE_IMPL_SRC_DIR:-$REPO_ROOT/src}"
REQUIRED_TEST="${FAKE_IMPL_REQUIRED_TEST:-$REPO_ROOT/tests/e2e_prediction_verify.rs}"

if [ ! -d "$SRC_DIR" ]; then
    echo "[check_fake_impl] FAIL: 源码目录不存在: $SRC_DIR" >&2
    exit 1
fi
if [ ! -f "$REQUIRED_TEST" ]; then
    echo "[check_fake_impl] FAIL: 必须存在行为回归: $REQUIRED_TEST" >&2
    exit 1
fi

python3 - "$SRC_DIR" <<'PY'
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
action = re.compile(
    r"(?:^|_)(?:verify|save|notify|push|sync|update_result|reconcile)(?:_|$)"
)
fn_start = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^\n)]*\))?)[ \t]+)?"
    r"(?:(?:async|unsafe|const)[ \t]+)*fn[ \t]+([A-Za-z_][A-Za-z0-9_]*)"
    r"(?:[ \t]*<[^\n{;]*>)?[ \t]*\("
)
explicit_fake = re.compile(
    r"\b(?:todo|unimplemented)\s*!|\bpanic\s*!\s*\(\s*[\"r#]*[^\n\"]*"
    r"(?:not implemented|unimplemented|stub)",
    re.IGNORECASE,
)
legacy_fake = re.compile(r"update_[A-Za-z0-9_]*result[^\n;]*0\.0[^\n;]*false")


def mask_non_code(text: str) -> str:
    """Mask strings/comments while preserving byte positions and braces."""
    chars = list(text)
    out = chars.copy()
    i = 0
    state = "code"
    block_depth = 0
    raw_hashes = 0
    while i < len(chars):
        ch = chars[i]
        nxt = chars[i + 1] if i + 1 < len(chars) else ""
        if state == "code":
            if ch == "/" and nxt == "/":
                out[i] = out[i + 1] = " "
                state = "line"
                i += 2
                continue
            if ch == "/" and nxt == "*":
                out[i] = out[i + 1] = " "
                block_depth = 1
                state = "block"
                i += 2
                continue
            if ch == '"':
                out[i] = " "
                state = "string"
                i += 1
                continue
            if ch == "r":
                match = re.match(r'r(#{0,16})"', text[i:])
                if match:
                    raw_hashes = len(match.group(1))
                    for j in range(i, i + len(match.group(0))):
                        out[j] = " "
                    i += len(match.group(0))
                    state = "raw"
                    continue
            i += 1
        elif state == "line":
            if ch == "\n":
                state = "code"
            else:
                out[i] = " "
            i += 1
        elif state == "block":
            out[i] = " "
            if ch == "/" and nxt == "*":
                out[i + 1] = " "
                block_depth += 1
                i += 2
            elif ch == "*" and nxt == "/":
                out[i + 1] = " "
                block_depth -= 1
                i += 2
                if block_depth == 0:
                    state = "code"
            else:
                i += 1
        elif state == "string":
            out[i] = " "
            if ch == "\\":
                if i + 1 < len(chars):
                    out[i + 1] = " "
                i += 2
            elif ch == '"':
                state = "code"
                i += 1
            else:
                i += 1
        else:  # raw string
            out[i] = " "
            terminator = '"' + ("#" * raw_hashes)
            if text.startswith(terminator, i):
                for j in range(i, i + len(terminator)):
                    out[j] = " "
                i += len(terminator)
                state = "code"
            else:
                i += 1
    return "".join(out)


def find_body(masked: str, start: int):
    parens = 1
    i = start
    while i < len(masked) and parens:
        if masked[i] == "(":
            parens += 1
        elif masked[i] == ")":
            parens -= 1
        i += 1
    while i < len(masked) and masked[i] not in "{;":
        i += 1
    if i >= len(masked) or masked[i] == ";":
        return None
    open_brace = i
    depth = 1
    i += 1
    while i < len(masked) and depth:
        if masked[i] == "{":
            depth += 1
        elif masked[i] == "}":
            depth -= 1
        i += 1
    if depth:
        return None
    return open_brace, i - 1


def remove_logging_macros(code: str) -> str:
    pattern = re.compile(
        r"(?:(?:log|tracing)::)?(?:trace|debug|info|warn|error)!\s*\(|"
        r"(?:e?println)!\s*\("
    )
    while True:
        match = pattern.search(code)
        if not match:
            return code
        depth = 1
        i = match.end()
        while i < len(code) and depth:
            if code[i] == "(":
                depth += 1
            elif code[i] == ")":
                depth -= 1
            i += 1
        if depth:
            return code
        if i < len(code) and code[i] == ";":
            i += 1
        code = code[: match.start()] + code[i:]


def logging_only(masked_body: str) -> bool:
    code = remove_logging_macros(masked_body)
    code = re.sub(r"\blet\s+_\s*=\s*[^;]+;", "", code)
    code = re.sub(r"\breturn\s+", "", code)
    code = re.sub(r"\s+", "", code).rstrip(";")
    return code in {
        "",
        "()",
        "true",
        "false",
        "Ok(())",
        "Ok(true)",
        "Ok(false)",
        "Some(())",
        "Default::default()",
    }


findings = []
for path in sorted(root.rglob("*.rs")):
    if any(part in {"target", "tests", "fixtures"} for part in path.parts):
        continue
    raw = path.read_text(encoding="utf-8", errors="replace")
    masked = mask_non_code(raw)
    for match in fn_start.finditer(masked):
        name = match.group(1)
        if name.startswith(("test_", "mock_", "stub_", "fixture_")) or not action.search(name):
            continue
        body_range = find_body(masked, match.end())
        if body_range is None:
            continue
        start, end = body_range
        raw_body = raw[start + 1 : end]
        masked_body = masked[start + 1 : end]
        line = raw.count("\n", 0, match.start()) + 1
        if explicit_fake.search(raw_body):
            findings.append((path, line, name, "todo/unimplemented/stub"))
        elif legacy_fake.search(masked_body):
            findings.append((path, line, name, "zero/default update_result"))
        elif logging_only(masked_body):
            findings.append((path, line, name, "logging-only/literal result"))

if findings:
    print("[check_fake_impl] FAIL: 发现高置信假实现:", file=sys.stderr)
    for path, line, name, reason in findings:
        print(f"  {path}:{line}: {name}: {reason}", file=sys.stderr)
    raise SystemExit(1)

print("[check_fake_impl] PASS: 动作函数无 todo/unimplemented/日志后字面成功")
PY
