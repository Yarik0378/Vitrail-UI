//! CSS animations and transitions run on the clock `vitrail_tick` gives.
use std::ffi::CString;
use vitrail::*;

unsafe fn pixels(ui: *mut Vitrail, w: u32, h: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];
    unsafe { vitrail_render(ui, buf.as_mut_ptr(), buf.len()) };
    buf
}

/// The x of the first opaque pixel in row `y`.
fn left_edge(buf: &[u8], w: u32, y: u32) -> Option<u32> {
    (0..w).find(|&x| buf[((y * w + x) * 4 + 3) as usize] > 128)
}

#[test]
fn keyframes_move_with_the_clock() {
    let html = r#"<html><head><style>
        @keyframes slide { from { transform: translateX(0px) } to { transform: translateX(100px) } }
        body { margin: 0 }
        #b { width: 20px; height: 20px; background: #c33; animation: slide 1s linear infinite; }
        </style></head><body><div id="b"></div></body></html>"#;
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new("file:///tmp/a.html").unwrap().as_ptr(), 200, 40, 1.0);
        // Frames at 60 per second, as the game ticks.
        let mut f = 0.0;
        let mut at = |t: f64| { while f < t { f += 1.0 / 60.0; vitrail_tick(ui, f); let _ = pixels(ui, 200, 40); } left_edge(&pixels(ui, 200, 40), 200, 10).unwrap() };
        let a = at(0.1);
        let b = at(0.6);
        assert_eq!(vitrail_tick(ui, 0.6), 1, "an infinite animation keeps ticking");
        assert!(b > a + 30, "half a second later the box moved: {a} -> {b}");
        vitrail_destroy(ui);
    }
}

#[test]
fn transitions_run_and_stop() {
    let html = r#"<html><head><style>
        body { margin: 0 }
        #b { width: 40px; height: 40px; background: rgb(0, 0, 255); transition: background-color 1s linear; }
        #b.on { background: rgb(255, 0, 0); }
        </style></head><body><div id="b"></div></body></html>"#;
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new("file:///tmp/t.html").unwrap().as_ptr(), 40, 40, 1.0);
        vitrail_tick(ui, 1.0);
        let _ = pixels(ui, 40, 40);
        vitrail_set_class(ui, CString::new("#b").unwrap().as_ptr(), CString::new("on").unwrap().as_ptr(), 1);
        vitrail_tick(ui, 1.0);
        let _ = pixels(ui, 40, 40); // the transition starts here
        vitrail_tick(ui, 1.5);
        let mid = pixels(ui, 40, 40);
        let (r, b) = (mid[(20 * 40 + 20) * 4], mid[(20 * 40 + 20) * 4 + 2]);
        assert!(r > 60 && b > 60, "halfway the colour is between blue and red: r {r} b {b}");
        vitrail_tick(ui, 3.0);
        let end = pixels(ui, 40, 40);
        assert!(end[(20 * 40 + 20) * 4] > 240, "after it ends the box is red");
        assert_eq!(vitrail_tick(ui, 3.1), 0, "nothing animates any more");
        vitrail_destroy(ui);
    }
}

#[test]
fn transform_transitions_move_layers_without_painting() {
    let html = r#"<html><head><style>
        body { margin: 0 }
        #b { width: 40px; height: 40px; background: #c33; transition: transform 1s linear, opacity 1s linear; }
        #b.on { transform: translateX(100px); opacity: 0.5; }
        </style></head><body><div id="b"></div><p>still text</p></body></html>"#;
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new("file:///tmp/c.html").unwrap().as_ptr(), 200, 80, 1.0);
        vitrail_set_gpu(ui, 2);
        let mut c = [0u32; 7];
        vitrail_tick(ui, 1.0);
        vitrail_gpu_build(ui, c.as_mut_ptr());
        vitrail_set_class(ui, CString::new("#b").unwrap().as_ptr(), CString::new("on").unwrap().as_ptr(), 1);
        vitrail_tick(ui, 1.0);
        assert_eq!(vitrail_needs_paint(ui), 1, "the class change paints once");
        vitrail_gpu_build(ui, c.as_mut_ptr()); // the transition starts here, #b becomes a layer
        assert_eq!(vitrail_gpu_layers(ui, std::ptr::null_mut(), 0), 1, "one composited layer");
        let mut st = [0f32; 7];
        let mut xs = Vec::new();
        for i in 1..=5 {
            assert_eq!(vitrail_tick(ui, 1.0 + i as f64 * 0.1), 2, "frame {i}: only the layer moves");
            vitrail_layer_states(ui, st.as_mut_ptr(), 7);
            xs.push(st[4]);
        }
        assert!((xs[4] - 50.0).abs() < 3.0, "half way the layer is 50 px along: {xs:?}");
        assert!(xs.windows(2).all(|w| w[1] > w[0]), "it moves every frame: {xs:?}");
        assert!((st[6] - 0.75).abs() < 0.05, "and fades: opacity {}", st[6]);
        // The end: one more paint, as the page now stands.
        for i in 6..=12 { vitrail_tick(ui, 1.0 + i as f64 * 0.1); }
        assert_eq!(vitrail_tick(ui, 2.5), 0);
        assert_eq!(vitrail_needs_paint(ui), 1, "the end paints once");
        vitrail_destroy(ui);
    }
}
