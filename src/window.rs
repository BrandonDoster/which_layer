//! Windows overlay window. winit owns the window and event loop; pixels go out
//! through UpdateLayeredWindow because softbuffer's GDI path drops per-pixel
//! alpha on Windows, and a layered window is the native way to get a
//! transparent, click-through, never-focused overlay.

use tiny_skia::Pixmap;
use windows_sys::Win32::Foundation::{HWND, POINT, SIZE};
use windows_sys::Win32::Graphics::Dwm::DwmFlush;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::platform::windows::WindowAttributesExtWindows;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes, WindowLevel};

pub struct Overlay {
    _window: Window,
    hwnd: HWND,
    dib: Option<Dib>,
}

impl Overlay {
    /// Centred horizontally on the primary monitor, `margin` physical pixels
    /// from its top or bottom edge. Anchor + margin rather than absolute x/y,
    /// because that's all Wayland layer-shell offers.
    pub fn create(el: &ActiveEventLoop, w: u32, h: u32, top: bool, margin: i32) -> Overlay {
        let pos = el
            .primary_monitor()
            .map(|m| {
                let (s, p) = (m.size(), m.position());
                let y = if top { p.y + margin } else { p.y + s.height as i32 - h as i32 - margin };
                PhysicalPosition::new(p.x + (s.width as i32 - w as i32) / 2, y)
            })
            .unwrap_or_default();
        let attrs = WindowAttributes::default()
            .with_title("which_layer")
            .with_visible(false)
            .with_decorations(false)
            .with_resizable(false)
            .with_active(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_skip_taskbar(true)
            .with_inner_size(PhysicalSize::new(w, h))
            .with_position(pos);
        let window = el.create_window(attrs).expect("create overlay window");
        let RawWindowHandle::Win32(handle) = window.window_handle().expect("window handle").as_raw() else {
            unreachable!("winit on Windows always gives a Win32 handle")
        };
        let hwnd = handle.hwnd.get() as HWND;
        unsafe {
            // LAYERED + TRANSPARENT = per-pixel alpha and click-through.
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let add = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | add as isize);
        }
        Overlay { _window: window, hwnd, dib: None }
    }

    /// `opacity` (0..=1) scales the whole window on top of per-pixel alpha.
    /// If GDI can't allocate the surface (out of memory or handles) the frame is
    /// skipped; the window keeps its previous contents.
    pub fn present(&mut self, pm: &Pixmap, opacity: f32) {
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        if self.dib.as_ref().is_none_or(|d| d.w != w || d.h != h) {
            self.dib = None; // free the old surface before allocating the new one
            self.dib = Dib::new(w, h);
        }
        let Some(dib) = &self.dib else { return };
        // The DIB is exactly w*h 32-bit pixels, the same byte length as the pixmap.
        debug_assert_eq!(pm.data().len(), w as usize * h as usize * 4);
        let dst = unsafe { std::slice::from_raw_parts_mut(dib.bits, pm.data().len()) };
        // tiny-skia is premultiplied RGBA; GDI wants premultiplied BGRA.
        for (d, s) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
            d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
        }
        self.set_opacity(opacity);
    }

    /// Re-composites the last presented pixels at a new opacity (fade frames).
    pub fn set_opacity(&self, opacity: f32) {
        let Some(dib) = &self.dib else { return };
        let size = SIZE { cx: dib.w, cy: dib.h };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
                dib.dc,
                &src,
                0,
                &blend,
                ULW_ALPHA,
            );
        }
    }

    pub fn show(&self) {
        unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
    }

    pub fn hide(&self) {
        unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }
}

/// Blocks until DWM has composed the next frame, i.e. the change is on screen.
pub fn wait_for_compositor() {
    unsafe { DwmFlush() };
}

/// Top-down 32bpp DIB selected into a memory DC, the source for UpdateLayeredWindow.
struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    w: i32,
    h: i32,
}

impl Dib {
    fn new(w: i32, h: i32) -> Option<Dib> {
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                if !bmp.is_null() {
                    DeleteObject(bmp);
                }
                DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, bmp);
            Some(Dib { dc, bmp, old, bits: bits as *mut u8, w, h })
        }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
    }
}
