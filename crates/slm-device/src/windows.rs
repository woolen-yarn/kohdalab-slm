//! Windows HDMI/DVI output: extended desktop, one borderless top-most window
//! covering exactly the chosen monitor, drawn 1:1 with `SetDIBitsToDevice`.
//!
//! Safety behavior:
//! - enumeration and drawing run per-monitor-DPI-aware (v2), so all sizes are
//!   physical pixels and no DPI virtualization scales the frame;
//! - the window is created hidden and first shown with the reset frame;
//! - `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, `WM_SETTINGCHANGE` or any move away
//!   from the expected rectangle hides the window immediately and reports an
//!   invalidation; the worker then requires reselect/reconnect;
//! - each show is checked by geometry and a readback of the composed desktop.
//!
//! Not verified on hardware: whether the SLM keeps an image after the window
//! closes, behavior under RDP connect/disconnect, GPU color processing after
//! composition (gamma, Night light, dithering), and the Santec bit layout.

use crate::backend::{BackendKind, OutputBackend, OutputTarget, Verification};
use crate::display::{Connector, DisplayInfo, Rect};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_MODE_INFO_TYPE_TARGET,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EXTERNAL, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_ROTATION_IDENTITY,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::DwmFlush;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DEVMODEW, DIB_RGB_COLORS, DeleteDC, DeleteObject, ENUM_CURRENT_SETTINGS, EndPaint,
    EnumDisplayMonitors, EnumDisplaySettingsW, GdiFlush, GetDC, GetMonitorInfoW, HDC, HMONITOR,
    InvalidateRect, MONITOR_DEFAULTTONULL, MONITORINFO, MONITORINFOEXW, MonitorFromWindow,
    PAINTSTRUCT, ReleaseDC, SRCCOPY, SelectObject, SetDIBitsToDevice, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, GetWindowRect, HWND_TOPMOST, IsWindowVisible, MA_NOACTIVATE,
    MONITORINFOF_PRIMARY, MSG, PostQuitMessage, RegisterClassExW, SM_REMOTESESSION, SW_HIDE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SendMessageW, SetCursor, SetWindowPos, ShowWindow,
    TranslateMessage, WM_APP, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND,
    WM_MOUSEACTIVATE, WM_PAINT, WM_SETCURSOR, WM_SETTINGCHANGE, WM_WINDOWPOSCHANGED, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const WM_APP_SHOW: u32 = WM_APP + 1;
const WM_APP_CLOSE: u32 = WM_APP + 2;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Run `f` with this thread per-monitor-DPI-aware (v2), restoring afterwards.
fn with_physical_pixels<T>(f: impl FnOnce() -> T) -> T {
    // SAFETY: documented pseudo-handle; returns the previous context or null.
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let result = f();
    if !previous.is_null() {
        // SAFETY: restoring the context returned above.
        unsafe { SetThreadDpiAwarenessContext(previous) };
    }
    result
}

struct Monitor {
    gdi_name: String,
    rect: Rect,
    primary: bool,
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> i32 {
    // SAFETY: `data` is the `&mut Vec<Monitor>` passed to EnumDisplayMonitors
    // and lives for the whole enumeration.
    let monitors = unsafe { &mut *(data as *mut Vec<Monitor>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: MONITORINFOEXW starts with MONITORINFO and cbSize says EX.
    if unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    } != 0
    {
        let r = info.monitorInfo.rcMonitor;
        monitors.push(Monitor {
            gdi_name: from_wide(&info.szDevice),
            rect: Rect {
                x: r.left,
                y: r.top,
                width: (r.right - r.left).max(0) as u32,
                height: (r.bottom - r.top).max(0) as u32,
            },
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    1
}

fn monitors() -> Vec<Monitor> {
    let mut monitors: Vec<Monitor> = Vec::new();
    // SAFETY: callback only dereferences `data` as the vector above.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            &mut monitors as *mut Vec<Monitor> as LPARAM,
        );
    }
    monitors
}

fn query_paths() -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), String> {
    for _ in 0..4 {
        let (mut n_paths, mut n_modes) = (0u32, 0u32);
        // SAFETY: out-pointers to locals.
        let status = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes)
        };
        if status != 0 {
            return Err(format!("GetDisplayConfigBufferSizes failed ({status})"));
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
        // SAFETY: buffers sized by the counts passed in.
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut n_paths,
                paths.as_mut_ptr(),
                &mut n_modes,
                modes.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue; // topology changed between the two calls
        }
        if status != 0 {
            return Err(format!("QueryDisplayConfig failed ({status})"));
        }
        paths.truncate(n_paths as usize);
        modes.truncate(n_modes as usize);
        return Ok((paths, modes));
    }
    Err("Display configuration kept changing during enumeration".into())
}

