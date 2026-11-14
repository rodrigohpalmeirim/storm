//! Frosted-glass backdrop for modal dialogs.
//!
//! gpui has no backdrop filter, so when a dialog opens we grab the window's
//! last presented frame, blur it on the CPU (at low resolution, which is both
//! cheap and part of the blur), and draw it over the app beneath the dialog
//! layer. Under a modal the app can't be interacted with, so a still image is
//! indistinguishable from a live blur in practice.

use std::sync::Arc;

use gpui::{RenderImage, Window};

/// Capture is downscaled by this factor before blurring.
const DOWNSCALE: usize = 6;
/// Box-blur radius (in downscaled pixels) and passes; 3 passes ≈ gaussian.
const RADIUS: usize = 3;
const PASSES: usize = 3;

/// Captures and blurs the window's current contents. `None` if unsupported or capture failed.
pub fn capture(window: &Window) -> Option<Arc<RenderImage>> {
    let (w, h, bgra) = platform::grab(window)?;
    let (sw, sh, mut small) = downscale(&bgra, w, h, DOWNSCALE);
    for _ in 0..PASSES {
        box_blur(&mut small, sw, sh, RADIUS);
    }
    let buffer = image::RgbaImage::from_raw(sw as u32, sh as u32, small)?;
    Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
}

/// Averages `f`×`f` blocks. Pixels are 4 bytes; alpha is forced opaque.
fn downscale(src: &[u8], w: usize, h: usize, f: usize) -> (usize, usize, Vec<u8>) {
    let (sw, sh) = ((w / f).max(1), (h / f).max(1));
    let mut out = vec![0u8; sw * sh * 4];
    for sy in 0..sh {
        for sx in 0..sw {
            let mut acc = [0u32; 3];
            let mut n = 0u32;
            for y in sy * f..((sy + 1) * f).min(h) {
                let row = y * w * 4;
                for x in sx * f..((sx + 1) * f).min(w) {
                    let p = row + x * 4;
                    acc[0] += src[p] as u32;
                    acc[1] += src[p + 1] as u32;
                    acc[2] += src[p + 2] as u32;
                    n += 1;
                }
            }
            let o = (sy * sw + sx) * 4;
            let n = n.max(1);
            out[o] = (acc[0] / n) as u8;
            out[o + 1] = (acc[1] / n) as u8;
            out[o + 2] = (acc[2] / n) as u8;
            out[o + 3] = 255;
        }
    }
    (sw, sh, out)
}

/// Separable box blur with clamped edges, in place.
fn box_blur(px: &mut [u8], w: usize, h: usize, r: usize) {
    let mut tmp = vec![0u8; px.len()];
    blur_pass(px, &mut tmp, w, h, r, true);
    blur_pass(&tmp, px, w, h, r, false);
}

fn blur_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let idx = |line: usize, i: usize| if horizontal { (line * w + i) * 4 } else { (i * w + line) * 4 };
    let win = (2 * r + 1) as u32;
    for line in 0..lines {
        for c in 0..3 {
            let at = |i: isize| src[idx(line, i.clamp(0, len as isize - 1) as usize) + c] as u32;
            let mut sum: u32 = (-(r as isize)..=r as isize).map(at).sum();
            for i in 0..len {
                dst[idx(line, i) + c] = (sum / win) as u8;
                sum += at(i as isize + r as isize + 1);
                sum -= at(i as isize - r as isize);
            }
        }
        for i in 0..len {
            dst[idx(line, i) + 3] = 255;
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use gpui::Window;
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

    /// Copies the window's client area from the screen (what DWM last composed),
    /// as top-down BGRA. Reading the screen DC sends no messages to our window,
    /// so it's safe to call mid-render.
    pub fn grab(window: &Window) -> Option<(usize, usize, Vec<u8>)> {
        let RawWindowHandle::Win32(handle) = raw_window_handle::HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        let hwnd = handle.hwnd.get() as HWND;
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            if GetClientRect(hwnd, &mut rc) == 0 {
                return None;
            }
            let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
            if w <= 0 || h <= 0 {
                return None;
            }
            let mut origin = POINT { x: 0, y: 0 };
            ClientToScreen(hwnd, &mut origin);

            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return None;
            }
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp);
            let copied = BitBlt(mem, 0, 0, w, h, screen, origin.x, origin.y, SRCCOPY) != 0;

            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut buf = vec![0u8; w as usize * h as usize * 4];
            SelectObject(mem, old);
            let lines = if copied {
                GetDIBits(mem, bmp, 0, h as u32, buf.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS)
            } else {
                0
            };
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
            (lines == h).then_some((w as usize, h as usize, buf))
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    pub fn grab(_: &gpui::Window) -> Option<(usize, usize, Vec<u8>)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_averages_blocks() {
        // 2x2 image: two black, two white pixels -> one mid-grey pixel.
        let src = [0, 0, 0, 0, 255, 255, 255, 0, 255, 255, 255, 0, 0, 0, 0, 0];
        let (w, h, out) = downscale(&src, 2, 2, 2);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![127, 127, 127, 255]);
    }

    #[test]
    fn blur_preserves_flat_colour_and_spreads_edges() {
        let (w, h) = (8, 4);
        let mut flat = [40u8, 80, 120, 255].repeat(w * h);
        box_blur(&mut flat, w, h, 2);
        assert!(flat.chunks(4).all(|p| p == [40, 80, 120, 255]));

        // Left half black, right half white: the edge gets intermediate values.
        let mut img: Vec<u8> =
            (0..w * h).flat_map(|i| if i % w < w / 2 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
        box_blur(&mut img, w, h, 1);
        let edge = img[(w / 2 - 1) * 4];
        assert!(edge > 0 && edge < 255, "{edge}");
    }
}
