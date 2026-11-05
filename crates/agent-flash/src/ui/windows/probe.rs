//! Idle time, the focused application and the reduced-motion preference.

use std::path::Path;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
};
use windows_sys::core::BOOL;

use crate::ui::Probe;

/// Every call here is safe from any thread.
pub struct SystemProbe;

impl Probe for SystemProbe {
    fn idle_ms(&self) -> Option<u64> {
        let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        // SAFETY: `info` carries its size, as the call requires.
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return None;
        }
        // Both are 32-bit millisecond ticks, so the difference survives wrap-around.
        // SAFETY: GetTickCount has no preconditions.
        Some(u64::from(unsafe { GetTickCount() }.wrapping_sub(info.dwTime)))
    }

    fn focused_app(&self) -> Option<String> {
        // SAFETY: each handle is checked before use and the process handle is closed;
        // the name buffer's length is passed alongside it.
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() {
                return None;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            if pid == 0 {
                return None;
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(process);
            if ok == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            Path::new(&path).file_stem().map(|stem| stem.to_string_lossy().into_owned())
        }
    }
}

/// Whether "Animation effects" is turned off in Windows' accessibility settings.
pub fn reduce_motion() -> bool {
    let mut animate: BOOL = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through the pointer.
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animate).cast(), 0) };
    ok != 0 && animate == 0
}
