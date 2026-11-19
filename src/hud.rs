use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread;
use std::time::Instant;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_UPDATE_HUD: u32 = WM_APP + 10;
const TIMER_HUD_ANIM: usize = 1;
const COLORKEY_MAGENTA: u32 = 0x00FF00FF; // RGB(255, 0, 255)

#[derive(Copy, Clone, PartialEq)]
pub enum HudStatus {
    Idle = 0,
    Recording = 1,
    Uploading = 2,
    Sent = 3,
    Error = 4,
}

static CURRENT_STATUS: AtomicU8 = AtomicU8::new(0);
static HUD_HWND: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

static mut RECORD_START: Option<Instant> = None;
static mut UPLOAD_START: Option<Instant> = None;
static mut SENT_START: Option<Instant> = None;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn update_hud(status: HudStatus) {
    let s_val = status as u8;
    CURRENT_STATUS.store(s_val, Ordering::SeqCst);

    let hwnd = HUD_HWND.load(Ordering::SeqCst) as HWND;
    if hwnd != null_mut() {
        unsafe {
            PostMessageW(hwnd, WM_UPDATE_HUD, s_val as usize, 0);
        }
    }
}

unsafe extern "system" fn hud_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_UPDATE_HUD => {
            let status = wparam as u8;
            if status == (HudStatus::Idle as u8) {
                KillTimer(hwnd, TIMER_HUD_ANIM);
                ShowWindow(hwnd, SW_HIDE);
            } else {
                if status == (HudStatus::Recording as u8) {
                    RECORD_START = Some(Instant::now());
                } else if status == (HudStatus::Uploading as u8) {
                    UPLOAD_START = Some(Instant::now());
                } else if status == (HudStatus::Sent as u8) {
                    SENT_START = Some(Instant::now());
                }

                SetTimer(hwnd, TIMER_HUD_ANIM, 150, None);

                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_TIMER => {
            let status = CURRENT_STATUS.load(Ordering::SeqCst);
            if status == (HudStatus::Sent as u8) || status == (HudStatus::Error as u8) {
                if let Some(t) = SENT_START {
                    if t.elapsed().as_millis() > 2800 {
                        KillTimer(hwnd, TIMER_HUD_ANIM);
                        ShowWindow(hwnd, SW_HIDE);
                        CURRENT_STATUS.store(HudStatus::Idle as u8, Ordering::SeqCst);
                    }
                }
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rect);
            let w = rect.right - rect.left;
            let h = rect.bottom - rect.top;

            // Double buffering to eliminate flicker
            let mem_dc = CreateCompatibleDC(hdc);
            let mem_bmp = CreateCompatibleBitmap(hdc, w, h);
            let old_bmp = SelectObject(mem_dc, mem_bmp);

            // 1. Fill entire background with colorkey magenta (100% transparent to DWM)
            let key_brush = CreateSolidBrush(COLORKEY_MAGENTA);
            FillRect(mem_dc, &rect, key_brush);
            DeleteObject(key_brush);

            let status = CURRENT_STATUS.load(Ordering::SeqCst);
            

            SetBkMode(mem_dc, TRANSPARENT as i32);

            let font_name = to_wide("Segoe UI");
            let title_font = CreateFontW(
                20, 0, 0, 0, FW_BOLD as i32, 0, 0, 0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                font_name.as_ptr(),
            );
            let old_font = SelectObject(mem_dc, title_font);

            let cy = h / 2;
            let cx = 22;

            if status == (HudStatus::Recording as u8) {
                let (elapsed_secs, anim_tick) = if let Some(t) = RECORD_START {
                    let ms = t.elapsed().as_millis();
                    (t.elapsed().as_secs(), ((ms / 250) % 3) as i32)
                } else {
                    (0, 0)
                };

                let cur_sec = (elapsed_secs + 1).min(30);

                // --- Minimalist Animated Glowing REC Dot ---
                // Outer subtle dark shadow ring for contrast
                let shadow_brush = CreateSolidBrush(0x00151515);
                let shadow_pen = CreatePen(PS_SOLID as i32, 1, 0x00151515);
                let old_b = SelectObject(mem_dc, shadow_brush);
                let old_p = SelectObject(mem_dc, shadow_pen);
                Ellipse(mem_dc, cx - 11, cy - 11, cx + 11, cy + 11);

                // Outer pulsing red glow (3 frames)
                let glow_radius = match anim_tick {
                    0 => 8,
                    1 => 10,
                    _ => 9,
                };
                let glow_color = match anim_tick {
                    0 => 0x001B1BEE, // Red
                    1 => 0x002B2BFF, // Bright neon red
                    _ => 0x002020EE,
                };
                let glow_brush = CreateSolidBrush(glow_color);
                let glow_pen = CreatePen(PS_SOLID as i32, 1, glow_color);
                SelectObject(mem_dc, glow_brush);
                SelectObject(mem_dc, glow_pen);
                Ellipse(mem_dc, cx - glow_radius, cy - glow_radius, cx + glow_radius, cy + glow_radius);
                DeleteObject(glow_brush);
                DeleteObject(glow_pen);

                // Inner intense core
                let core_brush = CreateSolidBrush(0x000000FF); // Pure red
                let core_pen = CreatePen(PS_SOLID as i32, 1, 0x000000FF);
                SelectObject(mem_dc, core_brush);
                SelectObject(mem_dc, core_pen);
                Ellipse(mem_dc, cx - 5, cy - 5, cx + 5, cy + 5);
                DeleteObject(core_brush);
                DeleteObject(core_pen);

                // Center specular glint (white dot)
                let glint_brush = CreateSolidBrush(0x00FFFFFF);
                let glint_pen = CreatePen(PS_SOLID as i32, 1, 0x00FFFFFF);
                SelectObject(mem_dc, glint_brush);
                SelectObject(mem_dc, glint_pen);
                Ellipse(mem_dc, cx - 2, cy - 3, cx + 1, cy);
                DeleteObject(glint_brush);
                DeleteObject(glint_pen);

                SelectObject(mem_dc, old_b);
                SelectObject(mem_dc, old_p);
                DeleteObject(shadow_brush);
                DeleteObject(shadow_pen);

                // --- Live Timer Text (e.g. "05s / 30s") ---
                let text_str = format!("{:02}s / 30s", cur_sec);
                let text_w = to_wide(&text_str);

                // Text drop shadow (dark charcoal)
                SetTextColor(mem_dc, 0x00101010);
                let mut shadow_rect = RECT {
                    left: 42,
                    top: 13,
                    right: w,
                    bottom: h,
                };
                DrawTextW(mem_dc, text_w.as_ptr(), -1, &mut shadow_rect, DT_LEFT | DT_SINGLELINE);

                // Text crisp white foreground
                SetTextColor(mem_dc, 0x00FFFFFF);
                let mut fg_rect = RECT {
                    left: 40,
                    top: 11,
                    right: w,
                    bottom: h,
                };
                DrawTextW(mem_dc, text_w.as_ptr(), -1, &mut fg_rect, DT_LEFT | DT_SINGLELINE);

            } else if status == (HudStatus::Uploading as u8) {
                let anim_tick = if let Some(t) = UPLOAD_START {
                    ((t.elapsed().as_millis() / 280) % 3) as usize
                } else {
                    0
                };

                // WhatsApp brand green circle (RGB: 37, 211, 102 -> BGR: 0x0066D325)
                let shadow_brush = CreateSolidBrush(0x00151515);
                let shadow_pen = CreatePen(PS_SOLID as i32, 1, 0x00151515);
                let old_b = SelectObject(mem_dc, shadow_brush);
                let old_p = SelectObject(mem_dc, shadow_pen);
                Ellipse(mem_dc, cx - 11, cy - 11, cx + 11, cy + 11);

                let wpp_brush = CreateSolidBrush(0x0066D325);
                let wpp_pen = CreatePen(PS_SOLID as i32, 1, 0x0066D325);
                SelectObject(mem_dc, wpp_brush);
                SelectObject(mem_dc, wpp_pen);
                Ellipse(mem_dc, cx - 9, cy - 9, cx + 9, cy + 9);

                // White paper airplane / upload arrow icon inside
                let icon_pen = CreatePen(PS_SOLID as i32, 2, 0x00FFFFFF);
                SelectObject(mem_dc, icon_pen);
                MoveToEx(mem_dc, cx - 3, cy + 3, null_mut());
                LineTo(mem_dc, cx + 3, cy - 3);
                MoveToEx(mem_dc, cx - 2, cy - 3, null_mut());
                LineTo(mem_dc, cx + 3, cy - 3);
                LineTo(mem_dc, cx + 3, cy + 2);
                DeleteObject(icon_pen);

                SelectObject(mem_dc, old_b);
                SelectObject(mem_dc, old_p);
                DeleteObject(shadow_brush);
                DeleteObject(shadow_pen);
                DeleteObject(wpp_brush);
                DeleteObject(wpp_pen);

                // Animated Dots: "WhatsApp .", "WhatsApp . .", "WhatsApp . . ."
                let dots = match anim_tick {
                    0 => "WhatsApp .",
                    1 => "WhatsApp . .",
                    _ => "WhatsApp . . .",
                };
                let text_w = to_wide(dots);

                SetTextColor(mem_dc, 0x00101010);
                let mut shadow_rect = RECT { left: 42, top: 13, right: w, bottom: h };
                DrawTextW(mem_dc, text_w.as_ptr(), -1, &mut shadow_rect, DT_LEFT | DT_SINGLELINE);

                SetTextColor(mem_dc, 0x00FFFFFF);
                let mut fg_rect = RECT { left: 40, top: 11, right: w, bottom: h };
                DrawTextW(mem_dc, text_w.as_ptr(), -1, &mut fg_rect, DT_LEFT | DT_SINGLELINE);

            } else if status == (HudStatus::Sent as u8) {
                // Vibrant green circle with checkmark
                let shadow_brush = CreateSolidBrush(0x00151515);
                let shadow_pen = CreatePen(PS_SOLID as i32, 1, 0x00151515);
                let old_b = SelectObject(mem_dc, shadow_brush);
                let old_p = SelectObject(mem_dc, shadow_pen);
                Ellipse(mem_dc, cx - 11, cy - 11, cx + 11, cy + 11);

                let ok_brush = CreateSolidBrush(0x0044DD44);
                let ok_pen = CreatePen(PS_SOLID as i32, 1, 0x0044DD44);
                SelectObject(mem_dc, ok_brush);
                SelectObject(mem_dc, ok_pen);
                Ellipse(mem_dc, cx - 9, cy - 9, cx + 9, cy + 9);

                // Crisp Checkmark
                let check_pen = CreatePen(PS_SOLID as i32, 2, 0x00FFFFFF);
                SelectObject(mem_dc, check_pen);
                MoveToEx(mem_dc, cx - 4, cy, null_mut());
                LineTo(mem_dc, cx - 1, cy + 3);
                LineTo(mem_dc, cx + 4, cy - 3);
                DeleteObject(check_pen);

                SelectObject(mem_dc, old_b);
                SelectObject(mem_dc, old_p);
                DeleteObject(shadow_brush);
                DeleteObject(shadow_pen);
                DeleteObject(ok_brush);
                DeleteObject(ok_pen);

                let text_w = to_wide("Salvo!");
                SetTextColor(mem_dc, 0x00101010);
                let mut shadow_rect = RECT { left: 42, top: 13, right: w, bottom: h };
                DrawTextW(mem_dc, text_w.as_ptr(), -1, &mut shadow_rect, DT_LEFT | DT_SINGLELINE);

                SetTextColor(mem_dc, 0x0044EE44);
                let mut fg_rect = RECT { left: 40, top: 11, right: w, bottom: h };
                DrawTextW(mem_dc, text_w.as_ptr(), -1, &mut fg_rect, DT_LEFT | DT_SINGLELINE);
            }

            SelectObject(mem_dc, old_font);
            DeleteObject(title_font);

            // Blit final frame to window DC
            BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);

            SelectObject(mem_dc, old_bmp);
            DeleteObject(mem_bmp);
            DeleteDC(mem_dc);

            EndPaint(hwnd, &mut ps);
            0
        }
        WM_ERASEBKGND => 1,
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn start_hud_window() {
    thread::spawn(|| {
        unsafe {
            let instance = GetModuleHandleW(null());
            let class_name = to_wide("ReplayHudClass");

            let wnd_class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(hud_wnd_proc),
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

            let screen_w = GetSystemMetrics(SM_CXSCREEN);
            let width = 190;
            let height = 48;
            let x = screen_w - width - 30;
            let y = 25;

            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                class_name.as_ptr(),
                to_wide("ReplayHUD").as_ptr(),
                WS_POPUP,
                x,
                y,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );

            if hwnd == null_mut() {
                return;
            }

            // Magenta colorkey gives 100% full background transparency
            SetLayeredWindowAttributes(hwnd, COLORKEY_MAGENTA, 0, LWA_COLORKEY);

            HUD_HWND.store(hwnd as usize, Ordering::SeqCst);

            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
}
