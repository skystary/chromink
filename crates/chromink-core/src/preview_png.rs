//! Lossless viewport transport; SVG export is independent of PNG compression.
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ImageEncoder,
};
pub fn encode_rgba(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Sub)
        .write_image(pixels, width, height, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(png)
}
pub fn encode_pixmap(mut pixmap: tiny_skia::Pixmap) -> Result<Vec<u8>, String> {
    // Same rounding as tiny-skia's encoder, but consume the pixmap to avoid its full copy.
    for pixel in pixmap.data_mut().chunks_exact_mut(4) {
        let c = tiny_skia::PremultipliedColorU8::from_rgba(pixel[0], pixel[1], pixel[2], pixel[3])
            .ok_or("无效的预乘颜色")?
            .demultiply();
        pixel.copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    encode_rgba(pixmap.data(), pixmap.width(), pixmap.height())
}
