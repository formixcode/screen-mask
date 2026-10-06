// screen_mask — หน้าต่าง dark mode โปร่งใส + สไลเดอร์ปรับความทึบ (มีขอบให้เห็นไว้ลาก)
#![windows_subsystem = "windows"]

use std::cell::Cell;
use std::ptr::null;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, Ellipse, EndPaint, FillRect,
    FrameRect, GetStockObject, HFONT, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode,
    SetTextColor, TextOutW, NULL_BRUSH, PS_SOLID, TRANSPARENT,
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
const C_EDGE: u32 = rgb(232, 196, 96); // ขอบเหลือง (หน้าต่าง + หัวหมุน)

const WIN_W: i32 = 800;
const WIN_H: i32 = 600;

// ตำแหน่งสไลเดอร์
const TRACK_X: i32 = 26;
const TRACK_Y: i32 = 104;
const TRACK_H: i32 = 10;
const THUMB: i32 = 14;

thread_local! {
    static PCT: Cell<i32> = const { Cell::new(60) };
    static DRAG: Cell<bool> = const { Cell::new(false) };
    static FONT: Cell<isize> = const { Cell::new(0) };
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// 0% = 20 (จางสุดที่ยังเห็นสไลเดอร์) · 100% = 255 (ทึบสุด)
fn alpha_of(pct: i32) -> u8 {
    (20 + pct.clamp(0, 100) * 235 / 100) as u8
}

fn track_rect(cw: i32) -> (i32, i32) {
    (TRACK_X, (cw - TRACK_X).max(TRACK_X + 60))
}

unsafe fn set_pct(hwnd: HWND, pct: i32) {
    let pct = pct.clamp(0, 100);
    PCT.with(|c| c.set(pct));
    SetLayeredWindowAttributes(hwnd, 0, alpha_of(pct), LWA_ALPHA);
    InvalidateRect(hwnd, null(), 1);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let cw = rc.right;
            let ch = rc.bottom;

            // พื้นหลังดำ
            let bg = CreateSolidBrush(C_PLATE);
            FillRect(hdc, &rc, bg);
            DeleteObject(bg as _);

            // ขอบหน้าต่าง 2px สีเหลือง (ให้เห็นไว้ลากขอบ)
            let edge = CreateSolidBrush(C_EDGE);
            let r1 = RECT { left: 0, top: 0, right: cw, bottom: ch };
            FrameRect(hdc, &r1, edge);
            let r2 = RECT { left: 1, top: 1, right: cw - 1, bottom: ch - 1 };
            FrameRect(hdc, &r2, edge);

            // ตัวเลข % (ขาว)
            let font = FONT.with(|c| c.get()) as HFONT;
            let old = SelectObject(hdc, font as _);
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, C_TEXT);
            let pct = PCT.with(|c| c.get());
            let line1 = format!("{}%", pct);
            let t1: Vec<u16> = line1.encode_utf16().collect();
            TextOutW(hdc, TRACK_X, 22, t1.as_ptr(), t1.len() as i32);
            SelectObject(hdc, old);

            // รางสไลเดอร์
            let (tx, tx2) = track_rect(cw);
            let bar = CreateSolidBrush(C_BAR);
            let rb = RECT { left: tx, top: TRACK_Y, right: tx2, bottom: TRACK_Y + TRACK_H };
            FillRect(hdc, &rb, bar);
            DeleteObject(bar as _);

            // หัวหมุน กลม เล็ก มีขอบเหลือง (พื้นในวงกลมโปร่ง)
            let pos_x = tx + (tx2 - tx) * pct / 100;
            let cy = TRACK_Y + TRACK_H / 2;
            let pen = CreatePen(PS_SOLID, 2, C_EDGE);
            let old_pen = SelectObject(hdc, pen as _);
            let old_br = SelectObject(hdc, GetStockObject(NULL_BRUSH));
            Ellipse(
                hdc,
                pos_x - THUMB / 2,
                cy - THUMB / 2,
                pos_x + THUMB / 2,
                cy + THUMB / 2,
            );
            SelectObject(hdc, old_br);
            SelectObject(hdc, old_pen);
            DeleteObject(pen as _);

            EndPaint(hwnd, &ps);
            0
        }
        WM_LBUTTONDOWN => {
            let cx = (lp & 0xFFFF) as u16 as i32;
            let cy = ((lp >> 16) & 0xFFFF) as u16 as i32;
            if cy >= TRACK_Y - 14 && cy <= TRACK_Y + TRACK_H + 14 {
                DRAG.with(|c| c.set(true));
                SetCapture(hwnd);
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let (tx, tx2) = track_rect(rc.right);
                let p = (cx - tx) * 100 / (tx2 - tx).max(1);
                set_pct(hwnd, p);
            }
            0
        }
        WM_MOUSEMOVE => {
            if DRAG.with(|c| c.get()) {
                let cx = (lp & 0xFFFF) as u16 as i32;
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let (tx, tx2) = track_rect(rc.right);
                let p = (cx - tx) * 100 / (tx2 - tx).max(1);
                set_pct(hwnd, p);
            }
            0
        }
        WM_LBUTTONUP => {
            if DRAG.with(|c| c.get()) {
                DRAG.with(|c| c.set(false));
                ReleaseCapture();
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            // ขนาดเปลี่ยน → สั่งวาดใหม่ทั้งใบ (กันรอยเส้นค้างตอนลาก/ขยาย)
            InvalidateRect(hwnd, null(), 1);
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            let f = FONT.with(|c| c.get());
            if f != 0 {
                DeleteObject(f as _);
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

        let cls = w("ScreenMaskWnd");
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
            return;
        }

        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED,
            cls.as_ptr(),
            w("screen mask").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
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
            return;
        }

        // title bar โหมดดำ
        let on: i32 = 1;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &on as *const i32 as *const std::ffi::c_void,
            4,
        );

        let face = w("Tahoma");
        let font = CreateFontW(-20, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
        FONT.with(|c| c.set(font as isize));

        SetLayeredWindowAttributes(hwnd, 0, alpha_of(60), LWA_ALPHA);
        ShowWindow(hwnd, SW_SHOW);
        InvalidateRect(hwnd, null(), 1);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
