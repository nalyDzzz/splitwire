//! Executable icons as PNG data URLs, for the app picker and rule lists.

use std::collections::HashMap;
use std::sync::Mutex;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

/// Pixel size to extract; rendered at 32 CSS px, so this stays crisp at 2x.
const SIZE: u32 = 64;

#[derive(Default)]
pub struct IconCache(Mutex<HashMap<String, Option<String>>>);

impl IconCache {
    pub fn get_many(&self, paths: &[String]) -> HashMap<String, Option<String>> {
        let mut out = HashMap::with_capacity(paths.len());
        for path in paths {
            let key = path.to_lowercase();
            if let Some(hit) = self.0.lock().unwrap().get(&key) {
                out.insert(path.clone(), hit.clone());
                continue;
            }
            let icon = extract(path).map(|png| format!("data:image/png;base64,{}", STANDARD.encode(png)));
            self.0.lock().unwrap().insert(key, icon.clone());
            out.insert(path.clone(), icon);
        }
        out
    }
}

/// RGBA pixels to PNG bytes.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(rgba).ok()?;
    Some(out)
}

#[cfg(windows)]
fn extract(path: &str) -> Option<Vec<u8>> {
    use windows::core::HSTRING;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
    };
    use windows::Win32::UI::Shell::SHDefExtractIconW;
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

    unsafe {
        let mut icon = HICON::default();
        // nIconSize packs the large size in the low word and small in the high word.
        let hr = SHDefExtractIconW(&HSTRING::from(path), 0, 0, Some(&mut icon), None, SIZE);
        if hr.is_err() || icon.is_invalid() {
            return None;
        }

        let mut info = ICONINFO::default();
        let got_info = GetIconInfo(icon, &mut info).is_ok();
        let _ = DestroyIcon(icon);
        if !got_info {
            return None;
        }

        let read = |bitmap: HBITMAP| -> Option<(u32, u32, Vec<u8>)> {
            if bitmap.is_invalid() {
                return None;
            }
            let mut bm = BITMAP::default();
            if GetObjectW(HGDIOBJ(bitmap.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
                return None;
            }
            let (width, height) = (bm.bmWidth as u32, bm.bmHeight.unsigned_abs());
            let mut header = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32), // top-down rows
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; (width * height * 4) as usize];
            let dc = CreateCompatibleDC(None);
            let lines = GetDIBits(dc, bitmap, 0, height, Some(pixels.as_mut_ptr() as *mut _), &mut header, DIB_RGB_COLORS);
            let _ = DeleteDC(dc);
            (lines != 0).then_some((width, height, pixels))
        };

        let color = read(info.hbmColor);
        let mask = read(info.hbmMask);
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));

        let (width, height, mut bgra) = color?;
        // Old icons have no alpha channel; derive it from the AND mask
        // (black = opaque).
        if bgra.chunks_exact(4).all(|px| px[3] == 0) {
            let (_, _, mask) = mask?;
            for (px, m) in bgra.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                px[3] = if m[0] == 0 && m[1] == 0 && m[2] == 0 { 255 } else { 0 };
            }
        }
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2); // BGRA -> RGBA
        }
        encode_png(width, height, &bgra)
    }
}

#[cfg(not(windows))]
fn extract(_path: &str) -> Option<Vec<u8>> {
    let _ = encode_png;
    None
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn extracts_notepad_icon() {
        let notepad = std::path::Path::new(&std::env::var("SystemRoot").unwrap()).join(r"System32\notepad.exe");
        let png = super::extract(&notepad.to_string_lossy()).expect("notepad has an icon");
        assert_eq!(&png[1..4], b"PNG");
    }
}
