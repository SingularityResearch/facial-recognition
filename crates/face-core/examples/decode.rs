//! A JPEG decoded by the core, written as a .bgr dump (width, height as little-endian 32-bit numbers, then the
//! bytes): for comparing the decoder with another one.
//!
//!     decode <in.jpg> <out.bgr>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: decode <in.jpg> <out.bgr>".into());
    }
    let img = face_core::image::Bgr::from_jpeg(&std::fs::read(&args[1])?)?;
    let mut out = Vec::with_capacity(8 + img.px.len());
    out.extend_from_slice(&(img.w as u32).to_le_bytes());
    out.extend_from_slice(&(img.h as u32).to_le_bytes());
    out.extend_from_slice(&img.px);
    std::fs::write(&args[2], out)?;
    Ok(())
}
