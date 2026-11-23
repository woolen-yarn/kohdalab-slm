"""Exercise a packaged SLM GUI from the separate SDK with no physical output."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bindings" / "control"))
from kohdalab_control import discover
from slm_gui_smoke import main


def verify(binary: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="kohdalab-slm-package-") as home:
        environment = dict(os.environ, HOME=home, USERPROFILE=home)
        directory = Path(home) / ".kohdalab" / "slm" / "run"
        with tempfile.TemporaryFile() as log:
            process = subprocess.Popen(
                [str(binary.resolve()), "--demo", "--gui-api"],
                env=environment, stdout=log, stderr=subprocess.STDOUT,
            )
            try:
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise RuntimeError(f"GUI exited during startup: {process.returncode}")
                    endpoints = discover("kohdalab-slm", directory=directory)
                    if len(endpoints) == 1 and endpoints[0].pid == process.pid:
                        main(endpoints[0], None)
                        return
                    time.sleep(0.05)
                raise TimeoutError("The packaged GUI never answered Python discovery")
            except Exception:
                log.seek(0)
                print(log.read().decode("utf-8", errors="replace"), file=sys.stderr)
                raise
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)


def verify_headless(binary: Path) -> None:
    """The Windows hosted runner has no OpenGL 2 desktop context."""
    with tempfile.TemporaryDirectory(prefix="kohdalab-slm-headless-") as home:
        directory = Path(home) / "smoke"
        env = dict(os.environ, HOME=home, USERPROFILE=home)
        subprocess.run([str(binary.resolve()), "--demo-smoke", str(directory)],
                       env=env, timeout=30, check=True)
        report = json.loads((directory / "smoke-summary.json").read_text(encoding="utf-8"))
        if not report:
            raise RuntimeError("Demo produced no verification results")
        print(json.dumps(report, ensure_ascii=True))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--headless", action="store_true")
    args = parser.parse_args()
    (verify_headless if args.headless else verify)(args.binary)
