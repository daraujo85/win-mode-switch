#![windows_subsystem = "windows"]

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

mod hud;
mod replay;

const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
const ID_TRAY_ICON: u32 = 1001;

const ID_TOGGLE: usize = 2001;
const ID_GAMER: usize = 2002;
const ID_WORK: usize = 2003;
const ID_EXIT: usize = 2004;

// Embed high-res ICO files directly inside the binary
const COFFEE_ICO: &[u8] = include_bytes!("../assets/icons/coffee.ico");
const GAMER_ICO: &[u8] = include_bytes!("../assets/icons/gamer.ico");

const DOCKER_EXE: &str = r"C:\Program Files\Docker\Docker\Docker Desktop.exe";

fn get_scripts_dir() -> std::path::PathBuf {
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        std::path::PathBuf::from(userprofile).join("scripts")
    } else {
        std::path::PathBuf::from(r"C:\scripts")
    }
}

fn get_state_file() -> std::path::PathBuf {
    get_scripts_dir().join("current_mode.txt")
}

fn get_ollama_exe() -> std::path::PathBuf {
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        std::path::PathBuf::from(localappdata).join(r"Programs\Ollama\ollama app.exe")
    } else {
        std::path::PathBuf::from(r"C:\Users\Default\AppData\Local\Programs\Ollama\ollama app.exe")
    }
}

#[link(name = "winmm")]
extern "system" {
    fn PlaySoundW(pszSound: *const u16, hmod: *mut std::ffi::c_void, fdwSound: u32) -> i32;
}

const SND_ASYNC: u32 = 0x0001;
const SND_FILENAME: u32 = 0x00020000;
const SND_NODEFAULT: u32 = 0x0002;

fn play_sound_for_mode(mode: u8) {
    let sound_path = if mode == MODE_GAMER {
        get_scripts_dir().join("hadouken.wav")
    } else {
        get_scripts_dir().join("theoffice.wav")
    };

    if sound_path.exists() {
        if let Some(path_str) = sound_path.to_str() {
            let wide = to_wide(path_str);
            unsafe {
                PlaySoundW(wide.as_ptr(), null_mut(), SND_ASYNC | SND_FILENAME | SND_NODEFAULT);
            }
        }
    }
}

// Hide console window when spawning helper processes
const CREATE_NO_WINDOW: u32 = 0x08000000;

const MODE_WORK: u8 = 0;
const MODE_GAMER: u8 = 1;

static CURRENT_MODE: AtomicU8 = AtomicU8::new(MODE_WORK);
static IS_SWITCHING: AtomicBool = AtomicBool::new(false);

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn to_wide_buf<const N: usize>(s: &str) -> [u16; N] {
    let mut buf = [0u16; N];
    let mut i = 0;
    for c in s.encode_utf16() {
        if i >= N - 1 {
            break;
        }
        buf[i] = c;
        i += 1;
    }
    buf
}

