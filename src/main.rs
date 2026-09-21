#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

fn main() {
    #[cfg(windows)]
    windows::run();

    #[cfg(target_os = "macos")]
    macos::run();
}