fn header(
    kind: i32,
    size: usize,
    adapter: windows_sys::Win32::Foundation::LUID,
    id: u32,
) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: size as u32,
        adapterId: adapter,
        id,
    }
}

pub fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    with_physical_pixels(|| {
        let (paths, modes) = query_paths()?;
        let monitors = monitors();
        let mut displays = Vec::new();
        for path in &paths {
            let source = &path.sourceInfo;
            let target = &path.targetInfo;
            let mut source_name = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: header(
                    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(),
                    source.adapterId,
                    source.id,
                ),
                ..Default::default()
            };
            // SAFETY: header describes the enclosing struct and its size.
            if unsafe { DisplayConfigGetDeviceInfo(&mut source_name.header) } != 0 {
                continue;
            }
            let mut target_name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                header: header(
                    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                    std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(),
                    target.adapterId,
                    target.id,
                ),
                ..Default::default()
            };
            // SAFETY: as above.
            let target_ok = unsafe { DisplayConfigGetDeviceInfo(&mut target_name.header) } == 0;
            let mut color = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
                header: header(
                    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                    std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(),
                    target.adapterId,
                    target.id,
                ),
                ..Default::default()
            };
            // SAFETY: as above.
            let advanced_color = if unsafe { DisplayConfigGetDeviceInfo(&mut color.header) } == 0 {
                // SAFETY: the call succeeded, so the flags union is initialized;
                // bit 1 is advancedColorEnabled.
                Some(unsafe { color.Anonymous.value } & 0x2 != 0)
            } else {
                None
            };

            // SAFETY: without QDC_VIRTUAL_MODE_AWARE the union holds modeInfoIdx.
            let source_index = unsafe { source.Anonymous.modeInfoIdx } as usize;
            let target_index = unsafe { target.Anonymous.modeInfoIdx } as usize;
            let Some(source_mode) = modes
                .get(source_index)
                .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
                // SAFETY: infoType says the union holds sourceMode.
                .map(|m| unsafe { m.Anonymous.sourceMode })
            else {
                continue;
            };
            let signal_active = modes
                .get(target_index)
                .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET)
                // SAFETY: infoType says the union holds targetMode.
                .map(|m| {
                    let size = unsafe { m.Anonymous.targetMode }
                        .targetVideoSignalInfo
                        .activeSize;
                    (size.cx, size.cy)
                });
            let gdi_name = from_wide(&source_name.viewGdiDeviceName);
            let clone_count = paths
                .iter()
                .filter(|p| {
                    p.sourceInfo.id == source.id
                        && p.sourceInfo.adapterId.LowPart == source.adapterId.LowPart
                        && p.sourceInfo.adapterId.HighPart == source.adapterId.HighPart
                })
                .count() as u32;
            let mut devmode = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            let name = wide(&gdi_name);
            // SAFETY: NUL-terminated name and sized DEVMODEW.
            let have_mode =
                unsafe { EnumDisplaySettingsW(name.as_ptr(), ENUM_CURRENT_SETTINGS, &mut devmode) }
                    != 0;
            let refresh = target.refreshRate;
            let refresh_hz = (refresh.Denominator != 0)
                .then(|| f64::from(refresh.Numerator) / f64::from(refresh.Denominator))
                .or_else(|| have_mode.then(|| f64::from(devmode.dmDisplayFrequency)));
            let monitor = monitors.iter().find(|m| m.gdi_name == gdi_name);
            let desktop = Rect {
                x: source_mode.position.x,
                y: source_mode.position.y,
                width: source_mode.width,
                height: source_mode.height,
            };
            let technology = if target_ok {
                target_name.outputTechnology
            } else {
                target.outputTechnology
            };
            displays.push(DisplayInfo {
                friendly_name: if target_ok {
                    from_wide(&target_name.monitorFriendlyDeviceName)
                } else {
                    String::new()
                },
                monitor_device_path: if target_ok {
                    from_wide(&target_name.monitorDevicePath)
                } else {
                    String::new()
                },
                adapter: format!(
                    "{:08x}:{:08x}",
                    target.adapterId.HighPart, target.adapterId.LowPart
                ),
                target_id: target.id,
                connector: match technology {
                    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI => Connector::Hdmi,
                    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI => Connector::Dvi,
                    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EXTERNAL => Connector::DisplayPort,
                    // 0x80000000: DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
                    t if t == i32::MIN => Connector::Internal,
                    t => Connector::Other(t),
                },
                desktop,
                monitor_rect: monitor.map(|m| m.rect),
                signal_active,
                refresh_hz,
                bits_per_pixel: have_mode.then_some(devmode.dmBitsPerPel),
                primary: monitor.map_or(desktop.x == 0 && desktop.y == 0, |m| m.primary),
                clone_count,
                rotated: target.rotation != DISPLAYCONFIG_ROTATION_IDENTITY,
                advanced_color,
                simulated: false,
                gdi_name,
            });
        }
        Ok(displays)
    })
}

