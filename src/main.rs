// screen_mask — หน้าต่าง dark mode โปร่งใส + สไลเดอร์ปรับความทึบ + ปุ่ม On top ในแถบหัว
#![windows_subsystem = "windows"]

use std::cell::{Cell, RefCell};
use std::ptr::null;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, Ellipse, EndPaint, FillRect,
    FrameRect, GetStockObject, HFONT, InvalidateRect, PAINTSTRUCT, ScreenToClient, SelectObject,
    SetBkMode, SetTextColor, TextOutW, NULL_PEN, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

// ---------- ชุดสีเข้มกลาง (โทนเดียวกับ ui_theme ของโปรเจกต์) ----------
const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}
const C_PLATE: u32 = rgb(30, 34, 42); // พื้นหลัง
const C_BAR: u32 = rgb(10, 11, 14); // แถบสไลเดอร์ (ดำ)
const C_TEXT: u32 = rgb(228, 232, 240); // ตัวหนังสือ
const C_DIM: u32 = rgb(146, 155, 170); // ตัวหนังสือรอง
const C_EDGE: u32 = rgb(232, 196, 96); // ขอบเหลือง
const C_HEAD: u32 = rgb(21, 24, 30); // แถบหัว

const WIN_W: i32 = 800;
const WIN_H: i32 = 600;

const TITLE_H: i32 = 32;
const BTN_W: i32 = 46;
const BTN_N: usize = 4; // 0 = on top · 1 = ย่อ · 2 = ขยาย · 3 = ปิด
const BTN_LABELS: [&str; 4] = ["T", "-", "\u{25A1}", "X"]; // □ = ขยาย

const TEXT_Y: i32 = TITLE_H + 18;
const TRACK_X: i32 = 26;
const TRACK_Y: i32 = TITLE_H + 74;
const TRACK_H: i32 = 10;
const THUMB: i32 = 16;
const EDGE_M: i32 = 8; // ขอบสำหรับลากย่อ/ขยาย

