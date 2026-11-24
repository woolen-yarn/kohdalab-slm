"""Unit tests for check_docs_links.py (standard library only, offline)."""

import tempfile
import unittest
from pathlib import Path

import check_docs_links as cdl

RELEASE = f"https://github.com/{cdl.REPO}/releases"


class SlugTests(unittest.TestCase):
    def test_github_heading_anchors(self):
        self.assertEqual(cdl.github_slug("GUI control API / GUI 制御 API"), "gui-control-api--gui-制御-api")
        self.assertEqual(cdl.github_slug("Lease, release, expiry / 所有権・解放・失効"), "lease-release-expiry--所有権解放失効")
        self.assertEqual(cdl.github_slug("Encoding (Santec 10-bit over RGB)"), "encoding-santec-10-bit-over-rgb")
        self.assertEqual(cdl.github_slug("`--gui-api` and **Stop**"), "--gui-api-and-stop")
        self.assertEqual(cdl.github_slug("[Link](x.md) text"), "link-text")
        self.assertEqual(cdl.github_slug("render_item and _note_"), "render_item-and-note")


class CheckerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.saved_root = cdl.ROOT
        cdl.ROOT = self.root
        (self.root / "docs").mkdir()
        (self.root / "docs" / "guide.md").write_text(
            "# Guide\n\n## Setup / 準備\n\n## Setup / 準備\n\n## `--gui-api` flag\n\n<a id=\"custom\"></a>\n",
            encoding="utf-8",
        )
        (self.root / "icon.png").write_bytes(b"")

    def tearDown(self):
        cdl.ROOT = self.saved_root
        self.tmp.cleanup()

    def run_on(self, text, tags=frozenset({"v1.0.0"}), version="1.1.0", require_tags=True):
        source = self.root / "README.md"
        source.write_text(text, encoding="utf-8")
        checker = cdl.Checker(set(tags), version, require_tags)
        checker.check_file(source)
        return checker.errors

    def test_valid_links_pass(self):
        text = "\n".join(
            [
                '<img src="icon.png" alt="">',
                "[g](docs/guide.md) [s](docs/guide.md#setup--準備) [s2](docs/guide.md#setup--準備-1)",
                "[api](docs/guide.md#--gui-api-flag) [c](docs/guide.md#custom) [d](docs/) [self](#top) ![i](icon.png \"title\")",
                "# Top",
                "[ext](https://example.com/missing) [mail](mailto:x@example.com)",
                f"[main](https://github.com/{cdl.REPO}/blob/main/docs/guide.md#guide)",
                f"[rel]({RELEASE}/tag/v1.0.0) [asset]({RELEASE}/download/v1.0.0/kohdalab-slm-1.0.0-linux-x64.deb)",
                f"[sums]({RELEASE}/download/v1.0.0/SHA256SUMS.txt)",
                # Upcoming version (Cargo.toml) may be linked before it is tagged.
                f"[next]({RELEASE}/download/v1.1.0/KohdaLab-SLM-1.1.0-Setup.exe)",
                "[ref]: docs/guide.md#guide",
                "```sh",
                "[ignored](missing.md)",
                "```",
                "`[ignored](missing.md)`",
            ]
        )
        self.assertEqual(self.run_on(text), [])

    def test_broken_links_are_reported(self):
        text = "\n".join(
            [
                "[a](missing.md)",
                "[b](docs/guide.md#nope)",
                "[c](#nowhere)",
                "[d](../outside.md)",
                f"[e](https://github.com/{cdl.REPO}/tree/main/nothing)",
                f"[f]({RELEASE}/download/v1.0.0/kohdalab-slm-1.0.0-linux-arm64.deb)",
                f"[g]({RELEASE}/download/v1.0.0/kohdalab-slm-0.9.0-linux-x64.deb)",
                f"[h]({RELEASE}/tag/v0.9.0)",
                f"[i]({RELEASE}/tag/latest)",
                '<img src="nope.png">',
                "[ref]: nope.md",
            ]
        )
        errors = self.run_on(text)
        self.assertEqual(len(errors), 11, "\n".join(errors))
        for line, error in enumerate(errors, 1):
            self.assertTrue(error.startswith(f"README.md:{line}:"), error)

    def test_missing_tags(self):
        text = f"[r]({RELEASE}/tag/v1.0.0)"
        self.assertEqual(len(self.run_on(text, tags=(), require_tags=True)), 1)
        self.assertEqual(self.run_on(text, tags=(), require_tags=False), [])


if __name__ == "__main__":
    unittest.main()