/// State shared between the worker thread and the window thread.
struct Shared {
    pixels: Mutex<Vec<u8>>,
    width: u32,
    height: u32,
    rect: Rect,
    gdi_name: String,
    invalidation: Mutex<Option<String>>,
    closing: AtomicBool,
    last_error: Mutex<Option<String>>,
}

thread_local! {
    static WINDOW_SHARED: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) };
}

fn invalidate(hwnd: HWND, shared: &Shared, reason: &str) {
    // SAFETY: hwnd is our own window on this thread.
    unsafe { ShowWindow(hwnd, SW_HIDE) };
    let mut slot = shared
        .invalidation
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if slot.is_none() {
        *slot = Some(reason.into());
    }
}

fn window_rect(hwnd: HWND) -> Rect {
    let mut r = RECT::default();
    // SAFETY: valid out-pointer.
    unsafe { GetWindowRect(hwnd, &mut r) };
    Rect {
        x: r.left,
        y: r.top,
        width: (r.right - r.left).max(0) as u32,
        height: (r.bottom - r.top).max(0) as u32,
    }
}

fn paint(hwnd: HWND, shared: &Shared) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: standard WM_PAINT sequence on our window.
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    if !hdc.is_null() {
        let pixels = shared.pixels.lock().unwrap_or_else(|p| p.into_inner());
        let info = bitmap_info(shared.width, shared.height);
        // SAFETY: pixels holds width*height*4 bytes in top-down BGRX order.
        unsafe {
            SetDIBitsToDevice(
                hdc,
                0,
                0,
                shared.width,
                shared.height,
                0,
                0,
                0,
                shared.height,
                pixels.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
            );
            GdiFlush();
        }
    }
    // SAFETY: paired with BeginPaint.
    unsafe { EndPaint(hwnd, &ps) };
}

fn bitmap_info(width: u32, height: u32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Geometry and readback checks after a draw (runs on the window thread).
fn verify(hwnd: HWND, shared: &Shared) -> Result<(), String> {
    // SAFETY: queries on our own window.
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return Err("Output window is not visible".into());
    }
    let rect = window_rect(hwnd);
    if rect != shared.rect {
        return Err(format!(
            "Output window is at {}×{}+{}+{}, expected {}×{}+{}+{}",
            rect.width,
            rect.height,
            rect.x,
            rect.y,
            shared.rect.width,
            shared.rect.height,
            shared.rect.x,
            shared.rect.y
        ));
    }
    let mut client = RECT::default();
    // SAFETY: valid out-pointer.
    unsafe { GetClientRect(hwnd, &mut client) };
    if (client.right - client.left, client.bottom - client.top)
        != (shared.width as i32, shared.height as i32)
    {
        return Err("Output client area is not the SLM raster size".into());
    }
    // SAFETY: our window; MONITOR_DEFAULTTONULL returns null if off-screen.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) };
    if monitor.is_null() {
        return Err("Output window is not on any monitor".into());
    }
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: as in collect_monitor.
    unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    if from_wide(&info.szDevice) != shared.gdi_name {
        return Err(format!(
            "Output window is on {}, not {}",
            from_wide(&info.szDevice),
            shared.gdi_name
        ));
    }
    readback(shared)
}

