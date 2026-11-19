//! Santec wavelength calibration is a separate USB command from video output.

pub const SANTEC_WAVELENGTH_NM: std::ops::RangeInclusive<u32> = 450..=1600;

#[cfg(windows)]
pub(crate) fn santec_wavelength(device: u32, requested: Option<u32>) -> Result<u32, String> {
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
    let symbol =
        unsafe { GetProcAddress(library.0, c"kohdalab_santec_wavelength".as_ptr().cast()) }
            .ok_or("The wavelength bridge is missing its entry point")?;
    type Calibrate = unsafe extern "C" fn(*const u16, u32, u32, *mut u32) -> i32;
    // SAFETY: The named bridge export has the exact ABI declared above.
    let calibrate: Calibrate = unsafe { std::mem::transmute(symbol) };
    let mut observed = 0;
    let status = unsafe {
        calibrate(
            wide_path(&sdk).as_ptr(),
            device,
            requested.unwrap_or(0),
            &mut observed,
        )
    };
    match status {
        0 if SANTEC_WAVELENGTH_NM.contains(&observed) => Ok(observed),
        0 => Err(format!("Unexpected device wavelength: {observed} nm")),
        -30001 => Err("Santec SDK DLL could not be loaded".into()),
        -30002 => Err("Santec SDK DLL is missing its wavelength functions".into()),
        -30003 => Err("Wavelength write was sent but readback did not confirm it".into()),
        code => Err(format!(
            "Santec SDK returned {code}. Read the current wavelength before retrying."
        )),
    }
}

#[cfg(not(windows))]
pub(crate) fn santec_wavelength(_: u32, _: Option<u32>) -> Result<u32, String> {
    Err("Santec USB calibration is available on Windows".into())
}
