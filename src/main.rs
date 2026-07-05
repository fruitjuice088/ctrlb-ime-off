#![windows_subsystem = "windows"]

use std::env;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::Ime::{ImmGetDefaultIMEWnd, IMC_SETOPENSTATUS};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_B,
    VK_CONTROL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    SendMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG,
    WH_KEYBOARD_LL, WM_IME_CONTROL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// 対象アプリの実行ファイル名(大文字小文字を無視して比較)
const TARGET_EXE: &str = "wezterm-gui.exe";

/// 自分がSendInputで送出したキーイベントに付与し、フックの再入(無限ループ)を防ぐ識別値
const SENDER_TAG: usize = 0x4354_5242_4942;

/// 直前に本物のCtrl-Bキーダウンを抑制したかどうか(対応するキーアップも抑制するため)
static SUPPRESS_PENDING_KEYUP: AtomicBool = AtomicBool::new(false);

fn log_error(context: &str, code: u32) {
    // 異常系のみ記録する。詳細を追わないと再現しない事故のためのログ。
    if let Ok(mut exe_path) = env::current_exe() {
        exe_path.set_file_name("ctrlb-ime-off.log");
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(exe_path) {
            let _ = writeln!(file, "[{context}] error_code={code}");
        }
    }
}

fn is_foreground_target() -> bool {
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return false;
        }

        let process: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }

        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(process);

        if ok == 0 {
            return false;
        }

        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/'])
            .next()
            .map(|name| name.eq_ignore_ascii_case(TARGET_EXE))
            .unwrap_or(false)
    }
}

fn set_ime_open(hwnd: HWND, open: bool) {
    unsafe {
        // ImmSetOpenStatus はプロセスをまたぐと反映されないことがあるため、
        // 対象ウィンドウのIMEウィンドウへ WM_IME_CONTROL を直接送る
        // (keyhac/pyautoのsetImeStatusと同じ経路。動作実績あり)。
        let hwnd_ime = ImmGetDefaultIMEWnd(hwnd);
        if hwnd_ime.is_null() {
            return;
        }
        SendMessageW(
            hwnd_ime,
            WM_IME_CONTROL,
            IMC_SETOPENSTATUS as usize,
            open as isize,
        );
    }
}

fn send_synthetic_b(key_up: bool) {
    unsafe {
        let mut input: INPUT = zeroed();
        input.r#type = INPUT_KEYBOARD;
        input.Anonymous.ki = KEYBDINPUT {
            wVk: VK_B,
            wScan: 0,
            dwFlags: if key_up { KEYEVENTF_KEYUP } else { 0 },
            time: 0,
            dwExtraInfo: SENDER_TAG,
        };
        let sent = SendInput(1, &input, size_of::<INPUT>() as i32);
        if sent == 0 {
            log_error("SendInput", GetLastError());
        }
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code >= 0 {
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);

        if kb.dwExtraInfo == SENDER_TAG {
            // 自分が送出したイベントはそのまま通す(再入防止)
            return CallNextHookEx(null_mut(), code, wparam, lparam);
        }

        if kb.vkCode == VK_B as u32 {
            let msg = wparam as u32;
            let is_keydown = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let is_keyup = msg == WM_KEYUP || msg == WM_SYSKEYUP;

            if is_keydown {
                let ctrl_down = (GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
                if ctrl_down && is_foreground_target() {
                    let hwnd = GetForegroundWindow();
                    set_ime_open(hwnd, false);
                    send_synthetic_b(false);
                    send_synthetic_b(true);
                    SUPPRESS_PENDING_KEYUP.store(true, Ordering::SeqCst);
                    return 1;
                }
            } else if is_keyup && SUPPRESS_PENDING_KEYUP.swap(false, Ordering::SeqCst) {
                return 1;
            }
        }
    }

    CallNextHookEx(null_mut(), code, wparam, lparam)
}

fn main() {
    unsafe {
        // 多重起動防止: 二重フックによるキー二重送出事故を避ける
        let mutex_name: Vec<u16> = "Local\\ctrlb_ime_off_mutex\0".encode_utf16().collect();
        let mutex = CreateMutexW(null_mut(), 0, mutex_name.as_ptr());
        if mutex.is_null() {
            log_error("CreateMutexW", GetLastError());
            std::process::exit(1);
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // 既に起動済み: 何もせず終了する
            std::process::exit(0);
        }

        let module = GetModuleHandleW(null_mut());
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0);
        if hook.is_null() {
            log_error("SetWindowsHookExW", GetLastError());
            std::process::exit(1);
        }

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        UnhookWindowsHookEx(hook);
    }
}
