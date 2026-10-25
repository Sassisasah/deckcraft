//! Media settings PowerPoint keeps in the slide's timing tree (`p:video`/`p:audio` →
//! `p:cMediaNode`): volume, mute, Loop until Stopped, Hide While Not Playing and Play Full Screen.

use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use deckcraft_geom::Xfrm;
use deckcraft_model::{LayoutType, MediaClip, Presentation, Shape, ShapeId, ShapeKind, defaults};

fn video_pptx() -> (Vec<u8>, u32) {
    let mut p = defaults::new_presentation(None);
    p.slides.clear();
    let media = p.add_media("clip.mp4", "video/mp4", b"not really a video".to_vec()).expect("media");
    let layout = defaults::layout_of_kind(&p, LayoutType::Blank).unwrap_or_default();
    let mut slide = defaults::new_slide(&mut p, layout).expect("slide");
    let id = p.alloc_id().expect("id");
    let mut film = Shape { id: ShapeId(id), name: "Film".into(), xfrm: Some(Xfrm::new(40.0, 40.0, 320.0, 180.0)), ..Default::default() };
    film.kind = ShapeKind::Media(MediaClip { media, video: true, volume: 1.0, ..Default::default() });
    slide.shapes.push(film);
    p.slides.push(Arc::new(slide));
    (deckcraft_pptx::export(&p).expect("export"), id)
}

/// Replace the slide's timing tree with one holding `media` (a `p:video` or `p:audio` element).
fn with_timing(src: &[u8], media: &str) -> Vec<u8> {
    let timing = format!(
        r#"<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst>{media}</p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>"#
    );
    let mut zip = zip::ZipArchive::new(Cursor::new(src)).expect("zip");
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).expect("entry");
        let name = f.name().to_string();
        let mut data = vec![];
        f.read_to_end(&mut data).expect("read");
        if name == "ppt/slides/slide1.xml" {
            let mut text = String::from_utf8(data).expect("utf8");
            if let (Some(a), Some(b)) = (text.find("<p:timing"), text.find("</p:timing>")) {
                text.replace_range(a..b + "</p:timing>".len(), "");
            }
            let at = text.find("<p:extLst").filter(|&i| i > text.find("</p:cSld>").unwrap_or(0)).or_else(|| text.rfind("</p:sld>")).expect("end");
            text.insert_str(at, &timing);
            data = text.into_bytes();
        }
        out.start_file(&name, opts).expect("start");
        out.write_all(&data).expect("write");
    }
    out.finish().expect("finish").into_inner()
}

fn clip(p: &Presentation) -> MediaClip {
    let s = p.slides[0].shapes.iter().find(|s| s.name == "Film").expect("film");
    let ShapeKind::Media(m) = &s.kind else { panic!("not media") };
    m.clone()
}

#[test]
fn volume_loop_hide_and_full_screen_come_from_the_media_node() {
    let (src, id) = video_pptx();
    let node = |attrs: &str, cnt: &str, video: &str| {
        format!(
            r#"<p:video{video}><p:cMediaNode {attrs}><p:cTn id="2" fill="hold" display="0"{cnt}><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="{id}"/></p:tgtEl></p:cMediaNode></p:video>"#
        )
    };
    let p =
        deckcraft_pptx::import(&with_timing(&src, &node(r#"vol="40000" showWhenStopped="0""#, r#" repeatCount="indefinite""#, r#" fullScrn="1""#)))
            .expect("import");
    let c = clip(&p);
    assert!((c.volume - 0.4).abs() < 1e-9, "{}", c.volume);
    assert!(c.loop_play && c.hide_while_not_playing && c.full_screen);

    let p = deckcraft_pptx::import(&with_timing(&src, &node(r#"vol="80000" mute="1""#, "", ""))).expect("import");
    let c = clip(&p);
    assert_eq!(c.volume, 0.0, "muted");
    assert!(!c.loop_play && !c.hide_while_not_playing && !c.full_screen);
}
