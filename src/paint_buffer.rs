use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, SelectObject,
    HBITMAP, HDC, HGDIOBJ, SRCCOPY,
};

/// A reusable offscreen frame. The window's paint DC retains its own update and
/// child-control clipping when the completed frame is copied to it.
#[derive(Default)]
pub(crate) struct PaintBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    original_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl PaintBuffer {
    /// The caller may draw into this DC, but must leave its bitmap selected.
    pub(crate) unsafe fn target(
        &mut self,
        compatible_dc: HDC,
        width: i32,
        height: i32,
    ) -> Option<HDC> {
        if compatible_dc.is_null() || width <= 0 || height <= 0 {
            return None;
        }
        if !self.dc.is_null() && self.width == width && self.height == height {
            return Some(self.dc);
        }
        // Cached dimensions were already validated; check only a new allocation.
        width.checked_mul(height)?.checked_mul(4)?;
        let created_dc = self.dc.is_null();
        let dc = if created_dc {
            CreateCompatibleDC(compatible_dc)
        } else {
            self.dc
        };
        if dc.is_null() {
            return None;
        }
        // Use the destination DC, because a new memory DC is monochrome.
        let bitmap = CreateCompatibleBitmap(compatible_dc, width, height);
        let previous = if bitmap.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(dc, bitmap)
        };
        if previous.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if created_dc {
                DeleteDC(dc);
            }
            return None;
        }
        if created_dc {
            self.original_bitmap = previous;
        } else {
            // The previous bitmap is no longer selected into the memory DC.
            DeleteObject(self.bitmap);
        }
        self.dc = dc;
        self.bitmap = bitmap;
        self.width = width;
        self.height = height;
        Some(dc)
    }

    pub(crate) unsafe fn present(&self, destination: HDC) -> bool {
        !destination.is_null()
            && !self.dc.is_null()
            && BitBlt(
                destination,
                0,
                0,
                self.width,
                self.height,
                self.dc,
                0,
                0,
                SRCCOPY,
            ) != 0
    }

    /// Used after a failed copy so the next paint starts with the current device.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

