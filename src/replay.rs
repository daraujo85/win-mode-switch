use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use crate::hud::{update_hud, HudStatus};

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[repr(C)]
struct XInputGamepad {
    w_buttons: u16,
    b_left_trigger: u8,
    b_right_trigger: u8,
    s_thumb_lx: i16,
    s_thumb_ly: i16,
    s_thumb_rx: i16,
    s_thumb_ry: i16,
}

#[repr(C)]
struct XInputState {
    dw_packet_number: u32,
    gamepad: XInputGamepad,
}

type XInputGetStateFn = unsafe extern "system" fn(dw_user_index: u32, p_state: *mut XInputState) -> u32;

const XINPUT_GAMEPAD_BACK: u16 = 0x0020;
const XINPUT_GAMEPAD_START: u16 = 0x0010;
const XINPUT_GAMEPAD_LEFT_THUMB: u16 = 0x0040;  // L3
const XINPUT_GAMEPAD_RIGHT_THUMB: u16 = 0x0080; // R3

static IS_RECORDING: AtomicBool = AtomicBool::new(false);

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn log_msg(msg: &str) {
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(userprofile).join(r"scripts\replay.log");
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _ = writeln!(f, "[{}] {}", ts, msg);
        }
    }
}

fn get_status_file() -> PathBuf {
    PathBuf::from(r"F:\SP Football Life 2027\SiderAddons\replay_status.txt")
}

fn get_replays_dir() -> PathBuf {
    PathBuf::from(r"F:\SP Football Life 2027\Replays")
}

