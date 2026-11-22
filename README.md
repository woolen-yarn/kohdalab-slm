<img src="crates/slm-desktop/assets/icon.png" width="64" height="64" align="right" alt="KohdaLab SLM icon">

# KohdaLab SLM

[![CI](https://github.com/woolen-yarn/kohdalab-slm/actions/workflows/test.yml/badge.svg)](https://github.com/woolen-yarn/kohdalab-slm/actions/workflows/test.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Rust / `egui` desktop app for creating SLM patterns, keeping ordered experiment lists and
switching patterns by hand. First target: **Santec SLM-200** over Windows HDMI/DVI
(extended desktop). macOS builds generate, edit and export but only use the simulated
output.

Santec SLM-200向けのRust製デスクトップアプリです。パターン生成、スイープ、
並べ替えと手動切り替え、CSV／PNG出力に対応します。WindowsではHDMI／DVIで
実機に出力し、macOSではシミュレーションで同じ操作を試せます。

[Documentation / ドキュメント](docs/hardware-checklist.md) · [Report an issue / 問題を報告](https://github.com/woolen-yarn/kohdalab-slm/issues)

Agreed specification: [`kohdalab-slm-design.md`](kohdalab-slm-design.md).

> **Hardware status:** The Windows bench LCOS-SLM display showed a 200 px Single Wave
> with matching Windows desktop readback. See the [bench checklist](docs/hardware-checklist.md)
> for optical verification.

## Release / リリース

Version **1.0.0** is being prepared; it is not yet a published binary release.
See the [release notes](docs/releases/v1.0.0.md) for supported platforms and verification limits.

v1.0.0は公開準備中です。配布ファイルの公開完了まではソースからビルドしてください。

## Workspace / 構成

| Crate | Role |
|---|---|
| `crates/slm-core` | Units (px / µm / µm⁻¹, deg / rad / π), Uniform / Single Wave / Multi-Wave generation, one-axis sweeps, experiment JSON, Santec `Y/X` CSV, heatmap PNG. Phase (`PhaseFrame`) and device codes (`CodeFrame`, `u16`) are separate types. |
| `crates/slm-device` | Device profile, config, display identity checks, 10-bit RGB encoding, demo and Windows backends, and the single serialized device worker. |
| `crates/slm-desktop` | `kohdalab-slm` GUI in the KohdaLab TRKR layout (default 1566 × 900, full-layout minimum width 1566). Left: Config, SLM card on TRKR's device-card grid (Device, Display + Refresh, **Connect | Disconnect | Settings**, Reset Pattern row; **Settings** opens a separate window with pitch, magnification, mount angle, phase0/code period, default band width and center, wavelength, reset code), and a Patterns list shared by every tab. Center: tabs with one form card (pattern inputs only) plus Output/Run, above the **Input Pattern** heatmap (SLM codes of the current input, rendered off the UI thread by the same `render`/`render_item` as a send; a compact colormap selector in the header (**Grayscale** default, **Rainbow** = matplotlib `rainbow`, **Viridis** = matplotlib `viridis` via `colorous`), always on the fixed scale code 0 … device max code with a matching colorbar on the right; the choice is saved in the config as `preview_colormap` (missing or unknown → Grayscale); code range, colormap and frame hash on hover; square pixels; axes follow the image columns/rows, x right and y down, with (0, 0) always at the exact image center on every tab, independent of Center X/Y, rotation and pattern kind, which only change the generated codes; ticks at pixel centers in px, or in sample-plane µm (pitch × magnification) when a period is entered in µm or µm⁻¹; a small open reticle marks the center and is drawn over the image, not into it. Colors never change the codes, CSVs, frame hashes or the sent RGB bits). Right: Log above the applied-condition Field/Value table. `<`/`>` column toggles. |

## Build and run / ビルドと起動

On a screen that can fit only one side panel alongside the center, opening **Device**
or **Log** folds the other panel. The default 1566 × 900 window is fitted to the screen
at startup.

In **Patterns**, **Previous** and **Next** apply the adjacent row relative to the focused
row and stop at the list ends. **Apply Row** applies the focused row. Click to select,
Ctrl/Cmd-click to toggle rows, and Shift-click to select a range. Drag a row to reposition
it, or drag a selected row to move the entire selection in its existing order. **Remove
Selected** deletes the selection, and **Remove All** asks before clearing the list.
With **Auto-apply on click** enabled, a plain row click applies that pattern while
connected. If a send is in progress, only the most recent clicked row waits to be sent.
The toggle starts on in the demo and off in a normal session.
After clicking a Patterns row, the Up and Down keys move the selection and follow
the same auto-apply setting. The outline shows when Patterns has keyboard focus.
In **Run**, **Save CSV…** writes Santec pattern codes and **Save PNG…** writes
the native-size code heatmap for the current Input Pattern in the selected colormap.
Export heatmaps use the same colormap on fixed scales (code 0 … max code, phase 0 … 2π,
sin φ −1 … 1) and record it as `heatmap_colormap` in `manifest.json`. Sweep uses the
selected preview source (**First value** or **List row**). The **Save target** label names the source before saving. The preview header shows **Selected**, **Applying**, and **Applied** separately. Sweep rows show the value and unit in Patterns, with the full item label on hover.

```sh
cargo test --workspace --locked
cargo build --release --locked
```

On Windows, `crates/slm-desktop/build.rs` embeds `assets/icon.ico` and version info into the EXE with `rc.exe` (MSVC; override with `RC`) or `windres` (GNU; override with `WINDRES`). Other platforms need no resource compiler; the window icon comes from `assets/icon.png`.

```sh
kohdalab-slm                       # normal GUI, ~/.kohdalab/slm/config/config.json
kohdalab-slm --config <path>       # normal GUI with another config
kohdalab-slm --demo                # simulated output only, temporary config
kohdalab-slm --demo-wavelength     # demo with wavelength controls open
kohdalab-slm --demo-capture <png> [w h [tab [expanded]]]  # demo check: connect, apply, sweep, step through the list, screenshot own window, close
kohdalab-slm --demo-smoke <dir>    # headless demo vertical slice; writes evidence to a new dir
```

`--demo`, `--demo-capture` and `--demo-smoke` only create the demo backend; they never
enumerate real displays or open an output window.

## Output safety rules (Windows backend)

- Choosing an **Output Display** sends nothing. `Connect` shows the reset pattern
  (uniform raw code, provisional `0`, configurable). `Disconnect` and normal exit send the
  reset pattern, then close the output window.
- Output is **blocked until the SLM input mode is confirmed** in Device (mode, note,
  explicit checkbox). The confirmation is saved for the selected display in `config.json`.
  Confirm the mode again after switching displays or opening an older config whose
  confirmation does not identify a display.
- Before Connect **and every Apply**, displays are re-enumerated
  (`QueryDisplayConfig` / `DisplayConfigGetDeviceInfo`). Output stops and the state becomes
  **Output unknown** if the saved monitor device path is missing or ambiguous, the GDI
  number changed, the display is duplicated, primary, rotated, not 1920 × 1200, GPU-scaled,
  DPI-mismatched, not 60 Hz, not 32-bit on the desktop, HDR/advanced color, inside a
  Remote Desktop session, or moved since Connect. Windows must report the monitor bounds,
  active signal resolution, color mode, and RGB output at 8 bits per channel. Nothing
  is retargeted automatically.
- The output window is borderless, top-most, non-activating, per-monitor-DPI-aware (v2),
  created hidden and drawn 1:1 with `SetDIBitsToDevice`. `WM_DISPLAYCHANGE`,
  `WM_DPICHANGED` or any move hides it at once. Each draw is checked by window geometry and
  a readback of the composed desktop; the pointer is hidden over the window.
- Session changes (including lock/unlock and RDP), suspend/resume, and display off/dim
  notifications invalidate the output and require explicit reconnection. Notification
  registration failure blocks connection. Hiding the output cannot guarantee a black
  physical SLM: Windows may display its own desktop/lock screen. Verify optically.
- Wavelength SDK calls run in a separate helper process, terminated after 15 seconds
  without a reply. A timeout does not prove that the device was unchanged; re-read the
  wavelength and phase before retrying. No automatic retry is performed.
- `Applied` means the backend confirmed the draw (demo: encoding round-trip; Windows:
  desktop readback). It does **not** prove the optical response.

## Encoding (Santec 10-bit over RGB)

**60 Hz** follows SLM-200 manual v14.0 **Table 6 (p. 58)**:
 `R = (c >> 7) << 5`, `G = ((c >> 4) & 7) << 5`,
`B = (c & 15) << 4`. Tests pin 0 → (0,0,0), 1 → (0,0,16), 511 → (96,224,240),
512 → (128,0,0), 1023 → (224,224,240).

**120 Hz (Table 7) is not supported.** The public text extraction does not fix every bit
of the second frame, so the mode cannot be confirmed in the GUI and is refused by
`check_output` and the encoder. Nothing is sent.

## Pattern conventions

- Pixel coordinates follow `SLM.ipynb`: `x = i − W/2 + dx`, `y = j − H/2 + dy`
  (default center `(−16, 110)`). Rotation = pattern angle − profile mount angle.
- **Phase mapping** in Device Settings selects the generated phase-to-code conversion.
  New configs default to **Santec full scale (2π)**. Santec manual v14.0 Figure 42
  describes codes 0–1023 spanning 0–2π. The app wraps generated phase into one cycle,
  scales it over that range, and rounds to the nearest code. Use this mapping with the
  controller calibrated to 2π at the operating wavelength. The rounding convention
  is used by this app and has not been compared to the vendor's pattern generator.
- **Notebook calibration** uses `round(phase0 + φ/(2π)·period) mod period` with NumPy
  ties-to-even (defaults `phase0 = 136`, `period = 888`). Older configs and experiments
  continue to load this mapping. Uniform and imported CSV codes pass through directly.
- Applying a generated phase pattern requires its saved phase mapping to match the
  active output profile, including the Notebook calibration values when selected.
- Notebook parity is tested against a pure-Python port of the notebook formulas
  (`tests/reference/notebook_reference.py` → `notebook_fixtures.json`), including a full
  1920 × 1200 raster hash.
- Multi-Wave bands follow the band coordinate `v = −x sin θ + y cos θ`. **Reverse Band
  Order** counts them along `−v`. That keeps the transform continuous at any angle, and at
  90° it reproduces the notebook's `phi=90` +X band order pixel for pixel (tested,
  including 3 components with 2 px bands). The flag is saved per pattern in experiment
  files and is off by default; files without it load as off.
- `q = 0` gives a coordinate-independent phase (period shown as ∞).

## Files / 保存ファイル

- Config: `~/.kohdalab/slm/config/config.json` (schema 2, reads schema 1): profile, backend, output
  display identity, input-mode confirmation, reset code, default band width.
- Experiment: `<name>.json` (schema 2, reads schema 1) with item IDs, order, entered values and units, the
  generation profile per item and sweep origin. Imported CSVs are copied to
  `<name>.assets/<item-id>.csv` and checked by SHA-256. On load, item IDs must be UUIDs,
  `sha256` must be 64 lowercase hex characters, and asset references must be exactly
  `<dir>.assets/<item-id>.csv` (no absolute paths, `..`, backslashes or drive letters).
  They resolve inside the experiment's directory (symlinks pointing outside are refused).
  Moving the directory, or renaming the JSON with or without its `.assets` directory,
  keeps the references valid.
- Export: `NNN_<item-id>.csv` (Santec `Y/X`), `NNN_<item-id>_{phase,kerr_sin_phase,code}.png`
  (native resolution; raw-code items get the code map only) and `manifest.json`.

## Save protection and session records

Settings refuse to overwrite malformed, future-schema, or foreign application files.
Choose a new Config path if the original cannot be loaded. Unsaved settings are marked
in the Config panel; loading another config requires an explicit discard. List name
changes count as edits. Closing offers Save / Discard / Cancel, and a cancelled or
failed save keeps the window open. List saves also reject incompatible existing files.

Experiment Export runs in the background from a snapshot, with progress and cancellation
between items. Choose a parent folder: each export creates a new `slm-export-<UUID>`
subfolder and never merges into existing exports. Rendering completes in a staging
folder first; the completion manifest is published last. A crash during final copying
can leave an incomplete folder without a manifest. The manifest records schema/app
version, generation profile, pattern source, colormap, and code hashes. Single-pattern
Save CSV / PNG also runs in the background from a snapshot. A single-file save completes
atomically and is not cancellable; closing waits for it. Dialogs remember the last folder
per purpose within the session, and file names derive from the preview caption.

Normal launches append session records to `logs/slm-session-<UUID>.jsonl` alongside
the config directory (default `~/.kohdalab/slm/logs`). Records include UTC timestamps,
apply requests/results, errors, output receipts, profiles, and wavelength/maximum-phase
readback. Demo-backend events are marked simulated; explicit `--demo` sessions do not
persist journals. SDK phase values are recorded as raw values, not assumed to mean 2π.
Journal failures are reported in the GUI. Logs are not automatically rotated.

Closing waits asynchronously for output reset. Failure or an unconfirmed reset keeps
a result dialog open; Close Anyway is an explicit choice. This confirms the software
operation only, not the SLM optical response.

## Editing and keyboard controls

Settings opens in a separate scrollable window with fixed Use Settings / Cancel Settings
buttons. Optical/profile fields are drafts until Use Settings; Config Save persists them.
USB Read/Write and Copy default center to forms are explicitly immediate operations.
The latter changes the Single/Multi forms only and never applies output.

Undo Remove restores the last removed batch, including CSV asset references, while
keeping rows added afterwards. Loading another list clears this undo operation. Add Sweep
selects its first new row, consistent with Add to List; adding or undoing never applies.
With focus outside a text field, Ctrl/Cmd+Enter applies the selected row, Ctrl/Cmd+S saves
the list, and Ctrl/Cmd+Z undoes removal. Existing list Up/Down navigation remains available.
Numeric fields abbreviate long values only for display; focus reveals the full value,
and invalid/empty input retains the last valid value with an explanatory tooltip.

## Not verified / 未検証項目

- Optical response to Windows HDMI/DVI output. The Windows x64 build from
  `x86_64-pc-windows-gnu` ran its GUI demo and passed its demo smoke test on the Windows bench PC.
  A later build also verified Windows desktop readback on the LCOS-SLM display.
- Table 6 on the device itself (the layout comes from the manual, not a bench test).
  120 Hz stays refused until Table 7 is confirmed.
- Physical 1:1 output, orientation, uniform codes 0/1/1023, and the optical response.
- Whether the SLM keeps an image after the output window closes. What happens on RDP
  connect/disconnect, unplug, renumbering, DPI or mode change on real hardware.
- GPU color processing after composition (gamma, Night Light, ICC, dithering), which the
  desktop readback cannot see.
- That the reset code `0` and the notebook calibration match the vendor software.
- Thorlabs EXULUS, USB/SDK output and Python control (later stages).
