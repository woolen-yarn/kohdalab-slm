# Santec SLM-200 bench checklist (Windows, HDMI/DVI)

Use the physical Windows console for the output steps. Record the build identifier (commit
or executable SHA-256), the config file, and the result of each step. Stop at the first
unexpected result.

## Windows preflight

Extract the Windows x64 package. In PowerShell, open the extracted directory and run
the simulated checks:

```powershell
$exe = (Resolve-Path .\kohdalab-slm.exe).Path
$evidence = "slm-smoke-$(Get-Date -Format yyyyMMdd-HHmmss)"
$smoke = Start-Process -FilePath $exe -ArgumentList '--demo-smoke', $evidence -Wait -PassThru
if ($smoke.ExitCode -ne 0) { throw "SLM demo smoke test failed ($($smoke.ExitCode))." }
Get-FileHash $exe -Algorithm SHA256
Start-Process -FilePath $exe -ArgumentList '--demo'
```

In the demo window, open **Settings** and check that the separate window leaves the
Patterns list visible. Click **Connect**, then **Apply** in **Uniform** and check the simulated
status and **Input Pattern**. In **Single Wave**, check that the y-axis title sits to the
left of the tick labels at the height of the center reticle. On a screen that fits only
one side panel, check that opening the Log panel folds the Device panel and vice versa.
In **Sweep**, add a short sweep and check that **Patterns** remains visible when switching
tabs. Check that **Next** and **Previous** apply one row at a time and stop at the ends.
Close the demo before using the physical SLM.

Launch the bench session with a separate config file:

```powershell
& $exe --config (Join-Path $PWD 'slm-200-bench.json')
```

## Before Connect

1. Windows Display settings: **Extend**, SLM not primary, 1920 × 1200, Landscape, 100 %
   scale preferred, HDR off, Night Light off, no color profile on the SLM output.
   In Advanced display, verify the active signal is 1920 × 1200 with RGB at 8 bits
   per channel. Set full RGB range in the GPU controls if that option is available.
2. Confirm the SLM is the factory 60 Hz version. The 60 Hz bit layout follows manual v14.0 Table 6
   (`slm-device/src/encoding.rs`). 120 Hz is refused by this build.
3. Check that Windows drives the SLM output at approximately 60 Hz. The app accepts
   a difference of less than 0.5 Hz (the bench PC reported 59.96 Hz).
4. Expand **Device**, choose **Windows display (HDMI/DVI)**, select the SLM in
   **Display**, and confirm the input mode with a note of what was checked.
   Confirm again if the selected display changes or the saved confirmation predates
   display-specific confirmations.
   Use **Config → Save** to keep the bench config.

On the Windows bench PC, Windows Plug and Play identified the SLM as **Generic Monitor (LCOS-SLM)**
with status **OK**. At the physical console, match **LCOS-SLM** and the current
resolution and refresh rate in the app's Display list. Record the display number there.

## Output

5. Connect: the SLM shows the reset code. Compare with the vendor software's uniform 0.
6. Uniform `0`, `1`, `1023`, then an asymmetric CSV (e.g. the smoke `legacy_santec.csv`
   with the top-left marker). Check orientation on the SLM and the sample-plane fringes.
7. Show a CSV the vendor software also displays; compare optically.
8. Multi-Wave at 90° with Reverse Band Order: compare with a notebook CSV of the same
   condition.

## Failure handling

9. While connected: unplug the SLM, change its resolution, change DPI, renumber displays
   (change the primary), switch to Duplicate. Each must hide the output and show
   **Output unknown**; nothing may appear on the control monitor.
10. Connect via RDP and disconnect it; Apply must be refused while remote.
11. Disconnect and normal exit: the reset code is sent first. Record what the SLM shows
    after the output window closes (desktop or held image).

## Compare with Santec software

Record the operating wavelength and **Optical phase change** in the Santec software
(manual v14.0 §5.2.2.14). Use a controller set to 2π when comparing generated patterns
using **Santec full scale (2π)** in Device Settings. The USB wavelength command in
KohdaLab preserves the controller's current maximum phase, so set that value in
the Santec software first.

Export a pattern as Santec CSV in KohdaLab and open that same CSV in the vendor software
in DVI mode. For a pixel match, disable any image translation, offset, gain, or other
correction in the vendor software. Compare the native-sized outputs and record the
controller settings with the result. Compare the optical diffraction under the same
alignment and polarization if confirming that the codes produce the intended phase.

## Optional wavelength calibration check

The Windows app uses the Santec USB SDK for **Read Wavelength** and **Set Wavelength**.
Build the optional bridge from an x64 Visual Studio developer command prompt,
with the header that declares the SDK functions available as `SLMFunc.h`:

```cmd
cl /std:c++17 /EHsc /LD /I "C:\path\to\SantecSDK\include" crates\slm-device\vendor\santec_wavelength.cpp /link /OUT:kohdalab-santec-wavelength.dll
```

Place the bridge built from this source, the matching x64 `SLMFunc.dll`, and its SDK
dependencies beside the app. Rebuild any existing bridge to provide the v2 entry point. Install the Santec USB driver and close the vendor app before opening
the USB device. In KohdaLab SLM, disconnect video output, open **Settings**,
enter the USB device number and read the current wavelength. After recording
that value, enter the test wavelength in integer nm and click **Set Wavelength**.
Wait for **USB readback**. The app carries the current optical phase setting
through the write and checks both values. The log records the SDK phase bytes as
hexadecimal for comparison between reads. To identify the setting as 2π, check
**Optical phase change** in the vendor app. See Santec manual v14.0, §5.2.2.14.

At a fixed laser wavelength and alignment, record the original Santec wavelength
and optical phase settings. Apply a fixed binary grating and measure the zero- and
first-order intensities. Repeat with a second supported wavelength setting while
keeping the optical phase setting and pattern codes fixed. Allow the Santec
calibration to finish after each change. Restore the original device settings and
repeat the first measurement to check for drift.

## 2026-11-20 changes: release verification still required

These checks are pending on Windows; Mac unit tests and API type checking do not
replace them. Use the bench PC locally so session/sleep changes do not remove your
only way to inspect the output. Record app version, config, session JSONL, and the
SLM optical state.

1. Connect to the verified secondary LCOS-SLM display and apply a known pattern.
2. Lock/unlock Windows, suspend/resume, and turn the display off/on in separate runs.
   Expect output invalidation and no automatic pattern resume. Reconnect explicitly
   between runs. Observe what Windows actually presents to the SLM during lock/off:
   hiding an application window is not a physical beam shutter.
3. Repeat session connect/disconnect and display mode/DPI changes. Confirm the
   application requires deliberate re-selection/reconnection after invalidation.
4. Check USB read, write, and readback with the matching SDK/bridge. Confirm wave-
   length and maximum-phase raw are journaled together. Preserve the known 2π setting.
5. In a development build with a deliberately non-returning helper, confirm the
   parent returns an error after approximately 15 seconds, the helper is reaped,
   no retry is sent, and exit is possible. Do not treat the error as proof that a
   physical write did not happen; re-read before retrying.
6. At 100%, 125%, and 150% scaling, check Japanese names, Settings scrolling and its
   always-visible Use/Cancel controls, Undo Remove, numeric full-value tooltips,
   and text-field shortcut guards. Repeat on a small display.

Notification references:
- https://learn.microsoft.com/en-us/windows/win32/termserv/wm-wtssession-change
- https://learn.microsoft.com/en-us/windows/win32/power/wm-powerbroadcast
- https://learn.microsoft.com/en-us/windows/win32/power/registering-for-power-events
