//! Office SVG pictures (icons): `asvg:svgBlip` in the blip's `extLst`, with or without a raster
//! fallback in `a:blip r:embed`. The picture is the SVG.

use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use deckcraft_geom::Xfrm;
use deckcraft_model::style::PictureFill;
use deckcraft_model::{LayoutType, Shape, ShapeId, ShapeKind, defaults};

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96"><circle cx="48" cy="48" r="40" fill="#1E90FF"/></svg>"##;

fn picture_pptx() -> Vec<u8> {
    let mut p = defaults::new_presentation(None);
    p.slides.clear();
    let img = image::RgbImage::from_pixel(4, 4, image::Rgb([200, 10, 10]));
    let mut png = Cursor::new(vec![]);
    image::DynamicImage::ImageRgb8(img).write_to(&mut png, image::ImageFormat::Png).expect("png");
    let media = p.add_media("image1.png", "image/png", png.into_inner()).expect("media");
    let layout = defaults::layout_of_kind(&p, LayoutType::Blank).unwrap_or_default();
    let mut slide = defaults::new_slide(&mut p, layout).expect("slide");
    let mut pic =
        Shape { id: ShapeId(p.alloc_id().expect("id")), name: "Icon".into(), xfrm: Some(Xfrm::new(40.0, 40.0, 96.0, 96.0)), ..Default::default() };
    pic.kind = ShapeKind::Picture { fill: PictureFill { media, ..Default::default() } };
    slide.shapes.push(pic);
    p.slides.push(Arc::new(slide));
    deckcraft_pptx::export(&p).expect("export")
}

/// Add the SVG part and its relationship, and give the slide's blip an `asvg:svgBlip`; with
/// `keep_raster` false the blip loses its raster `r:embed`, as in icons saved without one.
fn with_svg_blip(src: &[u8], keep_raster: bool) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(src)).expect("zip");
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).expect("entry");
        let name = f.name().to_string();
        let mut data = vec![];
        f.read_to_end(&mut data).expect("read");
        let text = String::from_utf8_lossy(&data).into_owned();
        let data = if name == "[Content_Types].xml" {
            text.replacen("<Default ", r#"<Default Extension="svg" ContentType="image/svg+xml"/><Default "#, 1).into_bytes()
        } else if name == "ppt/slides/_rels/slide1.xml.rels" {
            text.replace(
                "</Relationships>",
                r#"<Relationship Id="rIdSvg1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/icon1.svg"/></Relationships>"#,
            )
            .into_bytes()
        } else if name == "ppt/slides/slide1.xml" {
            let start = text.find("<a:blip ").expect("blip");
            let end = start + text[start..].find('>').expect("blip end");
            let self_closing = text[..end].ends_with('/');
            let open = if keep_raster { text[start..end].trim_end_matches('/').to_string() } else { "<a:blip".to_string() };
            let ext = r#"<a:extLst><a:ext uri="{96DAC541-7B7A-43D3-8B79-37D633B846F1}"><asvg:svgBlip xmlns:asvg="http://schemas.microsoft.com/office/drawing/2016/SVG/main" r:embed="rIdSvg1"/></a:ext></a:extLst>"#;
            let blip = format!("{open}>{ext}{}", if self_closing { "</a:blip>" } else { "" });
            format!("{}{}{}", &text[..start], blip, &text[end + 1..]).into_bytes()
        } else {
            data
        };
        out.start_file(&name, opts).expect("start");
        out.write_all(&data).expect("write");
    }
    out.start_file("ppt/media/icon1.svg", opts).expect("start");
    out.write_all(SVG).expect("write");
    out.finish().expect("finish").into_inner()
}

fn picture_bytes(bytes: &[u8]) -> Vec<u8> {
    let p = deckcraft_pptx::import(bytes).expect("import");
    let s = p.slides[0].shapes.iter().find(|s| s.name == "Icon").expect("icon");
    let ShapeKind::Picture { fill } = &s.kind else { panic!("not a picture: {:?}", s.kind) };
    p.media(fill.media).expect("media").data.to_vec()
}

#[test]
fn an_svg_blip_is_the_picture_with_or_without_a_raster_fallback() {
    let src = picture_pptx();
    assert_eq!(picture_bytes(&with_svg_blip(&src, true)), SVG, "the SVG wins over the PNG fallback");
    assert_eq!(picture_bytes(&with_svg_blip(&src, false)), SVG, "an SVG-only icon is not dropped");
}
