#!/usr/bin/env python3
"""Reject new firmware build warnings while auditing pinned SDK/toolchain noise."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
WARNING_MARKERS = ("warning:", "userwarning:", "plugin needed to handle lto object")


def warning_lines(log_text: str) -> list[str]:
    clean = ANSI_ESCAPE.sub("", log_text)
    return [
        line.strip()
        for line in clean.splitlines()
        if any(marker in line.lower() for marker in WARNING_MARKERS)
    ]


def check(log_path: Path, baseline_path: Path) -> tuple[list[str], dict[str, int]]:
    baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    if baseline.get("schema") != 1:
        raise ValueError("unsupported firmware warning baseline schema")

    rules = baseline.get("warnings")
    if not isinstance(rules, list) or not rules:
        raise ValueError("firmware warning baseline has no rules")
    compiled = [(rule, re.compile(rule["pattern"], re.IGNORECASE)) for rule in rules]
    counts = {rule["id"]: 0 for rule, _ in compiled}
    errors: list[str] = []

    for line in warning_lines(log_path.read_text(encoding="utf-8", errors="replace")):
        matches = [(rule, pattern) for rule, pattern in compiled if pattern.fullmatch(line)]
        if len(matches) != 1:
            errors.append(f"unbaselined warning: {line}")
            continue
        rule = matches[0][0]
        counts[rule["id"]] += 1

    for rule, _ in compiled:
        count = counts[rule["id"]]
        maximum = int(rule["maxCount"])
        if count > maximum:
            errors.append(
                f"warning baseline exceeded: {rule['id']}={count}, allowed <= {maximum}"
            )
    return errors, counts


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("log", type=Path)
    parser.add_argument(
        "--baseline",
        type=Path,
        default=Path(__file__).with_name("firmware_warning_baseline.json"),
    )
    args = parser.parse_args()
    errors, counts = check(args.log, args.baseline)
    print("Firmware warning audit:")
    for warning_id, count in counts.items():
        print(f"  {warning_id}: {count}")
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print("Firmware warning audit passed; no new warnings were found.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
