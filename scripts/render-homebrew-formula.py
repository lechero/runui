#!/usr/bin/env python3
"""Render the pinned Homebrew formula for a commit or release tag."""

import re
import sys


def main() -> None:
    if len(sys.argv) not in (3, 4):
        raise SystemExit("usage: render-homebrew-formula.py VERSION COMMIT [TAG]")
    version, commit = sys.argv[1:3]
    tag = sys.argv[3] if len(sys.argv) == 4 else None
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
        raise SystemExit("invalid version")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise SystemExit("invalid commit")
    if tag is not None and tag != f"v{version}":
        raise SystemExit("tag must match version")
    source = f'      tag: "{tag}",\n' if tag else ""
    print(
        f'''class Runui < Formula
  desc "Project commands and persistent runs in your terminal"
  homepage "https://github.com/lechero/runui"
  url "https://github.com/lechero/runui.git",
{source}      revision: "{commit}"
  version "{version}"
  license "MIT"

  head "https://github.com/lechero/runui.git", branch: "main"

  depends_on "rust" => :build

  def fetch
    system "cargo", "fetch", *std_cargo_fetch_args
  end

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/runui")
  end

  test do
    assert_match "runui keeps a folder", shell_output("#{{bin}}/runui help")
  end
end'''
    )


if __name__ == "__main__":
    main()
