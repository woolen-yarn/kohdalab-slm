"""Exercise a packaged SLM GUI from the separate SDK with no physical output."""

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


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
