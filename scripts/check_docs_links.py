#!/usr/bin/env python3
"""Offline check of links in the repository's Markdown documentation.

Standard library only; makes no network requests, so it cannot flake on
GitHub or vendor outages. It checks:

* relative links and images (Markdown inline, reference definitions and HTML
  ``href``/``src``): the target file or directory must exist in the checkout;
* ``#fragment`` links into Markdown files (same file or another one): the
  fragment must match a heading anchor as GitHub generates it, or an explicit
  ``id``/``name`` anchor;
* absolute ``github.com/<repo>/blob|tree/main/...`` links to this repository:
  treated like relative links;
* release links ``github.com/<repo>/releases/tag/vX.Y.Z`` and
  ``.../releases/download/vX.Y.Z/<asset>``: the asset name must be one the
  packaging workflow publishes for that version, and the tag must exist in the
  local clone (``git tag``). A tag that does not exist yet is accepted only for
  the version currently in ``Cargo.toml``, so a release-preparation PR can
  point at the upcoming release before it is tagged.

Other external URLs (``http(s)``, ``mailto``) are not fetched.

Usage: ``python3 scripts/check_docs_links.py [--require-tags] [FILE ...]``.
Without files, every ``*.md`` outside build directories is checked.
``--require-tags`` fails if the clone has no tags at all (shallow CI checkout)
instead of skipping the tag check with a warning.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tomllib
import unicodedata
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parent.parent
REPO = "woolen-yarn/kohdalab-slm"
SKIP_DIRS = {".git", "target", "payload", "dist", "portable", "extracted", "node_modules"}

# Asset names published per release; keep in sync with
# .github/workflows/package.yml (Windows) and packaging/package_unix.sh.
# SHA256SUMS.txt is written when the release is assembled from those packages.
RELEASE_ASSETS = (
    "KohdaLab-SLM-{v}-Setup.exe",
    "kohdalab-slm-{v}-windows-x64-portable.zip",
    "kohdalab-slm-{v}-macos-arm64.dmg",
    "kohdalab-slm-{v}-macos-arm64-portable.zip",
    "kohdalab-slm-{v}-linux-x64.deb",
    "kohdalab-slm-{v}-linux-x64-portable.tar.gz",
    "SHA256SUMS.txt",
)

FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
HEADING = re.compile(r"^ {0,3}(#{1,6})[ \t]+(.*?)[ \t]*#*[ \t]*$")
INLINE_LINK = re.compile(r"!?\[(?:[^\[\]]|\[[^\]]*\])*\]\(\s*(<[^>]*>|[^\s()]*(?:\([^\s()]*\)[^\s()]*)*)(?:\s+(?:\"[^\"]*\"|'[^']*'|\([^)]*\)))?\s*\)")
REF_DEF = re.compile(r"^ {0,3}\[[^\]]+\]:\s*(<[^>]*>|\S+)")
HTML_ATTR = re.compile(r"""<[a-zA-Z][^>]*?\s(?:href|src)\s*=\s*["']([^"']*)["']""")
HTML_ANCHOR = re.compile(r"""<[a-zA-Z][^>]*?\s(?:id|name)\s*=\s*["']([^"']+)["']""")
CODE_SPAN = re.compile(r"(`+)(?:(?!\1).)+?\1")
RELEASE = re.compile(
    rf"^https://github\.com/{re.escape(REPO)}/releases/(?:tag/(?P<tag>[^/?#]+)/?|download/(?P<dtag>[^/?#]+)/(?P<asset>[^/?#]+))$"
)
BLOB = re.compile(rf"^https://github\.com/{re.escape(REPO)}/(?:blob|tree)/main/(?P<path>[^?#]*)")


def lines_outside_code(text: str):
    """Yield ``(line_number, raw_line, line_with_code_spans_blanked)`` for
    every line outside fenced code blocks."""
    fence = None
    for number, line in enumerate(text.splitlines(), 1):
        match = FENCE.match(line)
        if fence:
            if match and match.group(1)[0] == fence[0] and len(match.group(1)) >= len(fence):
                fence = None
            continue
        if match:
            fence = match.group(1)
            continue
        yield number, line, CODE_SPAN.sub(lambda m: " " * len(m.group(0)), line)


def github_slug(heading: str) -> str:
    """GitHub heading anchor: rendered text, lowercased, punctuation dropped."""
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", heading)  # links/images → text
    text = re.sub(r"<[^>]+>", "", text)  # inline HTML
    text = text.replace("`", "")
    # Emphasis markers; intraword underscores (snake_case) are literal text.
    text = re.sub(r"(\*{1,3})(\S(?:.*?\S)?)\1", r"\2", text)
    text = re.sub(r"(?<!\w)(_{1,3})(\S(?:.*?\S)?)\1(?!\w)", r"\2", text)
    text = text.strip().lower()
    kept = []
    for char in text:
        category = unicodedata.category(char)
        if char in "-_ " or category[0] in "LNM" or category == "Pc":
            kept.append("-" if char == " " else char)
    return "".join(kept)


def anchors(path: Path, cache: dict[Path, set[str]]) -> set[str]:
    if path not in cache:
        found: set[str] = set()
        counts: dict[str, int] = {}
        text = path.read_text(encoding="utf-8")
        for _, raw, line in lines_outside_code(text):
            # Code spans count toward the heading anchor text.
            heading = HEADING.match(raw)
            if heading:
                slug = github_slug(heading.group(2))
                n = counts.get(slug, 0)
                counts[slug] = n + 1
                found.add(slug if n == 0 else f"{slug}-{n}")
            found.update(HTML_ANCHOR.findall(line))
        cache[path] = found
    return cache[path]


def links(text: str):
    for number, _, line in lines_outside_code(text):
        for match in INLINE_LINK.finditer(line):
            yield number, match.group(1)
        ref = REF_DEF.match(line)
        if ref:
            yield number, ref.group(1)
        for match in HTML_ATTR.finditer(line):
            yield number, match.group(1)


def git_tags() -> set[str] | None:
    try:
        out = subprocess.run(
            ["git", "-C", str(ROOT), "tag", "--list"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    return {t.strip() for t in out.splitlines() if t.strip()}


def workspace_version() -> str:
    with open(ROOT / "Cargo.toml", "rb") as f:
        return tomllib.load(f)["workspace"]["package"]["version"]


class Checker:
    def __init__(self, tags: set[str] | None, version: str, require_tags: bool):
        self.tags = tags
        self.version = version
        self.require_tags = require_tags
        self.cache: dict[Path, set[str]] = {}
        self.errors: list[str] = []
        self.warned_tags = False

    def error(self, source: Path, line: int, target: str, message: str):
        rel = source.relative_to(ROOT) if source.is_relative_to(ROOT) else source
        self.errors.append(f"{rel}:{line}: {target}: {message}")

    def check_local(
        self, source: Path, line: int, target: str, path: str, fragment: str, base: Path | None = None
    ):
        base = source.parent if base is None else base
        resolved = (base / unquote(path)).resolve() if path else source
        if path and not resolved.is_relative_to(ROOT):
            self.error(source, line, target, "points outside the repository")
            return
        if not resolved.exists():
            self.error(source, line, target, "target does not exist")
            return
        if fragment and resolved.suffix.lower() == ".md" and resolved.is_file():
            if unquote(fragment).lower() not in {a.lower() for a in anchors(resolved, self.cache)}:
                self.error(source, line, target, f"no heading or anchor #{fragment}")

    def check_release(self, source: Path, line: int, target: str, match: re.Match):
        tag = match.group("tag") or match.group("dtag")
        if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", tag):
            self.error(source, line, target, f"release tag {tag!r} is not vX.Y.Z")
            return
        version = tag[1:]
        asset = match.group("asset")
        if asset is not None:
            expected = {name.format(v=version) for name in RELEASE_ASSETS}
            if unquote(asset) not in expected:
                self.error(
                    source,
                    line,
                    target,
                    f"{asset!r} is not a packaged asset of {tag} (expected one of: {', '.join(sorted(expected))})",
                )
        if self.tags is None or not self.tags:
            if self.require_tags:
                self.error(source, line, target, "no git tags available to verify the release (fetch tags)")
            elif not self.warned_tags:
                print("warning: no git tags in this clone; release tags not verified", file=sys.stderr)
                self.warned_tags = True
            return
        if tag not in self.tags and version != self.version:
            self.error(
                source,
                line,
                target,
                f"tag {tag} does not exist and is not the upcoming version {self.version}",
            )

    def check_file(self, source: Path):
        text = source.read_text(encoding="utf-8")
        for line, raw in links(text):
            target = raw[1:-1] if raw.startswith("<") and raw.endswith(">") else raw
            if not target:
                self.error(source, line, raw or "()", "empty link")
                continue
            release = RELEASE.match(target)
            if release:
                self.check_release(source, line, target, release)
                continue
            blob = BLOB.match(target)
            if blob:
                fragment = urlsplit(target).fragment
                path = unquote(blob.group("path")) or "."
                self.check_local(source, line, target, path, fragment, base=ROOT)
                continue
            parts = urlsplit(target)
            if parts.scheme or target.startswith("//"):
                continue  # external: not fetched
            self.check_local(source, line, target, parts.path, parts.fragment)


def markdown_files() -> list[Path]:
    found = []
    for path in sorted(ROOT.rglob("*.md")):
        if not SKIP_DIRS.intersection(path.relative_to(ROOT).parts):
            found.append(path)
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("files", nargs="*", type=Path)
    parser.add_argument("--require-tags", action="store_true")
    args = parser.parse_args(argv)
    files = [f.resolve() for f in args.files] or markdown_files()
    checker = Checker(git_tags(), workspace_version(), args.require_tags)
    for path in files:
        checker.check_file(path)
    for error in checker.errors:
        print(error)
    print(f"Checked {len(files)} Markdown files: {len(checker.errors)} problem(s).")
    return 1 if checker.errors else 0


if __name__ == "__main__":
    sys.exit(main())