impl Drop for PaintBuffer {
    fn drop(&mut self) {
        unsafe {
            if !self.dc.is_null() {
                SelectObject(self.dc, self.original_bitmap);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{mem::size_of, ptr::null_mut, slice};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateDIBSection, ExcludeClipRect, GdiFlush, GetObjectType, GetStockObject,
        IntersectClipRect, PatBlt, SetDCBrushColor, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DC_BRUSH, DIB_RGB_COLORS, OBJ_BITMAP, OBJ_MEMDC, PATCOPY,
    };
    use windows_sys::Win32::Graphics::Gdi::{GetCurrentObject, GetObjectW};

    const SENTINEL: u32 = 0x0010_2030;
    const WHITE: u32 = 0x00ff_ffff;
    const GRAY: u32 = 0x005a_5a5a;

    // A DIB belonging entirely to this test, with no screen input or capture.
    struct Canvas {
        dc: HDC,
        bitmap: HBITMAP,
        original_bitmap: HGDIOBJ,
        pixels: *mut u32,
        width: i32,
        height: i32,
    }

    impl Canvas {
        unsafe fn new(width: i32, height: i32) -> Self {
            let dc = CreateCompatibleDC(null_mut());
            assert!(!dc.is_null());
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut pixels = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
            assert!(!bitmap.is_null() && !pixels.is_null());
            let original_bitmap = SelectObject(dc, bitmap);
            assert!(!original_bitmap.is_null());
            let mut canvas = Self {
                dc,
                bitmap,
                original_bitmap,
                pixels: pixels.cast(),
                width,
                height,
            };
            canvas.pixels_mut().fill(SENTINEL);
            canvas
        }

        unsafe fn pixels_mut(&mut self) -> &mut [u32] {
            GdiFlush();
            slice::from_raw_parts_mut(self.pixels, (self.width * self.height) as usize)
        }

        unsafe fn pixels(&self) -> &[u32] {
            GdiFlush();
            slice::from_raw_parts(self.pixels, (self.width * self.height) as usize)
        }
    }

    impl Drop for Canvas {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.original_bitmap);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }

    unsafe fn fill(dc: HDC, color: u32, x: i32, y: i32, width: i32, height: i32) {
        let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        let previous_color = SetDCBrushColor(dc, color);
        assert_ne!(PatBlt(dc, x, y, width, height, PATCOPY), 0);
        SetDCBrushColor(dc, previous_color);
        SelectObject(dc, brush);
    }

    // GetObjectType can retain the encoded type of a deleted handle. Query the
    // actual object instead to prove that it no longer owns a bitmap allocation.
    unsafe fn assert_bitmap_released(bitmap: HBITMAP) {
        let mut object: BITMAP = std::mem::zeroed();
        assert_eq!(
            GetObjectW(
                bitmap,
                size_of::<BITMAP>() as i32,
                (&mut object as *mut BITMAP).cast()
            ),
            0
        );
    }

    #[test]
    fn drawing_is_hidden_until_the_complete_frame_is_presented() {
        unsafe {
            let canvas = Canvas::new(32, 24);
            let mut buffer = PaintBuffer::default();
            let dc = buffer.target(canvas.dc, 32, 24).unwrap();
            fill(dc, WHITE, 0, 0, 32, 24);
            fill(dc, GRAY, 0, 12, 32, 12);
            assert!(canvas.pixels().iter().all(|pixel| *pixel == SENTINEL));
            assert!(buffer.present(canvas.dc));
            for (index, pixel) in canvas.pixels().iter().enumerate() {
                assert_eq!(*pixel, if index / 32 < 12 { WHITE } else { GRAY });
            }
        }
    }

    #[test]
    fn presentation_preserves_the_update_clip_and_child_control_hole() {
        unsafe {
            let canvas = Canvas::new(30, 24);
            let mut buffer = PaintBuffer::default();
            let dc = buffer.target(canvas.dc, 30, 24).unwrap();
            fill(dc, WHITE, 0, 0, 30, 24);
            assert_ne!(IntersectClipRect(canvas.dc, 3, 2, 27, 22), 0);
            assert_ne!(ExcludeClipRect(canvas.dc, 9, 6, 20, 13), 0);
            assert!(buffer.present(canvas.dc));
            for (index, pixel) in canvas.pixels().iter().enumerate() {
                let x = (index % 30) as i32;
                let y = (index / 30) as i32;
                let visible = (3..27).contains(&x)
                    && (2..22).contains(&y)
                    && !((9..20).contains(&x) && (6..13).contains(&y));
                assert_eq!(*pixel, if visible { WHITE } else { SENTINEL });
            }
        }
    }

    #[test]
    fn same_size_reuses_resources_and_resize_reset_drop_release_owned_handles() {
        unsafe {
            let canvas = Canvas::new(30, 24);
            let mut buffer = PaintBuffer::default();
            let dc = buffer.target(canvas.dc, 12, 8).unwrap();
            let bitmap = buffer.bitmap;
            assert_eq!(buffer.target(canvas.dc, 12, 8), Some(dc));
            assert_eq!(buffer.bitmap, bitmap);
            assert_eq!(GetObjectType(dc), OBJ_MEMDC as u32);
            assert_eq!(GetObjectType(bitmap), OBJ_BITMAP as u32);
            assert_eq!(buffer.target(canvas.dc, 20, 10), Some(dc));
            assert_ne!(buffer.bitmap, bitmap);
            assert_ne!(GdiFlush(), 0);
            assert_bitmap_released(bitmap);
            assert_eq!((buffer.width, buffer.height), (20, 10));
            let resized_bitmap = buffer.bitmap;
            buffer.reset();
            assert!(buffer.dc.is_null() && buffer.bitmap.is_null());
            assert_eq!((buffer.width, buffer.height), (0, 0));
            assert_ne!(GdiFlush(), 0);
            assert!(GetCurrentObject(dc, OBJ_BITMAP as u32).is_null());
            assert_bitmap_released(resized_bitmap);
            let dc = buffer.target(canvas.dc, 16, 12).unwrap();
            let bitmap = buffer.bitmap;
            drop(buffer);
            assert_ne!(GdiFlush(), 0);
            assert!(GetCurrentObject(dc, OBJ_BITMAP as u32).is_null());
            assert_bitmap_released(bitmap);
        }
    }

    #[test]
    fn invalid_targets_do_not_allocate_or_replace_an_existing_frame() {
        unsafe {
            let canvas = Canvas::new(16, 12);
            let mut buffer = PaintBuffer::default();
            assert!(buffer.target(null_mut(), 16, 12).is_none());
            assert!(buffer.target(canvas.dc, 0, 12).is_none());
            assert!(buffer.target(canvas.dc, 16, -1).is_none());
            assert!(buffer.target(canvas.dc, i32::MAX, i32::MAX).is_none());
            assert!(buffer.dc.is_null() && buffer.bitmap.is_null());
            assert!(!buffer.present(canvas.dc));
            let dc = buffer.target(canvas.dc, 16, 12).unwrap();
            let bitmap = buffer.bitmap;
            fill(dc, WHITE, 0, 0, 16, 12);
            assert!(buffer.target(null_mut(), 16, 12).is_none());
            assert!(buffer.target(canvas.dc, -1, 12).is_none());
            assert!(buffer.target(canvas.dc, 16, 0).is_none());
            assert!(buffer.target(canvas.dc, i32::MAX, 12).is_none());
            assert_eq!(buffer.dc, dc);
            assert_eq!(buffer.bitmap, bitmap);
            assert!(!buffer.present(null_mut()));
            assert!(buffer.present(canvas.dc));
            assert!(canvas.pixels().iter().all(|pixel| *pixel == WHITE));
        }
    }
}
