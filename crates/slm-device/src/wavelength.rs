//! Santec wavelength calibration is a separate USB command from video output.

pub const SANTEC_WAVELENGTH_NM: std::ops::RangeInclusive<u32> = 450..=1600;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct WavelengthReading {
    pub nm: u32,
    /// Bytes returned by the Santec SDK. Their physical unit is not assumed.
    pub phase_bits: Option<u32>,
}

/// SDK calls run in a disposable process so a hung vendor DLL cannot hold the GUI.
#[cfg(windows)]
pub(crate) fn santec_wavelength(
    device: u32,
    requested: Option<u32>,
) -> Result<WavelengthReading, String> {
    use std::os::windows::process::CommandExt;
    let mut child = std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .args([
            "--wavelength-helper",
            &device.to_string(),
            &requested.unwrap_or(0).to_string(),
        ])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Start wavelength helper: {e}"))?;
    wait_helper(&mut child, std::time::Duration::from_secs(15))?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(
            "Wavelength helper failed; device state is unverified. Read again before retrying."
                .into(),
        );
    }
    serde_json::from_slice::<Result<WavelengthReading, String>>(&output.stdout)
        .map_err(|e| format!("Invalid wavelength helper response: {e}"))?
}

#[cfg(any(windows, test))]
fn wait_helper(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Wavelength helper timed out or stopped ({result:?}). Device wavelength/phase is unverified; read again before retrying."
                ));
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn hung_helper_is_killed_and_reaped() {
        let mut child = std::process::Command::new("sleep")
            .arg("10")
            .spawn()
            .unwrap();
        let result = super::wait_helper(&mut child, std::time::Duration::from_millis(30));
        assert!(result.unwrap_err().contains("unverified"));
        assert!(child.try_wait().unwrap().is_some());
    }
}

#[cfg(windows)]
pub fn wavelength_helper(device: u32, requested: Option<u32>) -> Result<WavelengthReading, String> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        LoadLibraryExW,
    };

    struct Library(HMODULE);
    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: This is the handle owned by this guard.
            unsafe { FreeLibrary(self.0) };
        }
    }

    let executable = std::env::current_exe().map_err(|err| err.to_string())?;
    let directory = executable
        .parent()
        .ok_or("Cannot locate the executable directory")?;
    let bridge = directory.join("kohdalab-santec-wavelength.dll");
    let sdk = directory.join("SLMFunc.dll");
    let wide_path = |path: &std::path::Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    if !bridge.is_file() || !sdk.is_file() {
        return Err(
            "Install the Santec wavelength bridge and matching SLMFunc.dll beside the app".into(),
        );
    }
    // SAFETY: The absolute paths remain alive during the calls. The bridge is
    // compiled against the vendor header and owns the Santec API session.
    let handle = unsafe {
        LoadLibraryExW(
            wide_path(&bridge).as_ptr(),
            std::ptr::null_mut::<c_void>(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
    };
    if handle.is_null() {
        return Err(format!(
            "Load wavelength bridge: {}",
            std::io::Error::last_os_error()
        ));
    }
    let library = Library(handle);
    let symbol = unsafe {
        GetProcAddress(library.0, c"kohdalab_santec_wavelength_v2".as_ptr().cast())
    }
    .ok_or(
        "Rebuild the wavelength bridge from this version of KohdaLab SLM (missing v2 entry point)",
    )?;
    type Calibrate = unsafe extern "C" fn(*const u16, u32, u32, *mut u32, *mut u32) -> i32;
    // SAFETY: The named bridge export has the exact ABI declared above.
    let calibrate: Calibrate = unsafe { std::mem::transmute(symbol) };
    let mut observed = 0;
    let mut phase_bits = 0;
    let status = unsafe {
        calibrate(
            wide_path(&sdk).as_ptr(),
            device,
            requested.unwrap_or(0),
            &mut observed,
            &mut phase_bits,
        )
    };
    match status {
        0 if SANTEC_WAVELENGTH_NM.contains(&observed) => Ok(WavelengthReading {
            nm: observed,
            phase_bits: Some(phase_bits),
        }),
        0 => Err(format!("Unexpected device wavelength: {observed} nm")),
        -30001 => Err("Santec SDK DLL could not be loaded".into()),
        -30002 => Err("Santec SDK DLL is missing its wavelength functions".into()),
        -30003 => Err(format!(
            "Write not confirmed. Last USB read: {observed} nm, phase raw 0x{phase_bits:08X}. Read again before retrying."
        )),
        code => Err(format!(
            "Santec SDK returned {code}. Read the current wavelength before retrying."
        )),
    }
}

#[cfg(not(windows))]
pub(crate) fn santec_wavelength(_: u32, _: Option<u32>) -> Result<WavelengthReading, String> {
    Err("Santec USB calibration is available on Windows".into())
}
