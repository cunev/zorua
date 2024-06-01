use lazy_static::lazy_static;
use neon::prelude::*;
use ntapi::ntexapi::{NtQueryTimerResolution, NtSetTimerResolution};
use ntapi::ntpsapi::{NtSetInformationProcess, ThreadPowerThrottlingState};
use std::alloc::{alloc, dealloc, Layout};
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{mem, thread};
use std::{ptr, time};
use std::{
    ptr::null_mut,
    sync::{Arc, Mutex},
};
use winapi::shared::minwindef::{BOOL, ULONG};
use winapi::shared::ntdef::TRUE;
use winapi::shared::windef::{HMONITOR, RECT};
use winapi::um::processthreadsapi::{GetCurrentProcess, SetPriorityClass, SetProcessInformation};
use winapi::um::winbase::HIGH_PRIORITY_CLASS;
use winapi::um::wingdi::{GetDeviceCaps, LOGPIXELSX, LOGPIXELSY};
use winapi::{
    shared::windef::{HHOOK, POINT},
    um::{
        libloaderapi::GetModuleHandleA,
        winuser::{DispatchMessageA, GetMessageA, SetWindowsHookExA, WM_MOUSEMOVE},
    },
};
struct CallbackData {
    callback: Box<dyn FnMut(i32, i32, char)>,
}

use winapi::um::winuser::{
    EnumDisplayMonitors, GetDC, GetMonitorInfoW, GetSystemMetrics, GetWindowLongPtrW,
    MonitorFromWindow, ReleaseDC, SetCursorPos, SetWindowLongPtrW, GWLP_USERDATA, MONITORINFO,
    MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MOUSE_MOVE_ABSOLUTE, SM_CXSCREEN, SM_CYSCREEN,
    WH_MOUSE_LL,
};
use winapi::{
    shared::basetsd::LONG_PTR,
    um::winuser::{CallNextHookEx, UnhookWindowsHookEx},
};
use winapi::{
    shared::{
        hidusage::{HID_USAGE_GENERIC_MOUSE, HID_USAGE_PAGE_GENERIC},
        minwindef::{DWORD, HINSTANCE, LPARAM, LPVOID, LRESULT, PUINT, UINT, WPARAM},
        windef::HWND,
    },
    um::{
        libloaderapi::GetModuleHandleW,
        winuser::{
            ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DispatchMessageW,
            GetMessageW, GetRawInputData, RegisterClassExW, RegisterRawInputDevices,
            RegisterWindowMessageW, TranslateMessage, HRAWINPUT, LPMSG, MOUSE_MOVE_RELATIVE, MSG,
            RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
            WM_INPUT, WM_QUERYENDSESSION, WNDCLASSEXW,
        },
    },
};

use widestring::U16CString;

const MSGFLT_ALLOW: DWORD = 1;

lazy_static! {
    static ref WM_TASKBAR_CREATED: UINT =
        unsafe { RegisterWindowMessageW(U16CString::from_str("TaskbarCreated").unwrap().as_ptr()) };
    static ref CB_SIZE_HEADER: UINT = mem::size_of::<RAWINPUTHEADER>() as UINT;
    static ref CLASS_NAME: U16CString = U16CString::from_str("W10Wheel/R_WM").unwrap();
    static ref STOP_FLAG: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    static ref X: Arc<AtomicI32> = Arc::new(AtomicI32::new(100));
    static ref Y: Arc<AtomicI32> = Arc::new(AtomicI32::new(100));
}