/// Read the composed desktop area back and compare RGB with the frame.
fn readback(shared: &Shared) -> Result<(), String> {
    // SAFETY: waits for the next composition pass; failure is tolerated.
    unsafe { DwmFlush() };
    let (w, h) = (shared.width as i32, shared.height as i32);
    // SAFETY: standard screen capture into a DIB section; every handle created
    // here is released below.
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err("GetDC(screen) failed".into());
        }
        let memory = CreateCompatibleDC(screen);
        let info = bitmap_info(shared.width, shared.height);
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            screen,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        let result = if memory.is_null() || bitmap.is_null() || bits.is_null() {
            Err("Could not allocate the readback buffer".to_string())
        } else {
            let old = SelectObject(memory, bitmap);
            let ok = BitBlt(
                memory,
                0,
                0,
                w,
                h,
                screen,
                shared.rect.x,
                shared.rect.y,
                SRCCOPY,
            ) != 0;
            GdiFlush();
            let result = if ok {
                let captured = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                let expected = shared.pixels.lock().unwrap_or_else(|p| p.into_inner());
                let mut first = None;
                let mut count = 0usize;
                for (i, (a, b)) in captured
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(expected.as_chunks::<4>().0.iter())
                    .enumerate()
                {
                    if a[..3] != b[..3] {
                        count += 1;
                        first.get_or_insert(i);
                    }
                }
                match first {
                    None => Ok(()),
                    Some(i) => Err(format!(
                        "Desktop readback differs at {count} pixels (first at x={}, y={}); another window, the cursor or color processing may be on the SLM display",
                        i % shared.width as usize,
                        i / shared.width as usize
                    )),
                }
            } else {
                Err("BitBlt readback failed".into())
            };
            SelectObject(memory, old);
            result
        };
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !memory.is_null() {
            DeleteDC(memory);
        }
        ReleaseDC(std::ptr::null_mut(), screen);
        result
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let shared = WINDOW_SHARED.with(|s| s.borrow().clone());
    let Some(shared) = shared else {
        // SAFETY: default processing.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd, &shared);
            0
        }
        WM_SETCURSOR => {
            // Hide the pointer over the SLM so it is never drawn into the pattern.
            // SAFETY: null cursor hides it while over this window.
            unsafe { SetCursor(std::ptr::null_mut()) };
            1
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_DISPLAYCHANGE => {
            invalidate(hwnd, &shared, "Windows display mode or layout changed.");
            0
        }
        WM_DPICHANGED => {
            invalidate(hwnd, &shared, "Display scaling (DPI) changed.");
            0
        }
        WM_SETTINGCHANGE => {
            // Work-area/taskbar changes arrive here too; only hide if moved.
            // SAFETY: query on our window.
            if unsafe { IsWindowVisible(hwnd) } != 0 && window_rect(hwnd) != shared.rect {
                invalidate(hwnd, &shared, "Windows settings changed the output window.");
            }
            // SAFETY: default processing.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_WINDOWPOSCHANGED => {
            // SAFETY: query on our window.
            if unsafe { IsWindowVisible(hwnd) } != 0 && window_rect(hwnd) != shared.rect {
                invalidate(
                    hwnd,
                    &shared,
                    "The output window was moved or resized by Windows.",
                );
            }
            0
        }
        WM_APP_SHOW => {
            if shared
                .invalidation
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_some()
            {
                return 0;
            }
            let r = shared.rect;
            // SAFETY: our window; places it exactly over the monitor.
            unsafe {
                if IsWindowVisible(hwnd) == 0 {
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        r.x,
                        r.y,
                        r.width as i32,
                        r.height as i32,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
                InvalidateRect(hwnd, std::ptr::null(), 0);
                UpdateWindow(hwnd);
            }
            let result = verify(hwnd, &shared);
            let ok = result.is_ok();
            *shared.last_error.lock().unwrap_or_else(|p| p.into_inner()) = result.err();
            ok as LRESULT
        }
        WM_CLOSE => {
            if shared.closing.load(Ordering::Acquire) {
                // SAFETY: our window.
                unsafe { DestroyWindow(hwnd) };
            }
            0
        }
        WM_APP_CLOSE => {
            shared.closing.store(true, Ordering::Release);
            // SAFETY: hide first so nothing lingers, then destroy.
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: default processing.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn class_name() -> &'static [u16] {
    static NAME: OnceLock<Vec<u16>> = OnceLock::new();
    NAME.get_or_init(|| wide("KohdaLabSlmOutput"))
}

fn register_class() -> Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                // SAFETY: handle of the current module.
                hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
                lpszClassName: class_name().as_ptr(),
                ..Default::default()
            };
            // SAFETY: class struct is fully initialized; name is static.
            if unsafe { RegisterClassExW(&class) } == 0 {
                Err("RegisterClassExW failed".into())
            } else {
                Ok(())
            }
        })
        .clone()
}

