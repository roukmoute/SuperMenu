//! Small native launcher artwork, generated at the exact display size.

use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CopyIcon, CreateIconIndirect, DestroyIcon, LoadIconW, HICON, ICONINFO, IDI_APPLICATION,
};

const VIOLET: u32 = 0xFF8A_62EC;

/// Only private icon handles enter this owner, including the fallback copy.
pub struct Icon(HICON);

impl Icon {
    pub fn new(size: u32) -> Self {
        let icon = generated_icon(size.clamp(16, 64)).unwrap_or_else(|| unsafe {
            // LoadIconW returns a shared handle. Copy it before owning it; a
            // failed copy stays null rather than taking ownership of the shared icon.
            CopyIcon(LoadIconW(null_mut(), IDI_APPLICATION))
        });
        Self(icon)
    }

    pub fn raw(&self) -> HICON {
        self.0
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DestroyIcon(self.0) };
        }
    }
}

struct Bitmap(HBITMAP);

impl Drop for Bitmap {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DeleteObject(self.0) };
        }
    }
}

struct Dib {
    bitmap: Bitmap,
    pixels: *mut u32,
}

fn dib(size: u32) -> Option<Dib> {
    unsafe {
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size as i32;
        info.bmiHeader.biHeight = -(size as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = null_mut();
        let bitmap = Bitmap(CreateDIBSection(
            null_mut(),
            &info,
            DIB_RGB_COLORS,
            &mut pixels,
            null_mut(),
            0,
        ));
        if bitmap.0.is_null() || pixels.is_null() {
            None
        } else {
            Some(Dib {
                bitmap,
                pixels: pixels.cast(),
            })
        }
    }
}

fn generated_icon(size: u32) -> Option<HICON> {
    let color = dib(size)?;
    let side = size as usize;
    let margin = (side / 16).max(1);
    let gap = margin;
    let available = side - 2 * margin - 2 * gap;
    let base = available / 3;
    let remainder = available % 3;
    // Any extra pixel sits in the middle, or one at each edge. The left and
    // right columns always mirror each other at intermediate DPI sizes.
    let widths = [
        base + usize::from(remainder == 2),
        base + usize::from(remainder == 1),
        base + usize::from(remainder == 2),
    ];
    let starts = [
        margin,
        margin + widths[0] + gap,
        margin + widths[0] + gap + widths[1] + gap,
    ];
    // CreateBitmap's 1-bit rows are WORD-aligned. Transparent pixels have an
    // AND bit of one, so older mask-based rendering preserves its background too.
    let stride = side.div_ceil(16) * 2;
    let mut mask_pixels = vec![0xFFu8; stride * side];
    unsafe {
        let pixels = std::slice::from_raw_parts_mut(color.pixels, side * side);
        pixels.fill(0);
        for row in 0..3 {
            for column in 0..3 {
                for y in starts[row]..starts[row] + widths[row] {
                    for x in starts[column]..starts[column] + widths[column] {
                        pixels[y * side + x] = VIOLET;
                        mask_pixels[y * stride + x / 8] &= !(0x80 >> (x % 8));
                    }
                }
            }
        }
        let mask = Bitmap(CreateBitmap(
            size as i32,
            size as i32,
            1,
            1,
            mask_pixels.as_ptr().cast(),
        ));
        if mask.0.is_null() {
            return None;
        }
        // CreateIconIndirect copies both bitmaps. Their guards release them
        // immediately; only the new private HICON survives this function.
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask.0,
            hbmColor: color.bitmap.0,
        });
        (!icon.is_null()).then_some(icon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, GdiFlush, GetObjectW, SelectObject, BITMAP, HDC, HGDIOBJ,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DrawIconEx, GetIconInfo, DI_MASK, DI_NORMAL,
    };

    struct Canvas {
        dc: HDC,
        previous: HGDIOBJ,
        image: Dib,
    }

    impl Canvas {
        fn new(size: u32) -> Self {
            let image = dib(size).expect("test DIB");
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                assert!(!dc.is_null());
                let previous = SelectObject(dc, image.bitmap.0);
                assert!(!previous.is_null());
                Self {
                    dc,
                    previous,
                    image,
                }
            }
        }
    }

    impl Drop for Canvas {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.previous);
                DeleteDC(self.dc);
            }
        }
    }

    fn export_bmp(size: u32, theme: &str, pixels: &[u32]) {
        let Some(directory) =
            std::env::var_os("SUPER_MENU_ICON_TEST_DIR").filter(|value| !value.is_empty())
        else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("icon QA directory");
        let mut bytes = Vec::with_capacity(54 + pixels.len() * 4);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&((54 + pixels.len() * 4) as u32).to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&(size as i32).to_le_bytes());
        bytes.extend_from_slice(&(-(size as i32)).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        for pixel in pixels {
            bytes.extend_from_slice(&pixel.to_le_bytes());
        }
        std::fs::write(
            directory.join(format!("brand-icon-{size}-{theme}.bmp")),
            bytes,
        )
        .expect("icon QA BMP");
    }

    #[test]
    fn native_brand_icon_is_transparent_centered_and_owns_all_its_handles() {
        for (size, ranges) in [
            (16u32, [(1, 4), (6, 9), (11, 14)]),
            (20, [(1, 5), (7, 12), (14, 18)]),
            (24, [(1, 7), (9, 14), (16, 22)]),
            (32, [(2, 9), (12, 19), (22, 29)]),
        ] {
            let icon = Icon::new(size);
            let raw = icon.raw();
            assert!(!raw.is_null());
            unsafe {
                let mut info: ICONINFO = zeroed();
                assert_ne!(GetIconInfo(raw, &mut info), 0);
                let color = Bitmap(info.hbmColor);
                let mask = Bitmap(info.hbmMask);
                assert!(!color.0.is_null() && !mask.0.is_null());
                for bitmap in [&color, &mask] {
                    let mut details: BITMAP = zeroed();
                    assert_ne!(
                        GetObjectW(
                            bitmap.0,
                            size_of::<BITMAP>() as i32,
                            (&mut details as *mut BITMAP).cast()
                        ),
                        0
                    );
                    assert_eq!(details.bmWidth, size as i32);
                    assert_eq!(details.bmHeight, size as i32);
                    assert_eq!(
                        details.bmBitsPixel,
                        if bitmap.0 == color.0 { 32 } else { 1 }
                    );
                }
                let color_raw = color.0;
                let mask_raw = mask.0;
                drop(color);
                drop(mask);
                for released in [color_raw, mask_raw] {
                    let mut details: BITMAP = zeroed();
                    assert_eq!(
                        GetObjectW(
                            released,
                            size_of::<BITMAP>() as i32,
                            (&mut details as *mut BITMAP).cast()
                        ),
                        0
                    );
                }
            }
            let canvas = Canvas::new(size);
            let side = size as usize;
            let occupied = |coordinate| {
                ranges
                    .iter()
                    .any(|&(start, end)| (start..=end).contains(&coordinate))
            };
            for (theme, background) in [("light", 0xFFF5_F5F5u32), ("dark", 0xFF10_1014)] {
                unsafe {
                    std::slice::from_raw_parts_mut(canvas.image.pixels, side * side)
                        .fill(background)
                };
                unsafe {
                    assert_ne!(
                        DrawIconEx(
                            canvas.dc,
                            0,
                            0,
                            raw,
                            size as i32,
                            size as i32,
                            0,
                            null_mut(),
                            DI_NORMAL
                        ),
                        0
                    );
                    assert_ne!(GdiFlush(), 0);
                }
                let pixels =
                    unsafe { std::slice::from_raw_parts(canvas.image.pixels, side * side) };
                for y in 0..side {
                    for x in 0..side {
                        let pixel = pixels[y * side + x];
                        if occupied(x) && occupied(y) {
                            assert_eq!(
                                pixel & 0x00FF_FFFF,
                                VIOLET & 0x00FF_FFFF,
                                "{size}px {theme} pixel {x},{y}"
                            );
                        } else {
                            assert_eq!(
                                pixel, background,
                                "{size}px {theme} transparent pixel {x},{y}"
                            );
                        }
                        assert_eq!(
                            pixel & 0x00FF_FFFF,
                            pixels[y * side + side - x - 1] & 0x00FF_FFFF
                        );
                        assert_eq!(
                            pixel & 0x00FF_FFFF,
                            pixels[(side - y - 1) * side + x] & 0x00FF_FFFF
                        );
                    }
                }
                export_bmp(size, theme, pixels);
            }
            // The legacy AND mask independently agrees with alpha transparency.
            unsafe {
                assert_ne!(
                    DrawIconEx(
                        canvas.dc,
                        0,
                        0,
                        raw,
                        size as i32,
                        size as i32,
                        0,
                        null_mut(),
                        DI_MASK
                    ),
                    0
                );
                assert_ne!(GdiFlush(), 0);
                let pixels = std::slice::from_raw_parts(canvas.image.pixels, side * side);
                for y in 0..side {
                    for x in 0..side {
                        assert_eq!(
                            pixels[y * side + x] & 0x00FF_FFFF,
                            if occupied(x) && occupied(y) {
                                0
                            } else {
                                0x00FF_FFFF
                            }
                        );
                    }
                }
            }
            drop(canvas);
            drop(icon);
            unsafe {
                let mut info: ICONINFO = zeroed();
                assert_eq!(GetIconInfo(raw, &mut info), 0, "private icon released");
            }
        }
        for (requested, expected) in [(0, 16), (u32::MAX, 64)] {
            let icon = Icon::new(requested);
            unsafe {
                let mut info: ICONINFO = zeroed();
                assert_ne!(GetIconInfo(icon.raw(), &mut info), 0);
                let color = Bitmap(info.hbmColor);
                let _mask = Bitmap(info.hbmMask);
                let mut details: BITMAP = zeroed();
                assert_ne!(
                    GetObjectW(
                        color.0,
                        size_of::<BITMAP>() as i32,
                        (&mut details as *mut BITMAP).cast()
                    ),
                    0
                );
                assert_eq!(details.bmWidth, expected);
                assert_eq!(details.bmHeight, expected);
            }
        }
        unsafe {
            let shared = LoadIconW(null_mut(), IDI_APPLICATION);
            assert!(!shared.is_null());
            let copied = Icon(CopyIcon(shared));
            assert!(!copied.raw().is_null());
            assert_ne!(copied.raw(), shared);
            drop(copied);
            let mut info: ICONINFO = zeroed();
            assert_ne!(
                GetIconInfo(shared, &mut info),
                0,
                "shared fallback remains available"
            );
            let _color = Bitmap(info.hbmColor);
            let _mask = Bitmap(info.hbmMask);
        }
    }
}
