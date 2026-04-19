#!/usr/bin/env python3
from __future__ import annotations

import argparse
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
FILES = [
    ROOT / "crates" / "themion-core" / "Cargo.toml",
    ROOT / "crates" / "themion-cli" / "Cargo.toml",
]
VERSION_RE = re.compile(r'^(version\s*=\s*")(\d+\.\d+\.\d+)("\s*)$', re.MULTILINE)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Bump themion crate package versions in Cargo.toml files."
    )
    parser.add_argument(
        "version",
        help="Target semver version, for example 0.6.2",
    )
    return parser.parse_args()


def validate_version(version: str) -> None:
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise SystemExit(f"invalid version '{version}'; expected semver like 0.6.2")


def update_file(path: pathlib.Path, version: str) -> tuple[str | None, bool]:
    original = path.read_text()
    match = VERSION_RE.search(original)
    if not match:
        raise SystemExit(f"could not find package version in {path}")
    current = match.group(2)
    updated = VERSION_RE.sub(rf'\g<1>{version}\g<3>', original, count=1)
    changed = updated != original
    if changed:
        path.write_text(updated)
    return current, changed


def main() -> int:
    args = parse_args()
    validate_version(args.version)

    changed_any = False
    for path in FILES:
        current, changed = update_file(path, args.version)
        status = "updated" if changed else "unchanged"
        print(f"{path.relative_to(ROOT)}: {current} -> {args.version} ({status})")
        changed_any = changed_any or changed

    if not changed_any:
        print("no files changed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