unsafe fn proc_raw_input(l_param: LPARAM, callback_data: &mut CallbackData) -> bool {
    let mut pcb_size = 0;

    let is_mouse_move_relative = |ri: RAWINPUT| {
        ri.header.dwType == RIM_TYPEMOUSE && ri.data.mouse().usFlags == MOUSE_MOVE_RELATIVE
    };

    let is_mouse_move_absolute = |ri: RAWINPUT| {
        ri.header.dwType == RIM_TYPEMOUSE && (ri.data.mouse().usFlags & MOUSE_MOVE_ABSOLUTE) != 0
    };

    let get_raw_input_data = |data: LPVOID, size: PUINT| {
        GetRawInputData(l_param as HRAWINPUT, RID_INPUT, data, size, *CB_SIZE_HEADER)
    };

    if get_raw_input_data(ptr::null_mut(), &mut pcb_size) == 0 {
        let layout = Layout::from_size_align(pcb_size as usize, 1).unwrap();
        let data = alloc(layout);
        let mut res = false;

        if get_raw_input_data(data as LPVOID, &mut pcb_size) == pcb_size {
            let ri = std::ptr::read(data as *const RAWINPUT);
            if is_mouse_move_relative(ri) {
                let mouse = ri.data.mouse();
                (callback_data.callback)(mouse.lLastX, mouse.lLastY, 'r');
                res = true;
            }
            if is_mouse_move_absolute(ri) {
                let mouse = ri.data.mouse();
                let mut virtual_screen_rect = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };

                // Get screen resolution
                get_virtual_screen_rect(&mut virtual_screen_rect);

                // Transform to pixels
                let x_pixel = (mouse.lLastX as f64 / 65535.0 * virtual_screen_rect.right as f64)
                    .round() as i32;
                let y_pixel = (mouse.lLastY as f64 / 65535.0 * virtual_screen_rect.bottom as f64)
                    .round() as i32;

                (callback_data.callback)(x_pixel, y_pixel, 'a');
                res = true;
            }
        }

        dealloc(data, layout);
        return res;
    }

    false
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: UINT,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    set_cursor_position(X.load(Ordering::SeqCst), Y.load(Ordering::SeqCst));
    match msg {
        WM_INPUT => {
            let mut callback_data =
                Box::from_raw(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CallbackData);
            if proc_raw_input(l_param, &mut callback_data) {
                Box::into_raw(callback_data);
                return 0;
            }
            Box::into_raw(callback_data);
        }
        WM_QUERYENDSESSION => {
            return 0;
        }
        _ => {
            if msg == *WM_TASKBAR_CREATED {
                return 0;
            }
        }
    };

    DefWindowProcW(hwnd, msg, w_param, l_param)
}

unsafe fn message_loop(msg: LPMSG) {
    loop {
        if GetMessageW(msg, ptr::null_mut(), 0, 0) == 0 {
            return;
        }

        TranslateMessage(msg);
        DispatchMessageW(msg);
        thread::sleep(time::Duration::from_millis(1))
    }
}

fn make_window_class(h_instance: HINSTANCE) -> WNDCLASSEXW {
    WNDCLASSEXW {
        cbSize: (mem::size_of::<WNDCLASSEXW>()) as UINT,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hbrBackground: ptr::null_mut(),
        hCursor: ptr::null_mut(),
        hIcon: ptr::null_mut(),
        hIconSm: ptr::null_mut(),
        hInstance: h_instance,
        lpfnWndProc: Some(window_proc),
        lpszClassName: CLASS_NAME.as_ptr(),
        lpszMenuName: ptr::null_mut(),
        style: 0,
    }
}

fn make_raw_input_device(hwnd: HWND) -> RAWINPUTDEVICE {
    RAWINPUTDEVICE {
        usUsagePage: HID_USAGE_PAGE_GENERIC,
        usUsage: HID_USAGE_GENERIC_MOUSE,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    }
}

fn start_raw_input(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let func = cx.argument::<JsFunction>(0)?.root(&mut cx);
    let channel = cx.channel();
    let func = Arc::new(Mutex::new(func));

    std::thread::spawn({
        move || unsafe {
            let h_instance = GetModuleHandleW(ptr::null());
            let window_class = make_window_class(h_instance);

            if RegisterClassExW(&window_class) != 0 {
                let hwnd = CreateWindowExW(
                    0,
                    CLASS_NAME.as_ptr(),
                    ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                let data = Box::new(CallbackData {
                    callback: Box::new(move |x, y, mode| {
                        let func = Arc::clone(&func);
                        channel.send(move |mut cx| {
                            let func = func.lock().unwrap();
                            let this = cx.undefined();
                            let jsx = cx.number(x);
                            let jsy = cx.number(y);
                            let mode = cx.string(mode.to_string());

                            let callback = func.to_inner(&mut cx);
                            let _ = callback.call(
                                &mut cx,
                                this,
                                &[jsx.upcast(), jsy.upcast(), mode.upcast()],
                            );
                            Ok(())
                        });
                    }),
                });

                SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as LONG_PTR);

                ChangeWindowMessageFilterEx(
                    hwnd,
                    *WM_TASKBAR_CREATED,
                    MSGFLT_ALLOW,
                    ptr::null_mut(),
                );

                let rid = make_raw_input_device(hwnd);
                let mut rid_array = vec![rid];
                RegisterRawInputDevices(
                    rid_array.as_mut_ptr(),
                    1,
                    mem::size_of::<RAWINPUTDEVICE>() as UINT,
                );

                let layout = Layout::new::<MSG>();
                let msg = alloc(layout);
                message_loop(msg as LPMSG);
                dealloc(msg, layout);
            }
        }
    });

    Ok(cx.undefined())
}

