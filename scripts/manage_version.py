from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path


SEMVER_RE = re.compile(
    r"^(0|[1-9]\d*)\."
    r"(0|[1-9]\d*)\."
    r"(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


@dataclass(frozen=True)
class Target:
    name: str
    path: Path
    kind: str
    package_name: str | None = None


REPO_ROOT = Path(__file__).resolve().parents[1]
ROOT_VERSION_PATH = REPO_ROOT / "VERSION"
TARGETS = (
    Target("runtime", REPO_ROOT / "assets/runtime.json", "package_json"),
    Target("birding-cli", REPO_ROOT / "cli/Cargo.toml", "cargo_toml"),
    Target("birding-cli lock", REPO_ROOT / "cli/Cargo.lock", "cargo_lock", "bulbul-birding"),
)



def validate_semver(value: str) -> str:
    version = value.strip()
    if not SEMVER_RE.fullmatch(version):
        raise ValueError(f"Invalid semantic version: {value!r}")
    return version


def read_root_version() -> str:
    return validate_semver(ROOT_VERSION_PATH.read_text(encoding="utf-8"))


def write_root_version(version: str) -> None:
    ROOT_VERSION_PATH.write_text(f"{version}\n", encoding="utf-8")


def read_target_version(target: Target) -> str:
    text = target.path.read_text(encoding="utf-8")
    if target.kind == "pyproject":
        data = tomllib.loads(text)
        return str(data["project"]["version"])
    if target.kind == "package_json":
        data = json.loads(text)
        return str(data["version"])
    if target.kind == "cargo_toml":
        data = tomllib.loads(text)
        return str(data["package"]["version"])
    if target.kind == "cargo_lock":
        if not target.package_name:
            raise ValueError(f"Cargo lock target {target.name} is missing package_name")
        data = tomllib.loads(text)
        for package in data.get("package", []):
            if package.get("name") == target.package_name:
                return str(package["version"])
        raise ValueError(f"Could not find package {target.package_name!r} in {target.path}")
    raise ValueError(f"Unsupported target kind: {target.kind}")


def write_target_version(target: Target, version: str) -> bool:
    text = target.path.read_text(encoding="utf-8")

    if target.kind == "pyproject":
        pattern = r'(?m)^version = "[^"]+"$'
        replacement = f'version = "{version}"'
    elif target.kind == "package_json":
        pattern = r'(?m)^(\s*"version":\s*")([^"]+)(",?)$'
        replacement = rf"\g<1>{version}\g<3>"
    elif target.kind == "cargo_toml":
        pattern = r'(?m)^version = "[^"]+"$'
        replacement = f'version = "{version}"'
    elif target.kind == "cargo_lock":
        if not target.package_name:
            raise ValueError(f"Cargo lock target {target.name} is missing package_name")
        package_name = re.escape(target.package_name)
        pattern = rf'(?m)(^\[\[package\]\]\r?\nname = "{package_name}"\r?\nversion = ")[^"]+(")'
        replacement = rf"\g<1>{version}\g<2>"
    else:
        raise ValueError(f"Unsupported target kind: {target.kind}")

    updated_text, replacements = re.subn(pattern, replacement, text, count=1)
    if replacements != 1:
        raise ValueError(f"Could not update version in {target.path}")

    if updated_text == text:
        return False

    target.path.write_text(updated_text, encoding="utf-8")
    return True


def sync_targets(version: str) -> int:
    changed = 0
    for target in TARGETS:
        if write_target_version(target, version):
            changed += 1
            print(f"updated {target.name}: {version}")
        else:
            print(f"ok {target.name}: {version}")
    return changed


def check_targets(version: str) -> int:
    mismatches = []
    for target in TARGETS:
        current = read_target_version(target)
        if current != version:
            mismatches.append((target, current))

    if not mismatches:
        print(f"All versioned targets match root VERSION ({version}).")
        return 0

    print(f"Root VERSION is {version}, but these files differ:", file=sys.stderr)
    for target, current in mismatches:
        relative_path = target.path.relative_to(REPO_ROOT)
        print(f"- {relative_path}: {current}", file=sys.stderr)
    return 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Manage the single semantic version source for the Bulbul birding skill."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("check", help="Verify all project manifests match the root VERSION file.")
    subparsers.add_parser("sync", help="Write the root VERSION value into all project manifests.")

    set_parser = subparsers.add_parser("set", help="Update the root VERSION file and sync all manifests.")
    set_parser.add_argument("version", help="Semantic version to set, e.g. 0.1.1 or 1.0.0-rc.1")

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    try:
        if args.command == "set":
            version = validate_semver(args.version)
            write_root_version(version)
            print(f"updated VERSION: {version}")
            sync_targets(version)
            return 0

        version = read_root_version()
        if args.command == "sync":
            sync_targets(version)
            return 0
        if args.command == "check":
            return check_targets(version)
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 2

    parser.error(f"Unsupported command: {args.command}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
