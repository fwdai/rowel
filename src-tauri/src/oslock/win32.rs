//! Windows: session-change and power broadcasts to a hidden window of our own,
//! since Tauri exposes no hook into the main window's messages.

use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, MSG,
    PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND, WINDOW_EX_STYLE, WINDOW_STYLE, WM_POWERBROADCAST,
    WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_DISCONNECT,
    WTS_SESSION_LOCK,
};

use super::{Event, Sink};

const CLASS: PCWSTR = w!("RowelOsLock");

static SINK: OnceLock<Sink> = OnceLock::new();

pub(super) fn watch(sink: Sink) {
    if SINK.set(sink).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("os-lock".into())
        .spawn(run);
    if let Err(error) = spawned {
        log::warn!("the OS lock listener did not start: {error}");
    }
}

fn run() {
    let Some(sink) = SINK.get() else {
        return;
    };
    let hwnd = match window() {
        Ok(hwnd) => hwnd,
        Err(error) => {
            log::warn!("the OS lock listener has no window: {error}");
            return;
        }
    };
    // SAFETY: a window this thread owns, for as long as the process runs.
    match unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } {
        Ok(()) => sink(Event::Watching),
        Err(error) => log::warn!("the session locking is not heard: {error}"),
    }
    let mut message = MSG::default();
    // -1 is an error and 0 is `WM_QUIT`; either ends the loop.
    // SAFETY: the message loop of the thread that owns the window.
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        unsafe { DispatchMessageW(&message) };
    }
    sink(Event::Lost);
}

/// Never shown. Top-level, since message-only windows miss power broadcasts.
fn window() -> windows::core::Result<HWND> {
    // SAFETY: plain Win32 calls with a class whose procedure is below.
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Rowel"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let (Some(event), Some(sink)) = (event(message, wparam), SINK.get()) {
        sink(event);
    }
    // SAFETY: the arguments the system called this procedure with.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn event(message: u32, wparam: WPARAM) -> Option<Event> {
    let code = u32::try_from(wparam.0).ok()?;
    match (message, code) {
        (WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK) => Some(Event::Locked),
        (WM_WTSSESSION_CHANGE, WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT) => {
            Some(Event::SwitchedAway)
        }
        (WM_POWERBROADCAST, PBT_APMSUSPEND) => Some(Event::Sleep),
        (WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC) => Some(Event::Wake),
        _ => None,
    }
}