fn set_mouse_position(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let a1 = cx.argument::<JsNumber>(0)?.value(&mut cx) as i32;
    let a2 = cx.argument::<JsNumber>(1)?.value(&mut cx) as i32;

    X.store(a1, Ordering::SeqCst);
    Y.store(a2, Ordering::SeqCst);

    Ok(cx.undefined())
}

unsafe fn set_cursor_position(screen_x: i32, screen_y: i32) {
    SetCursorPos(screen_x, screen_y);
}

unsafe extern "system" fn monitor_enum_proc(
    hmonitor: HMONITOR,
    hdc: winapi::shared::windef::HDC,
    rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let mut monitor_info: MONITORINFO = std::mem::zeroed();
    monitor_info.cbSize = std::mem::size_of::<MONITORINFO>() as UINT;

    if GetMonitorInfoW(hmonitor, &mut monitor_info) != 0 {
        let rect_ptr = lparam as *mut RECT;
        let monitor_rect = monitor_info.rcMonitor;

        (*rect_ptr).left = (*rect_ptr).left.min(monitor_rect.left);
        (*rect_ptr).top = (*rect_ptr).top.min(monitor_rect.top);
        (*rect_ptr).right = (*rect_ptr).right.max(monitor_rect.right);
        (*rect_ptr).bottom = (*rect_ptr).bottom.max(monitor_rect.bottom);
    }

    1 // Continue enumeration
}

unsafe fn get_virtual_screen_rect(rect: &mut RECT) {
    let mut monitor_info: MONITORINFO = std::mem::zeroed();
    monitor_info.cbSize = std::mem::size_of::<MONITORINFO>() as UINT;

    // Initialize the rect to cover at least one monitor
    if GetMonitorInfoW(
        MonitorFromWindow(null_mut(), MONITOR_DEFAULTTOPRIMARY),
        &mut monitor_info,
    ) != 0
    {
        *rect = monitor_info.rcMonitor;
    }
    // Enumerate all monitors and accumulate their dimensions
    EnumDisplayMonitors(
        null_mut(),
        null_mut(),
        Some(monitor_enum_proc),
        rect as *mut RECT as LPARAM,
    );
}
unsafe extern "system" fn raw_callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam as DWORD == WM_MOUSEMOVE {
        return 1;
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

fn block_input(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    thread::spawn(move || unsafe {
        let hook_id: HHOOK = SetWindowsHookExA(
            WH_MOUSE_LL,
            Some(raw_callback),
            GetModuleHandleA(null_mut()),
            0,
        );

        let mut msg: MSG = MSG {
            hwnd: null_mut(),
            message: 0,
            wParam: 0,
            lParam: 0,
            time: 0,
            pt: POINT { x: 0, y: 0 },
        };

        while GetMessageA(&mut msg, null_mut(), 0, 0) != 0 {
            TranslateMessage(&msg);
            DispatchMessageA(&msg);
        }

        UnhookWindowsHookEx(hook_id);
    });
    Ok(cx.undefined())
}

const PROCESS_POWER_THROTTLING_CURRENT_VERSION: ULONG = 1;
const PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION: ULONG = 0x00000001;
const PROCESS_INFORMATION_CLASS_PROCESS_POWER_THROTTLING: ULONG = 0x29; // Value depends on actual definition in Windows headers
#[repr(C)]
struct ProcessPowerThrottlingState {
    version: ULONG,
    control_mask: ULONG,
    state_mask: ULONG,
}

fn disable_throttling(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    unsafe {
        SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS);

        let mut state = ProcessPowerThrottlingState {
            version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            control_mask: PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
            state_mask: 0,
        };

        SetProcessInformation(
            GetCurrentProcess(),
            PROCESS_INFORMATION_CLASS_PROCESS_POWER_THROTTLING,
            &mut state as *mut _ as _,
            std::mem::size_of::<ProcessPowerThrottlingState>() as DWORD,
        );

        let mut current_resolution = 0;
        let mut maximum_resolution = 0;
        let mut minimum_resolution = 0;

        NtQueryTimerResolution(
            &mut minimum_resolution,
            &mut maximum_resolution,
            &mut current_resolution,
        );
        NtSetTimerResolution(maximum_resolution, TRUE, &mut current_resolution);
        Ok(cx.undefined())
    }
}

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    cx.export_function("disable_throttling", disable_throttling)?;
    cx.export_function("start_raw_input", start_raw_input)?;
    cx.export_function("block_input", block_input)?;
    cx.export_function("set_mouse_position", set_mouse_position)?;
    Ok(())
}
