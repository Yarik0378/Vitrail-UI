//! Prints a page's GPU ops (PAGE=... cargo test --release -p vitrail --test ops -- --nocapture).
use std::ffi::CString;
use vitrail::*;

#[test]
fn print_ops() {
    let page = std::env::var("PAGE").unwrap_or_else(|_| format!("{}/../../tests/pages/13-panel.html", env!("CARGO_MANIFEST_DIR")));
    let html = std::fs::read_to_string(&page).unwrap();
    unsafe {
        let (w, h) = std::env::var("SIZE").ok().and_then(|v| { let mut p = v.split('x'); Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?)) }).unwrap_or((1422u32, 118u32));
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new(format!("file://{page}")).unwrap().as_ptr(), w, h, 1.0);
        if std::env::var_os("AA").is_some() { vitrail_set_gpu(ui, 2); }
        let mut c = [0u32; 7];
        vitrail_gpu_build(ui, c.as_mut_ptr());
        let mut ops = vec![gpu::Op::default(); c[5] as usize];
        vitrail_gpu_ops(ui, ops.as_mut_ptr() as *mut u8, ops.len() * std::mem::size_of::<gpu::Op>());
        let mut batches = vec![gpu::Batch::default(); c[2] as usize];
        let mut v = vec![0u8; c[0] as usize * std::mem::size_of::<gpu::Vertex>()]; let mut i = vec![0u32; c[1] as usize];
        vitrail_gpu_copy(ui, v.as_mut_ptr(), v.len(), i.as_mut_ptr(), i.len(), batches.as_mut_ptr() as *mut u8, batches.len() * 20);
        let mut depth = 0i32; let mut s = String::new();
        for op in &ops {
            let b = batches[op.batch as usize];
            match op.kind { 0 => s += &format!("draw(k{} n{}{}) ", b.kind, b.count, if b.stencil > 0 { " sc" } else { "" }), 1 => { depth += 1; s += &format!("clip+{}(n{}) ", depth, b.count) }, 2 => { s += &format!("clip-{} ", depth); depth -= 1 }, 3 => s += &format!("GROUP[ "), 6 => s += "LAYER[ ", 7 => s += "]L ", 4 => s += &format!("]c{} a{} ", op.compose, op.alpha), 5 => s += "GLASS ", _ => {} }
        }
        println!("{s}");
        vitrail_destroy(ui);
    }
}