struct OutputWindow {
    hwnd: usize,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl OutputWindow {
    fn create(target: &OutputTarget) -> Result<Self, String> {
        register_class()?;
        let rect = target.display.desktop;
        let shared = Arc::new(Shared {
            pixels: Mutex::new(vec![0; rect.width as usize * rect.height as usize * 4]),
            width: rect.width,
            height: rect.height,
            rect,
            gdi_name: target.display.gdi_name.clone(),
            invalidation: Mutex::new(None),
            closing: AtomicBool::new(false),
            last_error: Mutex::new(None),
        });
        let (tx, rx) = channel::<Result<usize, String>>();
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("slm-output-window".into())
            .spawn(move || {
                // SAFETY: per-thread DPI context for everything this thread creates.
                if unsafe {
                    SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                }
                .is_null()
                {
                    let _ = tx.send(Err("Per-monitor DPI awareness (v2) is not available".into()));
                    return;
                }
                WINDOW_SHARED.with(|s| *s.borrow_mut() = Some(thread_shared.clone()));
                let title = wide("KohdaLab SLM output");
                let r = thread_shared.rect;
                // SAFETY: registered class; hidden popup (no WS_VISIBLE).
                let hwnd = unsafe {
                    CreateWindowExW(
                        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                        class_name().as_ptr(),
                        title.as_ptr(),
                        WS_POPUP,
                        r.x,
                        r.y,
                        r.width as i32,
                        r.height as i32,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        GetModuleHandleW(std::ptr::null()),
                        std::ptr::null(),
                    )
                };
                if hwnd.is_null() {
                    let _ = tx.send(Err("CreateWindowExW failed".into()));
                    return;
                }
                let _ = tx.send(Ok(hwnd as usize));
                let mut msg = MSG::default();
                // SAFETY: standard message loop for this thread's window.
                unsafe {
                    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                WINDOW_SHARED.with(|s| *s.borrow_mut() = None);
            })
            .map_err(|e| format!("Start output window thread: {e}"))?;
        match rx.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                shared,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err("Output window thread ended unexpectedly".into())
            }
        }
    }

    fn show(&self, bgrx: &[u8]) -> Result<(), String> {
        if let Some(reason) = self
            .shared
            .invalidation
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            return Err(reason);
        }
        {
            let mut pixels = self.shared.pixels.lock().unwrap_or_else(|p| p.into_inner());
            if pixels.len() != bgrx.len() {
                return Err("Encoded frame size does not match the output window".into());
            }
            pixels.copy_from_slice(bgrx);
        }
        // SAFETY: synchronous cross-thread message to our window; the window
        // thread paints and verifies before returning.
        let ok = unsafe { SendMessageW(self.hwnd as HWND, WM_APP_SHOW, 0, 0) } != 0;
        if ok {
            Ok(())
        } else {
            let error = self
                .shared
                .last_error
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let invalid = self
                .shared
                .invalidation
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            Err(invalid
                .or(error)
                .unwrap_or_else(|| "Output window did not confirm the draw".into()))
        }
    }

    fn take_invalidation(&self) -> Option<String> {
        self.shared
            .invalidation
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }
}

impl Drop for OutputWindow {
    fn drop(&mut self) {
        // SAFETY: synchronous; destroys the window on its own thread.
        unsafe { SendMessageW(self.hwnd as HWND, WM_APP_CLOSE, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct WindowsBackend {
    window: Option<OutputWindow>,
}

impl WindowsBackend {
    pub fn new() -> Self {
        Self { window: None }
    }
}

impl OutputBackend for WindowsBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::WindowsDisplay
    }
    fn list_displays(&mut self) -> Result<Vec<DisplayInfo>, String> {
        list_displays()
    }
    fn remote_session(&self) -> bool {
        // SAFETY: simple metric query.
        unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
    }
    fn open(&mut self, target: &OutputTarget) -> Result<(), String> {
        if self.window.is_some() {
            return Err("Output window already open".into());
        }
        self.window = Some(OutputWindow::create(target)?);
        Ok(())
    }
    fn show(&mut self, bgrx: &[u8]) -> Result<Verification, String> {
        let window = self.window.as_ref().ok_or("Output window is not open")?;
        window.show(bgrx).map(|()| Verification::DesktopReadback)
    }
    fn close(&mut self) -> Result<(), String> {
        self.window = None; // Drop hides and destroys the window.
        Ok(())
    }
    fn take_invalidation(&mut self) -> Option<String> {
        self.window.as_ref()?.take_invalidation()
    }
}