unsafe fn load_icon_from_memory(bytes: &[u8], target_size: i32) -> HICON {
    if bytes.len() < 22 {
        return null_mut();
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    let mut best_offset = 0;
    let mut best_len = 0;

    for i in 0..count {
        let entry_offset = 6 + i * 16;
        if entry_offset + 16 > bytes.len() {
            break;
        }
        let w = bytes[entry_offset] as i32;
        let bytes_in_res = u32::from_le_bytes([
            bytes[entry_offset + 8],
            bytes[entry_offset + 9],
            bytes[entry_offset + 10],
            bytes[entry_offset + 11],
        ]) as usize;
        let image_offset = u32::from_le_bytes([
            bytes[entry_offset + 12],
            bytes[entry_offset + 13],
            bytes[entry_offset + 14],
            bytes[entry_offset + 15],
        ]) as usize;

        if w == target_size || best_offset == 0 {
            best_offset = image_offset;
            best_len = bytes_in_res;
            if w == target_size {
                break;
            }
        }
    }

    if best_offset == 0 || best_offset + best_len > bytes.len() {
        return null_mut();
    }

    CreateIconFromResourceEx(
        bytes[best_offset..].as_ptr(),
        best_len as u32,
        1, // TRUE = icon
        0x00030000,
        target_size,
        target_size,
        0, // LR_DEFAULTCOLOR
    )
}

struct AppState {
    icon_work: HICON,
    icon_gamer: HICON,
    hwnd: HWND,
}

static mut APP_STATE: Option<AppState> = None;

fn get_current_mode_from_disk() -> u8 {
    let state_file = get_state_file();
    if let Ok(content) = fs::read_to_string(&state_file) {
        if content.trim().eq_ignore_ascii_case("GAMER") {
            return MODE_GAMER;
        }
    }
    MODE_WORK
}

fn set_current_mode_on_disk(mode: u8) {
    let _ = fs::create_dir_all(get_scripts_dir());
    let val = if mode == MODE_GAMER { "GAMER" } else { "WORK" };
    let _ = fs::write(get_state_file(), val);
}

unsafe fn update_tray_icon(notify: bool) {
    if let Some(ref state) = APP_STATE {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = state.hwnd;
        nid.uID = ID_TRAY_ICON;
        nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
        nid.uCallbackMessage = WM_TRAY_CALLBACK;

        let mode = CURRENT_MODE.load(Ordering::SeqCst);
        let (icon, tip, title, msg) = if mode == MODE_WORK {
            (
                state.icon_work,
                "Modo: TRABALHO ☕ (Docker, WSL, Ollama Ativos)",
                "☕ Modo Trabalho Ativado!",
                "Docker, WSL e Ollama iniciados. Ambiente de programacao pronto.",
            )
        } else {
            (
                state.icon_gamer,
                "Modo: GAMER 🎮 (Recursos 100% livres para Jogo)",
                "🎮 Modo Gamer Ativado!",
                "Docker, WSL e Ollama encerrados. VRAM da RTX 3060 e CPU 100% liberados!",
            )
        };

        nid.hIcon = icon;
        nid.szTip = to_wide_buf::<128>(tip);

        if notify {
            nid.uFlags |= NIF_INFO;
            nid.szInfo = to_wide_buf::<256>(msg);
            nid.szInfoTitle = to_wide_buf::<64>(title);
            nid.dwInfoFlags = NIIF_INFO;
        }

        Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn apply_mode_switching(target_mode: u8) {
    if IS_SWITCHING.swap(true, Ordering::SeqCst) {
        return; // Already switching in background
    }

    thread::spawn(move || {
        if target_mode == MODE_GAMER {
            // 1. Kill Ollama
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", "ollama.exe", "/IM", "ollama app.exe", "/IM", "llama-server.exe"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();

            // 2. Kill Docker Desktop processes
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", "Docker Desktop.exe", "/IM", "com.docker.backend.exe", "/IM", "wslrelay.exe"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();

            // Stop Docker service
            let _ = Command::new("net")
                .args(["stop", "com.docker.service", "/y"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();

            // 3. Shutdown WSL2 (releases hypervisor RAM & CPU)
            let _ = Command::new("wsl")
                .arg("--shutdown")
                .creation_flags(CREATE_NO_WINDOW)
                .output();

            // 4. Power Plan: Ultimate / High Performance
            let _ = Command::new("powercfg")
                .args(["-setactive", "872d9590-9f19-44a3-87f5-c9cbd49ea59e"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        } else {
            // 1. Start Ollama
            let ollama_path = get_ollama_exe();
            if ollama_path.exists() {
                let _ = Command::new(&ollama_path)
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn();
            }

            // 2. Start Docker Service & Desktop
            let _ = Command::new("net")
                .args(["start", "com.docker.service"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();

            if Path::new(DOCKER_EXE).exists() {
                let _ = Command::new(DOCKER_EXE)
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn();
            }

            // 3. Start WSL
            let _ = Command::new("wsl")
                .args(["-d", "Ubuntu", "-e", "true"])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }

        // Trigger helper script if present (plays audio feedback and updates Rainmeter)
        let ps_script = get_scripts_dir().join("toggle_mode.ps1");
        if ps_script.exists() {
            let mode_str = if target_mode == MODE_GAMER { "GAMER" } else { "WORK" };
            let _ = Command::new("powershell.exe")
                .args(["-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
                .arg(&ps_script)
                .arg("-TargetMode")
                .arg(mode_str)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }

        set_current_mode_on_disk(target_mode);
        IS_SWITCHING.store(false, Ordering::SeqCst);
    });
}

unsafe fn toggle_mode(forced: Option<u8>) {
    let current = CURRENT_MODE.load(Ordering::SeqCst);
    let new_mode = match forced {
        Some(m) => m,
        None => if current == MODE_WORK { MODE_GAMER } else { MODE_WORK },
    };

    if current == new_mode && forced.is_some() {
        return;
    }

    play_sound_for_mode(new_mode);

    CURRENT_MODE.store(new_mode, Ordering::SeqCst);
    update_tray_icon(true);
    apply_mode_switching(new_mode);
}

unsafe fn show_context_menu(hwnd: HWND) {
    let hmenu = CreatePopupMenu();
    if hmenu == null_mut() {
        return;
    }

    let toggle_title = to_wide("⚡ Alternar Modo (Gamer ⇋ Trabalho)");
    AppendMenuW(hmenu, MF_STRING, ID_TOGGLE, toggle_title.as_ptr());
    SetMenuDefaultItem(hmenu, ID_TOGGLE as u32, 0);

    AppendMenuW(hmenu, MF_SEPARATOR, 0, null());

    let current = CURRENT_MODE.load(Ordering::SeqCst);
    let check_gamer = if current == MODE_GAMER { MF_CHECKED } else { MF_UNCHECKED };
    let check_work = if current == MODE_WORK { MF_CHECKED } else { MF_UNCHECKED };

    let gamer_title = to_wide("🎮 Modo Gamer (Libera VRAM & CPU)");
    AppendMenuW(hmenu, MF_STRING | check_gamer, ID_GAMER, gamer_title.as_ptr());

    let work_title = to_wide("☕ Modo Trabalho (Docker, WSL & Ollama)");
    AppendMenuW(hmenu, MF_STRING | check_work, ID_WORK, work_title.as_ptr());

    AppendMenuW(hmenu, MF_SEPARATOR, 0, null());

    let exit_title = to_wide("❌ Sair");
    AppendMenuW(hmenu, MF_STRING, ID_EXIT, exit_title.as_ptr());

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    TrackPopupMenu(
        hmenu,
        TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        null(),
    );
    DestroyMenu(hmenu);
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY_CALLBACK => match lparam as u32 {
            WM_LBUTTONUP => {
                toggle_mode(None);
                0
            }
            WM_RBUTTONUP => {
                show_context_menu(hwnd);
                0
            }
            _ => 0,
        },
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as usize;
            match id {
                ID_TOGGLE => toggle_mode(None),
                ID_GAMER => toggle_mode(Some(MODE_GAMER)),
                ID_WORK => toggle_mode(Some(MODE_WORK)),
                ID_EXIT => {
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = ID_TRAY_ICON;
            Shell_NotifyIconW(NIM_DELETE, &nid);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class_name = to_wide("ModeSwitchWindowClass");

        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: null_mut(),
        };

        RegisterClassExW(&wnd_class);

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            to_wide("ModeSwitch").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );

        if hwnd == null_mut() {
            return;
        }

        let icon_work = load_icon_from_memory(COFFEE_ICO, 32);
        let icon_gamer = load_icon_from_memory(GAMER_ICO, 32);
        let current_mode = get_current_mode_from_disk();
        CURRENT_MODE.store(current_mode, Ordering::SeqCst);

        APP_STATE = Some(AppState {
            icon_work,
            icon_gamer,
            hwnd,
        });

        // Initialize Tray Icon
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = ID_TRAY_ICON;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY_CALLBACK;
        nid.hIcon = if current_mode == MODE_WORK { icon_work } else { icon_gamer };
        let tip = if current_mode == MODE_WORK {
            "Modo: TRABALHO ☕ (Docker, WSL, Ollama Ativos)"
        } else {
            "Modo: GAMER 🎮 (Recursos 100% livres para Jogo)"
        };
        nid.szTip = to_wide_buf::<128>(tip);

        Shell_NotifyIconW(NIM_ADD, &nid);

        // Start background replay monitor (L3+R3)
        hud::start_hud_window();
        replay::start_replay_monitor();

        // Win32 Message Loop
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