thread_local! {
    static PCT: Cell<i32> = const { Cell::new(60) };
    static TOPMOST: Cell<bool> = const { Cell::new(false) };
    // โหมดเมาส์: 0=ว่าง 1=ลากย้าย 2=สไลเดอร์ 3=ย่อ/ขยาย
    static MODE: Cell<u8> = const { Cell::new(0) };
    static RZF: Cell<u32> = const { Cell::new(0) }; // 1=ซ้าย 2=ขวา 4=บน 8=ล่าง
    static SX: Cell<i32> = const { Cell::new(0) };
    static SY: Cell<i32> = const { Cell::new(0) };
    static SL: Cell<i32> = const { Cell::new(0) };
    static ST: Cell<i32> = const { Cell::new(0) };
    static SR: Cell<i32> = const { Cell::new(0) };
    static SB: Cell<i32> = const { Cell::new(0) };
    static FONT: Cell<isize> = const { Cell::new(0) };
    static FONT_S: Cell<isize> = const { Cell::new(0) };
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy)]
struct Rct {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}
impl Rct {
    fn inside(self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn log(msg: &str) {
    use std::io::Write as _;
    let p = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("screen_mask.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{msg}");
    }
}

// 0% = 20 (จางสุดที่ยังเห็นสไลเดอร์) · 100% = 255 (ทึบสุด)
fn alpha_of(pct: i32) -> u8 {
    (20 + pct.clamp(0, 100) * 235 / 100) as u8
}

fn track_rect(cw: i32) -> (i32, i32) {
    (TRACK_X, (cw - TRACK_X).max(TRACK_X + 60))
}

fn btn_rect(i: usize, cw: i32) -> Rct {
    let x = cw - BTN_W * (BTN_N as i32 - i as i32);
    Rct { x, y: 0, w: BTN_W, h: TITLE_H }
}

unsafe fn set_pct(hwnd: HWND, pct: i32) {
    let pct = pct.clamp(0, 100);
    PCT.with(|c| c.set(pct));
    SetLayeredWindowAttributes(hwnd, 0, alpha_of(pct), LWA_ALPHA);
    InvalidateRect(hwnd, null(), 1);
}

unsafe fn toggle_topmost(hwnd: HWND) {
    let on = !TOPMOST.with(|c| c.get());
    TOPMOST.with(|c| c.set(on));
    let r = SetWindowPos(
        hwnd,
        if on { HWND_TOPMOST } else { HWND_NOTOPMOST },
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
    );
    log(&format!("toggle_topmost on={on} setwindowpos_ok={r}"));
    InvalidateRect(hwnd, null(), 1);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCCALCSIZE => {
            if wp == 1 {
                0
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
        WM_NCHITTEST => {
            // ทำลาก/ย่อขยายเอง → คืน HTCLIENT หมด เพื่อรับเมาส์เอง (ไม่มีกรอบขาวของระบบ)
            HTCLIENT as LRESULT
        }
        WM_NCACTIVATE => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let cw = rc.right;
            let ch = rc.bottom;

            // พื้นหลัง
            let bg = CreateSolidBrush(C_PLATE);
            FillRect(hdc, &rc, bg);
            DeleteObject(bg as _);

            // แถบหัว
            let hd = CreateSolidBrush(C_HEAD);
            let hr = RECT { left: 0, top: 0, right: cw, bottom: TITLE_H };
            FillRect(hdc, &hr, hd);
            DeleteObject(hd as _);

            let font = FONT.with(|c| c.get()) as HFONT;
            let font_s = FONT_S.with(|c| c.get()) as HFONT;

            // ชื่อหน้าต่าง
            let old = SelectObject(hdc, font_s as _);
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, C_DIM);
            let name: Vec<u16> = "screen mask".encode_utf16().collect();
            TextOutW(hdc, 12, 8, name.as_ptr(), name.len() as i32);
            SelectObject(hdc, old);

            // ปุ่มบนแถบหัว
            let on_top = TOPMOST.with(|c| c.get());
            for (i, label) in BTN_LABELS.iter().enumerate() {
                let b = btn_rect(i, cw);
                let active = i == 0 && on_top;
                if active {
                    let fb = CreateSolidBrush(C_EDGE);
                    let fr = RECT { left: b.x, top: b.y, right: b.x + b.w, bottom: b.y + b.h };
                    FillRect(hdc, &fr, fb);
                    DeleteObject(fb as _);
                }
                let col = if active { rgb(16, 18, 22) } else { C_TEXT };
                let txt: Vec<u16> = label.encode_utf16().collect();
                let old = SelectObject(hdc, font_s as _);
                SetTextColor(hdc, col);
                let tx = b.x + if *label == "-" { 20 } else { 18 };
                TextOutW(hdc, tx, 8, txt.as_ptr(), txt.len() as i32);
                SelectObject(hdc, old);
            }

            // ตัวเลข %
            let old = SelectObject(hdc, font as _);
            SetTextColor(hdc, C_TEXT);
            let pct = PCT.with(|c| c.get());
            let s = format!("{}%", pct);
            let t: Vec<u16> = s.encode_utf16().collect();
            TextOutW(hdc, TRACK_X, TEXT_Y, t.as_ptr(), t.len() as i32);
            SelectObject(hdc, old);

            // แถบสไลเดอร์
            let (tx, tx2) = track_rect(cw);
            let bar = CreateSolidBrush(C_BAR);
            let rb = RECT { left: tx, top: TRACK_Y, right: tx2, bottom: TRACK_Y + TRACK_H };
            FillRect(hdc, &rb, bar);
            DeleteObject(bar as _);

            // หัวหมุน กลม — วงแหวนเหลือง (วงกลมทึบ 2 ชั้น)
            let pos_x = tx + (tx2 - tx) * pct / 100;
            let cy = TRACK_Y + TRACK_H / 2;
            let r = THUMB / 2;
            let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
            let ring = CreateSolidBrush(C_EDGE);
            let ob = SelectObject(hdc, ring as _);
            Ellipse(hdc, pos_x - r, cy - r, pos_x + r, cy + r);
            let inner = CreateSolidBrush(C_BAR);
            SelectObject(hdc, inner as _);
            let ir = (r - 3).max(1);
            Ellipse(hdc, pos_x - ir, cy - ir, pos_x + ir, cy + ir);
            SelectObject(hdc, ob);
            SelectObject(hdc, old_pen);
            DeleteObject(ring as _);
            DeleteObject(inner as _);

            // ขอบเหลืองรอบหน้าต่าง (ให้เห็นไว้จับลากขอบ)
            let edge = CreateSolidBrush(C_EDGE);
            let r1 = RECT { left: 0, top: 0, right: cw, bottom: ch };
            FrameRect(hdc, &r1, edge);
            let r2 = RECT { left: 1, top: 1, right: cw - 1, bottom: ch - 1 };
            FrameRect(hdc, &r2, edge);
            DeleteObject(edge as _);

            EndPaint(hwnd, &ps);
            0
        }
        WM_LBUTTONDOWN => {
            let cx = (lp & 0xFFFF) as u16 as i16 as i32;
            let cy = ((lp >> 16) & 0xFFFF) as u16 as i16 as i32;
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let (cw, ch) = (rc.right, rc.bottom);

            // ปุ่มบนแถบหัว
            for i in 0..BTN_N {
                if btn_rect(i, cw).inside(cx, cy) {
                    match i {
                        0 => toggle_topmost(hwnd),
                        1 => {
                            ShowWindow(hwnd, SW_MINIMIZE);
                        }
                        2 => {
                            if IsZoomed(hwnd) != 0 {
                                ShowWindow(hwnd, SW_RESTORE);
                            } else {
                                ShowWindow(hwnd, SW_MAXIMIZE);
                            }
                        }
                        _ => {
                            DestroyWindow(hwnd);
                        }
                    }
                    return 0;
                }
            }

            // จำจุดเริ่มไว้คิดระยะ delta
            let mut cp = POINT { x: 0, y: 0 };
            GetCursorPos(&mut cp);
            SX.with(|c| c.set(cp.x));
            SY.with(|c| c.set(cp.y));
            let mut wr: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut wr);
            SL.with(|c| c.set(wr.left));
            ST.with(|c| c.set(wr.top));
            SR.with(|c| c.set(wr.right));
            SB.with(|c| c.set(wr.bottom));

            let on_slider = cy >= TRACK_Y - 14 && cy <= TRACK_Y + TRACK_H + 14;

            // ขอบ 8px = ย่อ/ขยาย
            let mut f = 0u32;
            if cx < EDGE_M {
                f |= 1;
            }
            if cx >= cw - EDGE_M {
                f |= 2;
            }
            if cy < EDGE_M {
                f |= 4;
            }
            if cy >= ch - EDGE_M {
                f |= 8;
            }
            if f != 0 && !on_slider {
                RZF.with(|c| c.set(f));
                MODE.with(|c| c.set(3));
                SetCapture(hwnd);
                return 0;
            }

            // แถบสไลเดอร์
            if on_slider {
                MODE.with(|c| c.set(2));
                SetCapture(hwnd);
                let (tx, tx2) = track_rect(cw);
                let p = (cx - tx) * 100 / (tx2 - tx).max(1);
                set_pct(hwnd, p);
                return 0;
            }

            // ที่เหลือ = คลิกแล้วลากหน้าต่าง
            MODE.with(|c| c.set(1));
            SetCapture(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let mode = MODE.with(|c| c.get());
            if mode == 0 {
                return 0;
            }
            let mut cp = POINT { x: 0, y: 0 };
            GetCursorPos(&mut cp);
            let dx = cp.x - SX.with(|c| c.get());
            let dy = cp.y - SY.with(|c| c.get());
            match mode {
                1 => {
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        SL.with(|c| c.get()) + dx,
                        ST.with(|c| c.get()) + dy,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                2 => {
                    let mut rc: RECT = std::mem::zeroed();
                    GetClientRect(hwnd, &mut rc);
                    let (tx, tx2) = track_rect(rc.right);
                    let mut cp2 = POINT { x: cp.x, y: cp.y };
                    ScreenToClient(hwnd, &mut cp2);
                    let p = (cp2.x - tx) * 100 / (tx2 - tx).max(1);
                    set_pct(hwnd, p);
                }
                _ => {
                    let (mut l, mut t, mut r, mut b) = (
                        SL.with(|c| c.get()),
                        ST.with(|c| c.get()),
                        SR.with(|c| c.get()),
                        SB.with(|c| c.get()),
                    );
                    let f = RZF.with(|c| c.get());
                    if f & 1 != 0 {
                        l += dx;
                    }
                    if f & 2 != 0 {
                        r += dx;
                    }
                    if f & 4 != 0 {
                        t += dy;
                    }
                    if f & 8 != 0 {
                        b += dy;
                    }
                    if r - l < 240 {
                        if f & 1 != 0 {
                            l = r - 240;
                        } else {
                            r = l + 240;
                        }
                    }
                    if b - t < 160 {
                        if f & 4 != 0 {
                            t = b - 160;
                        } else {
                            b = t + 160;
                        }
                    }
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        l,
                        t,
                        r - l,
                        b - t,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            0
        }
        WM_LBUTTONUP => {
            if MODE.with(|c| c.get()) != 0 {
                MODE.with(|c| c.set(0));
                ReleaseCapture();
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            InvalidateRect(hwnd, null(), 1);
            0
        }
        WM_DESTROY => {
            let f = FONT.with(|c| c.get());
            if f != 0 {
                DeleteObject(f as _);
            }
            let fs = FONT_S.with(|c| c.get());
            if fs != 0 {
                DeleteObject(fs as _);
            }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let cls = w("ScreenMaskWndV2");
        let hinst = GetModuleHandleW(null());
        let wc: WNDCLASSW = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: null(),
            lpszClassName: cls.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            log("RegisterClassW failed");
            return;
        }

        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_APPWINDOW,
            cls.as_ptr(),
            w("screen mask").as_ptr(),
            WS_POPUP | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_VISIBLE,
            (sw - WIN_W) / 2,
            (sh - WIN_H) / 2,
            WIN_W,
            WIN_H,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            null(),
        );
        if hwnd.is_null() {
            log("CreateWindowExW failed");
            return;
        }

        // title bar โหมดดำ (กันขอบขาวของ DWM ตอน maximize)
        let on: i32 = 1;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &on as *const i32 as *const std::ffi::c_void,
            4,
        );
        // ตัดกรอบขาวของ DWM ที่โพ่ขึ้นตอนหน้าต่างไม่ active + มุมไม่โค้ง
        let ncrp: i32 = DWMNCRP_DISABLED;
        let r_ncrp = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY as u32,
            &ncrp as *const i32 as *const std::ffi::c_void,
            4,
        );
        let blk: u32 = 0x0000_0000;
        let r_border = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &blk as *const u32 as *const std::ffi::c_void,
            4,
        );
        let noround: i32 = DWMWCP_DONOTROUND;
        let r_corner = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &noround as *const i32 as *const std::ffi::c_void,
            4,
        );
        log(&format!(
            "dwm ncrp={r_ncrp} border={r_border} corner={r_corner}"
        ));

        let face = w("Tahoma");
        let f = CreateFontW(-20, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
        let fs = CreateFontW(-16, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
        FONT.with(|c| c.set(f as isize));
        FONT_S.with(|c| c.set(fs as isize));

        SetLayeredWindowAttributes(hwnd, 0, alpha_of(60), LWA_ALPHA);
        ShowWindow(hwnd, SW_SHOW);
        InvalidateRect(hwnd, null(), 1);
        log(&format!("start ok hwnd={:?}", hwnd));

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
