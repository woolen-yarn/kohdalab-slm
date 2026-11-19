#!/usr/bin/env python3
"""Generate notebook-parity fixtures without NumPy.

This is a line-by-line port of `calculate_phase_value` and
`convert_phase_value_to_input_vlotage` from SLM.ipynb (2024-12-13 Kerr SLAP
notebook, which includes `multi_wave` and `double_wave`). Python's float `%`,
`//` and `round()` use the same divmod / ties-to-even rules as NumPy's
`np.remainder`, `np.floor_divide` and `np.round`, and `math.radians` is
`x * (pi / 180)` like `np.radians`.

The notebook's `double_wave` hard-codes `num_splits = 1`; here it is a
parameter (`band_width`) so integer band widths can be compared.

Usage: python3 -I tests/reference/notebook_reference.py > tests/reference/notebook_fixtures.json
"""

import hashlib
import json
import math
import sys

TWO_PI = 2 * math.pi


def phase_value(hpx, vpx, dh, dv, periods_px, phi, pattern_type, band_width=1):
    out = []
    phi_rad = math.radians(phi)
    c, s = math.cos(phi_rad), math.sin(phi_rad)
    n = len(periods_px)
    for j in range(vpx):
        y = j - vpx // 2 + dv
        for i in range(hpx):
            x = i - hpx // 2 + dh
            if pattern_type in ("single_wave", "binary_wave"):
                p = periods_px[0]
                x_rot = x * c + y * s
                v = (x_rot / p) * 2 * math.pi
                if pattern_type == "binary_wave":
                    v = 0.0 if (x_rot // (p / 2)) % 2 == 0 else math.pi
            elif pattern_type in ("multi_wave", "double_wave"):
                split = 1 if pattern_type == "multi_wave" else band_width
                if phi == 0:
                    p = periods_px[(int(y) // split) % n]
                    v = (x / p) * 2 * math.pi
                elif phi == 90:
                    p = periods_px[(int(x) // split) % n]
                    v = (y / p) * 2 * math.pi
                else:
                    raise ValueError("multi/double wave only at 0 or 90 degrees")
            else:
                raise ValueError(pattern_type)
            out.append(v % TWO_PI)
    return out


def to_code(phases, phase0, period):
    return [int(round(phase0 + (p / (2 * math.pi)) * period) % period) for p in phases]


def case(name, w, h, dh, dv, periods, phi, kind, band_width=1, expect_match=True, note="",
         reverse_band_order=False):
    codes = to_code(phase_value(w, h, dh, dv, periods, phi, kind, band_width), 136, 888)
    return {
        "name": name,
        "width": w,
        "height": h,
        "center_offset_px": [dh, dv],
        "periods_px": periods,
        "rotation_deg": phi,
        "pattern_type": kind,
        "band_width": band_width,
        "reverse_band_order": reverse_band_order,
        "phase0": 136,
        "code_period": 888,
        "expect_match": expect_match,
        "note": note,
        "codes": codes,
    }


def main():
    w, h, dh, dv = 40, 24, -3, 5
    notebook_order = "Rendered with Reverse Band Order, which reproduces the notebook phi=90 +X band order."
    default_order = (
        "Default order is a proper rotation of the 0-degree layout (-X at 90 degrees); "
        "differs from the notebook unless Reverse Band Order is set."
    )
    cases = [
        case("ramp_0", w, h, dh, dv, [7.5], 0, "single_wave"),
        case("ramp_30", w, h, dh, dv, [7.5], 30, "single_wave"),
        case("ramp_90", w, h, dh, dv, [7.5], 90, "single_wave"),
        case("ramp_137_5", w, h, dh, dv, [11.0], 137.5, "single_wave"),
        case("binary_0", w, h, dh, dv, [6.0], 0, "binary_wave"),
        case("binary_45", w, h, dh, dv, [6.0], 45, "binary_wave"),
        case("binary_90", w, h, dh, dv, [5.0], 90, "binary_wave"),
        case("multi_2_0", w, h, dh, dv, [3.0, 5.0], 0, "multi_wave"),
        case("multi_3_0", w, h, dh, dv, [2.0, 3.0, 7.0], 0, "multi_wave"),
        case("multi_2_90", w, h, dh, dv, [3.0, 5.0], 90, "multi_wave",
             reverse_band_order=True, note=notebook_order),
        case("multi_3_90", w, h, dh, dv, [2.0, 3.0, 7.0], 90, "multi_wave",
             reverse_band_order=True, note=notebook_order),
        case("multi_3_90_default_order", w, h, dh, dv, [2.0, 3.0, 7.0], 90, "multi_wave",
             expect_match=False, note=default_order),
        case("double_w2_0", w, h, dh, dv, [4.0, 9.0], 0, "double_wave", band_width=2),
        case("double_w3_0", w, h, dh, dv, [4.0, 9.0, 6.0], 0, "double_wave", band_width=3),
        case("double_w2_90", w, h, dh, dv, [4.0, 9.0], 90, "double_wave", band_width=2,
             reverse_band_order=True, note=notebook_order),
        case("double_3comp_w2_90", w, h, dh, dv, [4.0, 9.0, 6.0], 90, "double_wave", band_width=2,
             reverse_band_order=True, note=notebook_order),
    ]
    # Full Santec raster with the notebook profile candidate; compared by hash.
    full = to_code(phase_value(1920, 1200, -16, 110, [200.0], 30, "single_wave"), 136, 888)
    digest = hashlib.sha256()
    digest.update((1920).to_bytes(4, "little"))
    digest.update((1200).to_bytes(4, "little"))
    for code in full:
        digest.update(code.to_bytes(2, "little"))
    json.dump(
        {
            "generator": "tests/reference/notebook_reference.py",
            "cases": cases,
            "full_raster": {
                "width": 1920,
                "height": 1200,
                "center_offset_px": [-16, 110],
                "periods_px": [200.0],
                "rotation_deg": 30,
                "pattern_type": "single_wave",
                "phase0": 136,
                "code_period": 888,
                "code_sha256": digest.hexdigest(),
            },
        },
        sys.stdout,
        separators=(",", ":"),
    )
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
