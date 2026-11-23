"""Drive a demo KohdaLab SLM GUI through the common Python SDK.

Usage: python slm_gui_smoke.py <descriptor.json> <patterns-row-item-id>

Run by ``cargo test -p kohdalab-slm python_sdk`` against the real App on the
demo backend. It refuses to run unless the GUI reports simulated output, so
it never shows anything on a real SLM. Uses only the unmodified SDK.
"""

from __future__ import annotations

import sys
import uuid

from kohdalab_control import ControlClient, ControlError

SINGLE_WAVE = {
    "kind": "single_wave",
    "wave": {
        "period": {"value": 100.0, "unit": "px"},
        "waveform": "ramp",
        "initial_phase": {"value": 0.0, "unit": "deg"},
    },
    "geometry": {
        "center_offset_px": [-16, 110],
        "rotation": {"value": 15.0, "unit": "deg"},
    },
}


def expect_error(code, call, *args, **kwargs):
    try:
        call(*args, **kwargs)
    except ControlError as error:
        if error.code != code:
            raise AssertionError(f"expected {code}, got {error.code}: {error}")
        return error
    raise AssertionError(f"expected {code}, call succeeded")


def main(descriptor, item_id):
    slm = ControlClient("kohdalab-slm", endpoint=descriptor, timeout=5.0)
    description = slm.describe()
    for method in ("control.acquire", "control.apply", "control.stop"):
        assert method in description["methods"], method
    assert description["safety"]["optical_state_verified"] is False
    state = slm.status()
    # Safety: demo backend only.
    if not (state["native"]["simulated"] and state["native"]["demo"]):
        raise SystemExit("Refusing to run: the GUI does not report simulated output")
    assert state["ownership"]["owned"] is False

    with slm:
        # A second client is refused while this one holds the lease.
        other = ControlClient("kohdalab-slm", endpoint=descriptor, timeout=5.0)
        expect_error("ownership", other.acquire)

        job = slm.invoke("connect")
        assert job["terminal"] is False and job["cleanup"] == "pending", job
        done = slm.wait(job["job_id"], timeout=20)
        assert done["native"]["receipt"]["uniform_code"] == 0, done

        done = slm.wait(
            slm.invoke("apply", spec={"kind": "uniform", "code": 512}, label="smoke 512")[
                "job_id"
            ],
            timeout=20,
        )
        assert done["native"]["receipt"]["uniform_code"] == 512, done

        done = slm.wait(slm.invoke("apply", spec=SINGLE_WAVE)["job_id"], timeout=20)
        assert done["native"]["receipt"]["uniform_code"] is None, done

        done = slm.wait(slm.invoke("apply", item_id=item_id)["job_id"], timeout=20)
        assert done["native"]["receipt"]["uniform_code"] == 321, done

        # Explicit request ID: a repeated request is answered, not re-run.
        request_id = str(uuid.uuid4())
        first = slm.invoke("reset", request_id=request_id)
        again = slm.invoke("reset", request_id=request_id)
        assert again["job_id"] == first["job_id"] and again["duplicate"] is True
        done = slm.wait(first["job_id"], timeout=20)
        assert done["native"]["reset_code"] == 0
        assert "not verified dark" in done["native"]["reset_meaning"]

        expect_error(
            "validation", slm.invoke, "apply", spec={"kind": "uniform", "code": 5000}
        )
        expect_error(
            "validation",
            slm.invoke,
            "apply",
            spec={"kind": "uniform", "code": 1, "display": 3},
        )

        state = slm.stop_and_wait(timeout=20)
        assert state["native"]["device"] == "disconnected", state
        assert state["ownership"]["owned"] is True, "stop keeps the lease"
    # Context exit released: the SDK followed the release stop job.
    state = slm.status()
    assert state["ownership"]["owned"] is False, state
    assert state["ownership"]["last_revocation"] == "released", state
    assert state["cleanup"] == "not_required", state
    print("KohdaLab SLM GUI smoke: OK")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2])