fn update_status(status: &str) {
    let p = get_status_file();
    let _ = fs::write(p, status);
}

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn to_base64(bytes: &[u8]) -> String {
    let mut result = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        result.push(B64_CHARS[(b0 >> 2) as usize] as char);
        result.push(B64_CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            result.push(B64_CHARS[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(B64_CHARS[(b2 & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn send_to_whatsapp(file_path: &PathBuf) -> bool {
    let bytes = match fs::read(file_path) {
        Ok(b) => b,
        Err(e) => {
            log_msg(&format!("Failed to read MP4 for WhatsApp: {}", e));
            return false;
        }
    };

    let b64 = to_base64(&bytes);
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("replay_gol.mp4");

    let payload = format!(
        r#"{{"phoneNumber":"5521995278672","filename":"{}","caption":"⚽🔥 Golaço no SP Football Life 2027! Replay direto da TV.","base64":"data:video/mp4;base64,{}"}}"#,
        filename, b64
    );

    let temp_json = std::env::temp_dir().join(format!("wpp_payload_{}.json", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()));
    if let Err(e) = fs::write(&temp_json, payload) {
        log_msg(&format!("Failed to write payload JSON: {}", e));
        return false;
    }

    log_msg("Sending replay video to WhatsApp API...");
    let out = Command::new("curl.exe")
        .args([
            "-s",
            "-X", "POST",
            "-H", "Content-Type: application/json",
            "-d", &format!("@{}", temp_json.display()),
            "https://wpp.boletoazap.dev.br/development/sendVideoMessage",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let _ = fs::remove_file(temp_json);

    match out {
        Ok(res) => {
            let ok = res.status.success();
            let resp_str = String::from_utf8_lossy(&res.stdout);
            log_msg(&format!("WhatsApp API response (success={}): {}", ok, resp_str));
            ok
        }
        Err(e) => {
            log_msg(&format!("curl execution failed: {}", e));
            false
        }
    }
}

fn record_and_upload() {
    log_msg("Replay capture triggered!");
    let replays_dir = get_replays_dir();
    let _ = fs::create_dir_all(&replays_dir);

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let output_file = replays_dir.join(format!("gol_{}.mp4", ts));

    update_status("RECORDING");
    update_hud(HudStatus::Recording);
    log_msg(&format!("Recording 30s clip to {}", output_file.display()));

    let temp_v = replays_dir.join(format!("temp_v_{}.mp4", ts));
    let temp_a = replays_dir.join(format!("temp_a_{}.m4a", ts));

    log_msg("Starting isolated parallel capture (Video-only 60fps + Audio-only)...");

    // Spawn Audio capture in isolated process so it NEVER blocks video frame queue
    let a_spawn = Command::new(r"C:\ProgramData\chocolatey\bin\ffmpeg.exe")
        .args([
            "-y",
            "-f", "dshow",
            "-i", "audio=virtual-audio-capturer",
            "-c:a", "aac",
            "-b:a", "128k",
            "-t", "30",
            temp_a.to_str().unwrap_or("temp_a.m4a"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();

    // Spawn Video capture in isolated process - pure 60 FPS hardware NVENC, zero audio contention
    let v_spawn = Command::new(r"C:\ProgramData\chocolatey\bin\ffmpeg.exe")
        .args([
            "-y",
            "-f", "lavfi",
            "-i", "ddagrab=framerate=60:draw_mouse=0",
            "-c:v", "h264_nvenc",
            "-preset", "p1",
            "-tune", "ll",
            "-b:v", "4500k",
            "-maxrate", "6000k",
            "-bufsize", "8M",
            "-t", "30",
            temp_v.to_str().unwrap_or("temp_v.mp4"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();

    if let Ok(mut v_child) = v_spawn {
        let _ = v_child.wait();
    }
    if let Ok(mut a_child) = a_spawn {
        let _ = a_child.wait();
    }

    // Fallback: If ddagrab failed, capture with gdigrab video-only
    if !temp_v.exists() || fs::metadata(&temp_v).map(|m| m.len()).unwrap_or(0) < 5000 {
        log_msg("ddagrab video-only failed, falling back to gdigrab video-only...");
        let _ = Command::new(r"C:\ProgramData\chocolatey\bin\ffmpeg.exe")
            .args([
                "-y",
                "-f", "gdigrab",
                "-framerate", "60",
                "-draw_mouse", "0",
                "-i", "desktop",
                "-c:v", "h264_nvenc",
                "-preset", "p1",
                "-tune", "ll",
                "-b:v", "4000k",
                "-t", "30",
                temp_v.to_str().unwrap_or("temp_v.mp4"),
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    log_msg("Muxing video and audio streams into final MP4...");
    if temp_v.exists() && temp_a.exists() {
        let _ = Command::new(r"C:\ProgramData\chocolatey\bin\ffmpeg.exe")
            .args([
                "-y",
                "-i", temp_v.to_str().unwrap_or("temp_v.mp4"),
                "-i", temp_a.to_str().unwrap_or("temp_a.m4a"),
                "-c:v", "copy",
                "-c:a", "copy",
                "-shortest",
                output_file.to_str().unwrap_or("replay.mp4"),
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    } else if temp_v.exists() {
        let _ = fs::copy(&temp_v, &output_file);
    }

    let _ = fs::remove_file(&temp_v);
    let _ = fs::remove_file(&temp_a);

    let file_size = fs::metadata(&output_file).map(|m| m.len()).unwrap_or(0);
    log_msg(&format!("Finished recording, output size: {} bytes", file_size));

    if output_file.exists() && file_size > 10000 {
        update_status("UPLOADING");
        update_hud(HudStatus::Uploading);
        send_to_whatsapp(&output_file);
        update_status("SENT");
        update_hud(HudStatus::Sent);
    } else {
        log_msg("Recording failed or file too small");
        update_status("ERROR");
        update_hud(HudStatus::Error);
    }

    thread::sleep(Duration::from_secs(3));
    update_status("IDLE");
    update_hud(HudStatus::Idle);
    IS_RECORDING.store(false, Ordering::SeqCst);
    log_msg("Replay capture cycle complete, returned to IDLE");
}

pub fn start_replay_monitor() {
    log_msg("Replay monitor starting...");
    thread::spawn(|| {
        let xinput_lib = unsafe { LoadLibraryW(to_wide("xinput1_4.dll").as_ptr()) };
        let hmodule: HMODULE = if xinput_lib.is_null() {
            unsafe { LoadLibraryW(to_wide("xinput1_3.dll").as_ptr()) }
        } else {
            xinput_lib
        };

        if hmodule.is_null() {
            log_msg("Failed to load xinput DLL");
            return;
        }

        let proc_name = std::ffi::CString::new("XInputGetState").unwrap();
        let get_state: XInputGetStateFn = unsafe {
            let addr = GetProcAddress(hmodule, proc_name.as_ptr() as *const u8);
            if addr.is_none() {
                log_msg("Failed to find XInputGetState address");
                return;
            }
            std::mem::transmute(addr.unwrap())
        };

        log_msg("XInput loaded successfully, listening on Slots 0..3 for L3+R3 or Back+Start");

        let mut last_l3: [Option<Instant>; 4] = [None, None, None, None];
        let mut last_r3: [Option<Instant>; 4] = [None, None, None, None];
        let mut trigger_held = false;

        loop {
            thread::sleep(Duration::from_millis(30));

            if IS_RECORDING.load(Ordering::SeqCst) {
                continue;
            }

            let trigger_file = PathBuf::from(r"C:\Users\usuario\scripts\trigger_replay.txt");
            if trigger_file.exists() {
                let _ = fs::remove_file(&trigger_file);
                if !trigger_held {
                    trigger_held = true;
                    IS_RECORDING.store(true, Ordering::SeqCst);
                    thread::spawn(|| {
                        record_and_upload();
                    });
                    continue;
                }
            }

            let now = Instant::now();
            let mut combo_detected = false;

            for slot in 0..4u32 {
                let mut state: XInputState = unsafe { std::mem::zeroed() };
                let res = unsafe { get_state(slot, &mut state) };
                if res != 0 {
                    continue;
                }

                let buttons = state.gamepad.w_buttons;
                let s_idx = slot as usize;

                let l3_pressed = (buttons & XINPUT_GAMEPAD_LEFT_THUMB) != 0;
                let r3_pressed = (buttons & XINPUT_GAMEPAD_RIGHT_THUMB) != 0;

                if l3_pressed {
                    last_l3[s_idx] = Some(now);
                }
                if r3_pressed {
                    last_r3[s_idx] = Some(now);
                }

                // Combo is active if BOTH L3 and R3 were pressed within a 450ms window
                let l3_recent = last_l3[s_idx]
                    .map(|t| now.duration_since(t) < Duration::from_millis(450))
                    .unwrap_or(false);
                let r3_recent = last_r3[s_idx]
                    .map(|t| now.duration_since(t) < Duration::from_millis(450))
                    .unwrap_or(false);

                let l3_r3 = (l3_pressed && r3_pressed) || (l3_recent && r3_recent);

                let back_start = (buttons & (XINPUT_GAMEPAD_BACK | XINPUT_GAMEPAD_START))
                    == (XINPUT_GAMEPAD_BACK | XINPUT_GAMEPAD_START);

                if l3_r3 || back_start {
                    combo_detected = true;
                    // Reset latches
                    last_l3[s_idx] = None;
                    last_r3[s_idx] = None;
                    break;
                }
            }

            if combo_detected {
                if !trigger_held {
                    trigger_held = true;
                    IS_RECORDING.store(true, Ordering::SeqCst);
                    thread::spawn(|| {
                        record_and_upload();
                    });
                }
            } else {
                trigger_held = false;
            }
        }
    });
}
