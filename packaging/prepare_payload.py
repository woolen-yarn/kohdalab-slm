"""Stage a KohdaLab SLM executable, SDK, and dependency notices for packaging."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
SDK_FILES = ("LICENSE", "README.md", "kohdalab_control.py", "pyproject.toml")


def stage(binary: Path, output: Path, target: str) -> None:
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version=1", "--filter-platform", target],
        cwd=ROOT,
    ))
    output.mkdir(parents=True)
    shutil.copy2(binary, output / binary.name)
    shutil.copyfile(ROOT / "LICENSE", output / "LICENSE")
    shutil.copyfile(ROOT / "README.md", output / "README.md")
    sdk = output / "bindings" / "control"
    sdk.mkdir(parents=True)
    for name in SDK_FILES:
        shutil.copyfile(ROOT / "bindings" / "control" / name, sdk / name)
    shutil.copyfile(ROOT / "docs" / "gui-control-api.md", sdk / "GUI-API.md")

    notices = [f"# KohdaLab SLM {version} third-party notices", "", f"Target: `{target}`", "",
               "This inventory covers Cargo dependencies resolved for this target, including build and test dependencies.", ""]
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if package["source"] is None:
            continue
        name = f'{package["name"]}-{package["version"]}'
        directory = Path(package["manifest_path"]).parent
        license_files = {
            file for file in directory.iterdir()
            if file.is_file() and file.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT"))
        }
        if package["license_file"]:
            license_files.add(directory / package["license_file"])
        notices.append(f'- `{name}`: {package["license"] or "see accompanying license"}')
        if license_files:
            destination = output / "third-party-licenses" / name
            destination.mkdir(parents=True)
            for file in sorted(license_files):
                shutil.copyfile(file, destination / file.name)
        else:
            notices[-1] += " (license expression from Cargo metadata; no separate license file in the crate)"
    (output / "THIRD_PARTY_NOTICES.md").write_text("\n".join(notices) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--target", required=True)
    args = parser.parse_args()
    stage(args.binary, args.output, args.target)
