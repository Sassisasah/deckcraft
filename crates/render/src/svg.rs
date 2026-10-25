//! SVG pictures (Office icons, `asvg:svgBlip`), drawn with resvg at the size they are shown.

/// Does `bytes` look like an SVG document (optionally after a BOM, XML declaration or comments)?
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<?xml") || t.starts_with("<svg") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE svg")) && text.contains("<svg")
}

/// The SVG's own size in pixels (96 dpi): its `width`/`height`, else its `viewBox`. A tiled
/// picture fill lays its tiles out at this size.
pub fn natural_size(bytes: &[u8]) -> Option<(f64, f64)> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let (w, h) = (size.width() as f64, size.height() as f64);
    (w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()).then_some((w, h))
}

/// Draw the SVG with its longer side `side` px, to straight RGBA.
pub fn rasterize(bytes: &[u8], side: u32) -> Option<image::RgbaImage> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let (w, h) = (size.width() as f64, size.height() as f64);
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let k = side.clamp(16, 8192) as f64 / w.max(h);
    let (pw, ph) = (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1));
    let mut pm = resvg::tiny_skia::Pixmap::new(pw, ph)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k as f32, k as f32), &mut pm.as_mut());
    // tiny-skia is premultiplied; the picture cache wants straight RGBA.
    let mut data = pm.take();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(pw, ph, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_size_is_the_documents_width_and_height() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1281.21" height="721.119" viewBox="0 0 32489931 18286739"><line x1="0" y1="0" x2="32489931" y2="18286739" stroke="#D5001C" stroke-width="25360.5"/></svg>"##;
        let (w, h) = super::natural_size(svg).unwrap();
        assert!((w - 1281.21).abs() < 0.01 && (h - 721.119).abs() < 0.01, "{w}×{h}");
    }

    #[test]
    fn office_icon_draws() {
        let svg = br##"<svg viewBox="0 0 96 96" xmlns="http://www.w3.org/2000/svg"><circle cx="48" cy="48" r="40" fill="#FF0000"/></svg>"##;
        assert!(is_svg(svg));
        let img = rasterize(svg, 200).unwrap();
        assert_eq!(img.dimensions(), (200, 200));
        assert_eq!(img.get_pixel(100, 100).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(2, 2).0[3], 0);
        assert!(!is_svg(b"\x89PNG\r\n"));
    }
}
