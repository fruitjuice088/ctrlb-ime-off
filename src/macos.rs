use std::env;
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2_app_kit::NSWorkspace;
use objc2_core_foundation::{CFRetained, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventMask, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventTapProxy, CGEventType, CGKeyCode,
};

/// wezterm-gui.app の bundle identifier。この前面時のみ Ctrl-B に介入する。
const TARGET_BUNDLE_ID: &str = "com.github.wez.wezterm";

/// 英数(JIS_Eisu)キーの仮想キーコード。物理キーの有無に関わらず、この
/// キーコードの送出で IME が英数(OFF相当)に切り替わる。
const VK_JIS_EISU: CGKeyCode = 0x66;
const VK_ANSI_B: CGKeyCode = 0x0B;

/// 自分が送出したイベントに付与し、フックの再入(無限ループ)を防ぐ識別値。
const SENDER_TAG: i64 = 0x4354_5242_4942;

/// 直前に本物の Ctrl-B キーダウンを抑制したかどうか(対応する KeyUp も抑制するため)。
static SUPPRESS_PENDING_KEYUP: AtomicBool = AtomicBool::new(false);

fn log_error(context: &str, detail: &str) {
    // 異常系のみ記録する。詳細を追わないと再現しない事故のためのログ。
    if let Ok(mut exe_path) = env::current_exe() {
        exe_path.set_file_name("ctrlb-ime-off.log");
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(exe_path) {
            let _ = writeln!(file, "[{context}] {detail}");
        }
    }
}

fn is_foreground_target() -> bool {
    let workspace = NSWorkspace::sharedWorkspace();
    let Some(app) = workspace.frontmostApplication() else {
        return false;
    };
    let Some(bundle_id) = app.bundleIdentifier() else {
        return false;
    };
    bundle_id.to_string() == TARGET_BUNDLE_ID
}

fn set_event_sender_tag(event: NonNull<CGEvent>) {
    unsafe {
        CGEvent::set_integer_value_field(
            Some(event.as_ref()),
            CGEventField::EventSourceUserData,
            SENDER_TAG,
        );
    }
}

fn is_own_event(event: NonNull<CGEvent>) -> bool {
    (unsafe {
        CGEvent::integer_value_field(Some(event.as_ref()), CGEventField::EventSourceUserData)
    }) == SENDER_TAG
}

/// WindowServer の入力ソース切り替え処理は、合成された新規イベントには反応
/// せず、実ハードウェア由来のイベントの複製にのみ反応する。そのため受信した
/// イベントを複製し、キーコードだけ英数キーへ書き換えて送出する。
fn send_ime_off(source_event: NonNull<CGEvent>) {
    for etype in [CGEventType::KeyDown, CGEventType::KeyUp] {
        let Some(ev) = CGEvent::new_copy(Some(unsafe { source_event.as_ref() })) else {
            log_error("send_ime_off", "new_copy failed");
            return;
        };
        CGEvent::set_integer_value_field(
            Some(&ev),
            CGEventField::KeyboardEventKeycode,
            VK_JIS_EISU as i64,
        );
        CGEvent::set_type(Some(&ev), etype);
        CGEvent::set_flags(Some(&ev), CGEventFlags::empty());
        set_event_sender_tag(NonNull::from(&*ev));
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&ev));
    }
}

fn send_synthetic_b(key_down: bool) {
    let Some(ev) = CGEvent::new_keyboard_event(None, VK_ANSI_B, key_down) else {
        log_error("send_synthetic_b", "new_keyboard_event failed");
        return;
    };
    CGEvent::set_flags(Some(&ev), CGEventFlags::MaskControl);
    set_event_sender_tag(NonNull::from(&*ev));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&ev));
}

unsafe extern "C-unwind" fn hook_proc(
    _proxy: CGEventTapProxy,
    etype: CGEventType,
    event: NonNull<CGEvent>,
    _user_info: *mut c_void,
) -> *mut CGEvent {
    if is_own_event(event) {
        // 自分が送出したイベントはそのまま通す(再入防止)
        return event.as_ptr();
    }

    let keycode =
        CGEvent::integer_value_field(Some(event.as_ref()), CGEventField::KeyboardEventKeycode)
            as CGKeyCode;

    if keycode == VK_ANSI_B {
        match etype {
            CGEventType::KeyDown => {
                let flags = CGEvent::flags(Some(event.as_ref()));
                let ctrl_down = flags.contains(CGEventFlags::MaskControl);
                if ctrl_down && is_foreground_target() {
                    send_ime_off(event);
                    send_synthetic_b(true);
                    send_synthetic_b(false);
                    SUPPRESS_PENDING_KEYUP.store(true, Ordering::SeqCst);
                    return std::ptr::null_mut();
                }
            }
            CGEventType::KeyUp => {
                if SUPPRESS_PENDING_KEYUP.swap(false, Ordering::SeqCst) {
                    return std::ptr::null_mut();
                }
            }
            _ => {}
        }
    }

    event.as_ptr()
}

fn acquire_single_instance_lock() -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;

    let lock_path = std::env::temp_dir().join("ctrlb-ime-off.lock");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(lock_path)
        .ok()?;
    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret != 0 {
        return None;
    }
    Some(file)
}

pub fn run() {
    // 多重起動防止: 二重フックによるキー二重送出事故を避ける
    let Some(_lock) = acquire_single_instance_lock() else {
        std::process::exit(0);
    };

    let mask: CGEventMask =
        (1u64 << CGEventType::KeyDown.0 as u64) | (1u64 << CGEventType::KeyUp.0 as u64);

    let tap: Option<CFRetained<_>> = unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::HIDEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            mask,
            Some(hook_proc),
            std::ptr::null_mut(),
        )
    };

    let Some(tap) = tap else {
        log_error(
            "CGEvent::tap_create",
            "failed (Input Monitoring 権限が未許可の可能性)",
        );
        std::process::exit(1);
    };

    unsafe {
        let run_loop_source = objc2_core_foundation::CFMachPort::new_run_loop_source(
            None,
            Some(&tap),
            0,
        );
        let Some(run_loop_source) = run_loop_source else {
            log_error("CFMachPort::new_run_loop_source", "failed");
            std::process::exit(1);
        };
        let current = CFRunLoop::current().expect("current run loop");
        current.add_source(Some(&run_loop_source), kCFRunLoopCommonModes);
        CGEvent::tap_enable(&tap, true);
        CFRunLoop::run();
    }
}
