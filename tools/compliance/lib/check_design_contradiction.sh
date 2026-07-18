#!/usr/bin/env bash
# AGENTS.md §2.9 / BR-014 / BR-096: fail-closed threshold contract check.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CONTRACT_PATH="${DESIGN_CONTRACT_PATH:-$REPO_ROOT/config/design_contracts.toml}"

python3 - "$REPO_ROOT" "$CONTRACT_PATH" <<'PY'
import pathlib
import re
import sys
import tomllib

repo = pathlib.Path(sys.argv[1]).resolve()
contract_path = pathlib.Path(sys.argv[2]).resolve()


def fail(message: str) -> None:
    print(f"✗ §2.9 设计矛盾: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_toml(path: pathlib.Path, label: str) -> dict:
    try:
        with path.open("rb") as handle:
            return tomllib.load(handle)
    except FileNotFoundError:
        fail(f"缺少{label}: {path}")
    except (OSError, tomllib.TOMLDecodeError) as exc:
        fail(f"{label}无法解析: {path}: {exc}")


contract_doc = load_toml(contract_path, "机器合同")
contract = contract_doc.get("opportunity_event_risk")
if not isinstance(contract, dict):
    fail("机器合同缺少 [opportunity_event_risk]")

required = (
    "threshold_file",
    "threshold_field",
    "rust_source",
    "rust_score_max_constant",
    "score_min",
    "score_max",
    "insufficient_cap_relation",
)
missing = [name for name in required if name not in contract]
if missing:
    fail(f"机器合同缺少字段: {', '.join(missing)}")

config_path = pathlib.Path(
    __import__("os").environ.get(
        "DESIGN_THRESHOLD_CONFIG", str(repo / str(contract["threshold_file"]))
    )
).resolve()
source_path = pathlib.Path(
    __import__("os").environ.get(
        "DESIGN_SCORE_SOURCE", str(repo / str(contract["rust_source"]))
    )
).resolve()
config = load_toml(config_path, "阈值配置")

field = contract["threshold_field"]
if not isinstance(field, str) or not field:
    fail("threshold_field 必须是非空字符串")
threshold = config.get(field)
if isinstance(threshold, bool) or not isinstance(threshold, int):
    fail(f"配置根级缺少整数字段 {field}")

score_min = contract["score_min"]
score_max = contract["score_max"]
if any(isinstance(value, bool) or not isinstance(value, int) for value in (score_min, score_max)):
    fail("score_min/score_max 必须是整数")
if score_min != 0 or score_max <= score_min:
    fail(f"非法总分值域: {score_min}..={score_max}")
if not 1 <= threshold <= score_max:
    fail(f"推送门 {threshold} 必须位于 1..={score_max}")
if threshold > score_max:
    fail(f"推送门 ({threshold}) > 总分封顶 ({score_max})")

try:
    source = source_path.read_text(encoding="utf-8")
except FileNotFoundError:
    fail(f"缺少 Rust 评分实现: {source_path}")
except OSError as exc:
    fail(f"Rust 评分实现不可读: {source_path}: {exc}")

constant = contract["rust_score_max_constant"]
if not isinstance(constant, str) or not constant:
    fail("rust_score_max_constant 必须是非空字符串")
match = re.search(
    rf"\b(?:pub\s+)?const\s+{re.escape(constant)}\s*:\s*u8\s*=\s*(\d+)\s*;",
    source,
)
if match is None:
    fail(f"Rust 实现缺少常量 {constant}")
rust_score_max = int(match.group(1))
if rust_score_max != score_max:
    fail(f"合同 score_max={score_max} 与 Rust {constant}={rust_score_max} 不一致")

relation = contract["insufficient_cap_relation"]
if relation != "threshold_minus_one":
    fail(f"不支持的数据不足封顶关系: {relation!r}")
if ".checked_sub(1)" not in source or "valid_push_threshold(push_threshold)" not in source:
    fail("Rust 实现未显式实现 threshold_minus_one 与阈值有效性门")

print(
    f"✓ §2.9 阈值合同对齐: {field}={threshold}, "
    f"score={score_min}..={score_max}, insufficient_cap={threshold - 1}"
)
PY
