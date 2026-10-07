#!/usr/bin/env python3
"""Validate, preview and build an instructor task set; never publish it."""

import argparse
import getpass
import json
import sys
from pathlib import Path

from problem_bank.task_package import (
    build,
    load_bank,
    preview,
    publish_scan,
    render_prompt,
)


def read_pin():
    """The PIN from a non-echoing prompt, asked twice, or once from stdin."""
    if not sys.stdin.isatty():
        return sys.stdin.readline().rstrip("\r\n")
    pin = getpass.getpass("PIN: ")
    if getpass.getpass("PIN again: ") != pin:
        raise ValueError("the two PINs differ")
    return pin


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for command in ("validate", "render-prompt", "preview", "build"):
        arg = sub.add_parser(command)
        arg.add_argument("--bank", type=Path, required=True)
        # Authoring commands read the bank alone; only a build names the
        # set and version it publishes.
        building = command == "build"
        arg.add_argument("--set-id", required=building, default="preview")
        arg.add_argument("--version", type=int, required=building, default=1)
        if command in {"render-prompt", "preview"}:
            arg.add_argument("--task-id", required=True)
        if building:
            arg.add_argument("--site", required=True)
            arg.add_argument("--output", type=Path, required=True)
    arg = sub.add_parser("publish-scan")
    arg.add_argument("directory", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "publish-scan":
            print(f"Validated {publish_scan(args.directory)} encrypted packages")
        elif args.command == "build":
            directory = build(
                args.bank,
                args.set_id,
                args.version,
                read_pin(),
                args.site,
                args.output,
            )
            print(f"Built {directory}; upload {args.output} and share its index.html")
        else:
            package = load_bank(args.bank, args.set_id, args.version)
            if args.command == "validate":
                print(f"Validated {len(package['problems'])} tasks")
            elif args.command == "render-prompt":
                print(render_prompt(package, args.task_id))
            else:
                print(json.dumps(preview(package, args.task_id), indent=2))
    except (
        OSError,
        ValueError,
        RuntimeError,
        ImportError,
        KeyError,
        TypeError,
    ) as error:
        print(f"task-package: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
