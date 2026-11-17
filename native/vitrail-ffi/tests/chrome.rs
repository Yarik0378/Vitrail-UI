//! Every page of tests/suite.txt that has a headless Chrome reference, painted by the CPU painter and compared with it
//! as tests/diff.py does (the worse of the error over black and over white, per channel, its mean over the picture).
//! Each page must stay within 0.3/255 of its recorded baseline (tests/chrome-baseline.txt): a page drifting away
//! from Chrome is a regression. The references were taken on macOS (CoreText fonts), so this runs only there.
//! CHROME_BASELINE=write records the baseline anew.
//! Run: cargo test --release --test chrome -- --nocapture
#![cfg(target_os = "macos")]
use std::ffi::CString;
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

fn load_png(path: &std::path::Path) -> Option<(u32, u32, Vec<u8>)> {
    let d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let px = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return None,
    };
    Some((info.width, info.height, px))
}

/// Mean error in 1/255: `ours` premultiplied, `chrome` straight alpha.
fn mean_error(ours: &[u8], ow: u32, chrome: &[u8], cw: u32, w: u32, h: u32) -> f64 {
    let mut sum = 0.0;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let (o, c) = ((y * ow as usize + x) * 4, (y * cw as usize + x) * 4);
            let (oa, ca) = (ours[o + 3] as f64 / 255.0, chrome[c + 3] as f64 / 255.0);
            let mut worst: f64 = 0.0;
            for bg in [0.0, 1.0] {
                for k in 0..3 {
                    let a = ours[o + k] as f64 / 255.0 + bg * (1.0 - oa);
                    let b = chrome[c + k] as f64 / 255.0 * ca + bg * (1.0 - ca);
                    worst = worst.max((a - b).abs());
                }
            }
            sum += worst * 255.0;
        }
    }
    sum / (w * h) as f64
}

#[test]
fn pages_stay_as_close_to_chrome_as_recorded() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests");
    let baseline_path = tests.join("chrome-baseline.txt");
    let baseline: std::collections::HashMap<String, f64> = std::fs::read_to_string(&baseline_path).unwrap_or_default()
        .lines().filter_map(|l| { let mut p = l.split_whitespace(); Some((p.next()?.to_string(), p.next()?.parse().ok()?)) }).collect();
    let write = std::env::var("CHROME_BASELINE").as_deref() == Ok("write");
    let mut out = String::new();
    let mut worse = Vec::new();
    for line in std::fs::read_to_string(tests.join("suite.txt")).unwrap().lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() < 4 { continue; }
        let (page, w, h, scale): (&str, u32, u32, f32) = (p[0], p[1].parse().unwrap(), p[2].parse().unwrap(), p[3].parse().unwrap());
        let name = std::path::Path::new(page).file_stem().unwrap().to_string_lossy().into_owned();
        let reference = tests.join("ref").join(if scale == 2.0 { format!("{name}-x2.png") } else { format!("{name}.png") });
        let Some((cw, ch, chrome)) = load_png(&reference) else { continue };
        let file = tests.join(page);
        let html = std::fs::read_to_string(&file).unwrap();
        let (pw, ph) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
        let mut px = vec![0u8; (pw * ph * 4) as usize];
        unsafe {
            let ui = vitrail_create(s(&html).as_ptr(), s(&format!("file://{}", file.canonicalize().unwrap().display())).as_ptr(), pw, ph, scale);
            // Scripts and timers run as in Chrome's 2 s of virtual time.
            for i in 0..=20 { vitrail_tick(ui, i as f64 * 0.1); }
            vitrail_render(ui, px.as_mut_ptr(), px.len());
            vitrail_destroy(ui);
        }
        let e = mean_error(&px, pw, &chrome, cw, pw.min(cw), ph.min(ch));
        let was = baseline.get(&name).copied();
        println!("{name:<24} {e:.2}/255{}", was.map(|b| format!("  (baseline {b:.2})")).unwrap_or_default());
        out.push_str(&format!("{name} {e:.3}\n"));
        if let Some(b) = was { if e > b + 0.3 { worse.push(format!("{name}: {e:.2} against {b:.2}")); } }
    }
    if write { std::fs::write(&baseline_path, out).unwrap(); return; }
    assert!(worse.is_empty(), "pages drifted away from Chrome: {worse:?}");
}
