#!/usr/bin/env bash
# AGENTS §2.10 — canonical business-rule registry integrity gate.

set -euo pipefail

CANONICAL_FILE="${RULES_FILE:-docs/business_rules.md}"
LEGACY_FILE="${LEGACY_RULES_FILE:-docs/业务规则清单-registry.md}"
BASE_REF="${BASE_REF:-HEAD}"
WORK="$(mktemp -d -t business_rules.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT
FAIL="$WORK/fail"
WARN="$WORK/warn"
RECORDS="$WORK/records.tsv"
: >"$FAIL"
: >"$WARN"
: >"$RECORDS"

fail() { printf '✗ %s\n' "$*" >>"$FAIL"; }
warn() { printf '⚠ %s\n' "$*" >>"$WARN"; }

for file in "$CANONICAL_FILE" "$LEGACY_FILE"; do
    if [ ! -f "$file" ]; then
        fail "§2.10 registry missing: $file"
    fi
done
if [ -s "$FAIL" ]; then
    cat "$FAIL" >&2
    exit 1
fi

for category in 'dedup|去重' 'mutex|互斥' 'filter|过滤' 'sort|排序' 'limit|限额'; do
    if ! grep -Eiq "$category" "$CANONICAL_FILE"; then
        fail "§2.10 canonical registry missing category: $category"
    fi
done

# Canonical columns: id | status | intent | code.
awk -F'|' -v source="$CANONICAL_FILE" '
  /^\| BR-[0-9]+ / {
    id=$2; status=$3; intent=$4; code=$5;
    gsub(/^ +| +$/, "", id); gsub(/^ +| +$/, "", status);
    gsub(/^ +| +$/, "", intent); gsub(/^ +| +$/, "", code);
    print id "\t" status "\t" intent "\t" code "\t" source;
  }
' "$CANONICAL_FILE" >>"$RECORDS"

# Historical columns: id | category | rule | code | tests | review date.
awk -F'|' -v source="$LEGACY_FILE" '
  /^\| BR-[0-9]+ / {
    id=$2; intent=$4; code=(id ~ /BR-012/ ? $8 : $5);
    gsub(/^ +| +$/, "", id); gsub(/^ +| +$/, "", intent);
    status=($0 ~ /待实现/ ? "legacy-pending" : "legacy-active");
    print id "\t" status "\t" intent "\t" code "\t" source;
  }
' "$LEGACY_FILE" >>"$RECORDS"

if [ ! -s "$RECORDS" ]; then
    fail "§2.10 no BR rows parsed"
fi

cut -f1 "$RECORDS" | sort | uniq -d >"$WORK/duplicate_ids"
while IFS= read -r id; do
    [ -z "$id" ] && continue
    meanings="$(awk -F'\t' -v id="$id" '$1==id {print $3 " [" $5 "]"}' "$RECORDS" | paste -sd ' || ' -)"
    fail "§2.10 duplicate business-rule ID $id: $meanings"
done <"$WORK/duplicate_ids"

extract_paths() {
    awk -F'`' '{ for (i=2; i<=NF; i+=2) print $i }' | while IFS= read -r path; do
        path="${path%%::*}"
        path="${path%%:*}"
        path="${path%% *}"
        case "$path" in
            */*) printf '%s\n' "$path" ;;
        esac
    done
}

while IFS=$'\t' read -r id status _intent code source; do
    if [[ "$status" == *spec-only* || "$status" == legacy-pending ]]; then
        if [ "$status" = legacy-pending ]; then
            warn "§2.10 $id remains explicitly pending in $source"
        fi
        while IFS= read -r path; do
            case "$path" in
                docs/*) [ -e "$path" ] || fail "§2.10 $id spec path missing: $path" ;;
            esac
        done < <(printf '%s\n' "$code" | extract_paths)
        continue
    fi

    path_count=0
    while IFS= read -r path; do
        [ -z "$path" ] && continue
        path_count=$((path_count + 1))
        if [ ! -e "$path" ]; then
            fail "§2.10 $id active path missing: $path ($source)"
            continue
        fi
        if [ "$source" = "$CANONICAL_FILE" ] && ! grep -q "$id" "$path" 2>/dev/null; then
            if ! git cat-file -e "$BASE_REF:$path" 2>/dev/null; then
                fail "§2.10 new active path $path does not cite $id"
            else
                warn "§2.10 active path $path does not cite $id"
            fi
        fi
    done < <(printf '%s\n' "$code" | extract_paths)
    if [ "$path_count" -eq 0 ]; then
        fail "§2.10 $id active rule has no parseable code path ($source)"
    fi
done <"$RECORDS"

cat "$FAIL" >&2
cat "$WARN" >&2
FAIL_COUNT="$(wc -l <"$FAIL" | tr -d ' ')"
WARN_COUNT="$(wc -l <"$WARN" | tr -d ' ')"
RULE_COUNT="$(wc -l <"$RECORDS" | tr -d ' ')"
if [ "$FAIL_COUNT" -ne 0 ]; then
    echo "✗ §2.10 business-rule gate failed ($FAIL_COUNT errors, $WARN_COUNT warnings)" >&2
    exit 1
fi
echo "✓ §2.10 business-rule gate passed (rules: $RULE_COUNT, warnings: $WARN_COUNT)"
