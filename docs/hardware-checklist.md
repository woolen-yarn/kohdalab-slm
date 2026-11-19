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
2. Confirm the SLM is the factory 60 Hz version. The 60 Hz bit layout follows manual v14.0 Table 6
   (`slm-device/src/encoding.rs`). 120 Hz is refused by this build.
3. Check that Windows drives the SLM output at approximately 60 Hz. The app accepts
   a difference of less than 0.5 Hz (the bench PC reported 59.96 Hz).
4. Expand **Device**, choose **Windows display (HDMI/DVI)**, select the SLM in
   **Display**, and confirm the input mode with a note of what was checked.
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

## Optional wavelength calibration check

The Windows app uses the Santec USB SDK for **Read Wavelength** and **Set Wavelength**.
Build the optional bridge from an x64 Visual Studio developer command prompt,
with the header that declares the SDK functions available as `SLMFunc.h`:

```cmd
cl /std:c++17 /EHsc /LD /I "C:\path\to\SantecSDK\include" crates\slm-device\vendor\santec_wavelength.cpp /link /OUT:kohdalab-santec-wavelength.dll
```

Place the bridge, the matching x64 `SLMFunc.dll`, and its SDK dependencies beside
the app. Install the Santec USB driver and close the vendor app before opening
the USB device. In KohdaLab SLM, disconnect video output, open **Settings**,
enter the USB device number and read the current wavelength. After recording
that value, enter the test wavelength in integer nm and click **Set Wavelength**.
Wait for **USB readback**. The app carries the current optical phase setting
through the write. Santec describes the calibration in manual v14.0, §5.2.2.14.

At a fixed laser wavelength and alignment, record the original Santec wavelength
and optical phase settings. Apply a fixed binary grating and measure the zero- and
first-order intensities. Repeat with a second supported wavelength setting while
keeping the optical phase setting and pattern codes fixed. Allow the Santec
calibration to finish after each change. Restore the original device settings and
repeat the first measurement to check for drift.
