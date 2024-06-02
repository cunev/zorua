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
    GetDC, GetSystemMetrics, GetWindowLongPtrW,
    ReleaseDC, SetCursorPos, SetWindowLongPtrW, GWLP_USERDATA, MONITORINFO,
    MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MOUSE_MOVE_ABSOLUTE, SM_CXSCREEN, SM_CYSCREEN,
    WH_MOUSE_LL, MSLLHOOKSTRUCT
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
            GetMessageW, RegisterClassExW,
            RegisterWindowMessageW, TranslateMessage, LPMSG, MOUSE_MOVE_RELATIVE, MSG,
            RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
            WM_INPUT, WM_QUERYENDSESSION, WNDCLASSEXW,
        },
    },
};

use widestring::U16CString;

const MSGFLT_ALLOW: DWORD = 1;

//Thread safe thingy
struct SafeHWND(HWND);
unsafe impl Send for SafeHWND {}

lazy_static! {
    static ref WM_TASKBAR_CREATED: UINT =
        unsafe { RegisterWindowMessageW(U16CString::from_str("TaskbarCreated").unwrap().as_ptr()) };
    static ref CLASS_NAME: U16CString = U16CString::from_str("W10Wheel/R_WM").unwrap();
    static ref STOP_FLAG: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    static ref X: Arc<AtomicI32> = Arc::new(AtomicI32::new(100));
    static ref Y: Arc<AtomicI32> = Arc::new(AtomicI32::new(100));
    static ref GLOBAL_HWND: Mutex<SafeHWND> = Mutex::new(SafeHWND(ptr::null_mut()));
}

static mut VIRTUAL_SCREEN_RECT: RECT = RECT {
    left: 0,
    top: 0,
    right: 0,
    bottom: 0,
};

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

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: UINT,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, w_param, l_param)
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

fn set_callback(mut cx: FunctionContext) -> JsResult<JsUndefined> {
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
                
                {
                    let mut global_hwnd = GLOBAL_HWND.lock().unwrap();
                    global_hwnd.0 = hwnd;
                }

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

    unsafe {
        SetCursorPos(a1, a2);
    }

    Ok(cx.undefined())
}

unsafe extern "system" fn raw_callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam as DWORD == WM_MOUSEMOVE {
        let hook_struct = *(lparam as *const MSLLHOOKSTRUCT);

        let global_hwnd = GLOBAL_HWND.lock().unwrap();
        let hwnd = global_hwnd.0;
        let mut callback_data =
        Box::from_raw(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CallbackData);
        (callback_data.callback)(hook_struct.pt.x, hook_struct.pt.y, 'a');
        Box::into_raw(callback_data);
        return 1;
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

fn start_input_interception(mut cx: FunctionContext) -> JsResult<JsUndefined> {
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
    cx.export_function("set_callback", set_callback)?;
    cx.export_function("start_input_interception", start_input_interception)?;
    cx.export_function("set_mouse_position", set_mouse_position)?;
    Ok(())
}
