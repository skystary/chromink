//! Multi-image sources keep their original pixel sizes and one shared parameter set.
use chromink_core::control::{memory_budget, validate_decode};
use image::{AnimationDecoder, DynamicImage, RgbaImage};
use std::{fs::File, io::BufReader, path::Path};

fn tiff(path: &Path) -> Result<tiff::decoder::Decoder<BufReader<File>>, String> {
    let mut limits = tiff::decoder::Limits::default();
    limits.decoding_buffer_size = memory_budget();
    limits.intermediate_buffer_size = memory_budget();
    tiff::decoder::Decoder::new(BufReader::new(File::open(path).map_err(|e| e.to_string())?))
        .map(|d| d.with_limits(limits))
        .map_err(|e| e.to_string())
}
pub fn supported(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        e.eq_ignore_ascii_case("tif")
            || e.eq_ignore_ascii_case("tiff")
            || e.eq_ignore_ascii_case("gif")
    })
}
pub fn dimensions(path: &Path) -> Result<Vec<(u32, u32)>, String> {
    let mut result = Vec::new();
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
    {
        let (w, h) = chromink_core::image_dimensions(path)?;
        validate_decode(w, h)?;
        let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(
            File::open(path).map_err(|e| e.to_string())?,
        ))
        .map_err(|e| e.to_string())?;
        for frame in decoder.into_frames() {
            frame.map_err(|e| e.to_string())?;
            result.push((w, h));
            if result.len() > 4096 {
                return Err("多页文件超过 4096 项上限".into());
            }
        }
    } else {
        let mut decoder = tiff(path)?;
        loop {
            let (w, h) = decoder.dimensions().map_err(|e| e.to_string())?;
            validate_decode(w, h)?;
            result.push((w, h));
            if result.len() > 4096 {
                return Err("多页文件超过 4096 项上限".into());
            }
            if !decoder.more_images() {
                break;
            }
            decoder.next_image().map_err(|e| e.to_string())?;
        }
    }
    Ok(result)
}
pub fn decode(path: &Path, index: u32) -> Result<DynamicImage, String> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
    {
        let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(
            File::open(path).map_err(|e| e.to_string())?,
        ))
        .map_err(|e| e.to_string())?;
        let (w, h) = image::ImageDecoder::dimensions(&decoder);
        validate_decode(w, h)?;
        let frame = decoder
            .into_frames()
            .nth(index as usize)
            .ok_or("找不到 GIF 帧")?
            .map_err(|e| e.to_string())?;
        return Ok(DynamicImage::ImageRgba8(frame.into_buffer()));
    }
    let mut decoder = tiff(path)?;
    for _ in 0..index {
        decoder.next_image().map_err(|e| e.to_string())?;
    }
    let (w, h) = decoder.dimensions().map_err(|e| e.to_string())?;
    validate_decode(w, h)?;
    let color = decoder.colortype().map_err(|e| e.to_string())?;
    let samples = if let tiff::ColorType::Gray(bits @ (1 | 2 | 4)) = color {
        // tiff 0.9 reads sub-byte strips as full-byte samples. Decode packed rows ourselves.
        use std::io::{Read, Seek, SeekFrom};
        use tiff::tags::Tag;
        if decoder.get_tag_u32(Tag::Compression).unwrap_or(1) != 1 {
            return Err("此低位深 TIFF 压缩方式暂不支持，请使用 8 位灰度或 RGB TIFF".into());
        }
        let offsets = decoder
            .get_tag_u64_vec(Tag::StripOffsets)
            .map_err(|e| e.to_string())?;
        let counts = decoder
            .get_tag_u64_vec(Tag::StripByteCounts)
            .map_err(|e| e.to_string())?;
        let strip_rows = decoder.get_tag_u32(Tag::RowsPerStrip).unwrap_or(h).max(1);
        if offsets.len() != counts.len() || offsets.len() != h.div_ceil(strip_rows) as usize {
            return Err("TIFF 条带数量无效".into());
        }
        let white_zero = decoder
            .get_tag_u32(Tag::PhotometricInterpretation)
            .map_err(|e| e.to_string())?
            == 0;
        let reverse = decoder.get_tag_u32(Tag::FillOrder).unwrap_or(1) == 2;
        let pitch = (w as usize * bits as usize).div_ceil(8);
        let mask = (1u16 << bits) - 1;
        let mut samples = Vec::with_capacity(w as usize * h as usize);
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        for (index, (&offset, &count)) in offsets.iter().zip(&counts).enumerate() {
            let rows = strip_rows.min(h - index as u32 * strip_rows) as usize;
            let size = pitch * rows;
            if count < size as u64 {
                return Err("TIFF 条带长度无效".into());
            }
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| e.to_string())?;
            let mut packed = vec![0; size];
            file.read_exact(&mut packed).map_err(|e| e.to_string())?;
            for y in 0..rows {
                for x in 0..w as usize {
                    let bit = x * bits as usize;
                    let byte = packed[y * pitch + bit / 8];
                    let byte = if reverse { byte.reverse_bits() } else { byte };
                    let value = (u16::from(byte) >> (8 - bits as usize - bit % 8)) & mask;
                    let gray = (value * 255 / mask) as u8;
                    samples.push(if white_zero { 255 - gray } else { gray });
                }
            }
        }
        samples
    } else {
        match decoder.read_image().map_err(|e| e.to_string())? {
            tiff::decoder::DecodingResult::U8(v) => v,
            tiff::decoder::DecodingResult::U16(v) => {
                v.into_iter().map(|x| (x >> 8) as u8).collect()
            }
            tiff::decoder::DecodingResult::F32(v) => v
                .into_iter()
                .map(|x| (x.clamp(0., 1.) * 255.).round() as u8)
                .collect(),
            _ => return Err("暂不支持此 TIFF 像素数据类型".into()),
        }
    };
    let channels = match color {
        tiff::ColorType::Gray(_) => 1,
        tiff::ColorType::GrayA(_) => 2,
        tiff::ColorType::RGB(_) => 3,
        tiff::ColorType::RGBA(_) | tiff::ColorType::CMYK(_) => 4,
        _ => return Err("暂不支持此 TIFF 色彩格式".into()),
    };
    if samples.len() != (w as usize) * (h as usize) * channels {
        return Err("TIFF 像素长度不正确".into());
    }
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for p in samples.chunks_exact(channels) {
        let pixel = match color {
            tiff::ColorType::Gray(_) => [p[0], p[0], p[0], 255],
            tiff::ColorType::GrayA(_) => [p[0], p[0], p[0], p[1]],
            tiff::ColorType::RGB(_) => [p[0], p[1], p[2], 255],
            tiff::ColorType::RGBA(_) => [p[0], p[1], p[2], p[3]],
            tiff::ColorType::CMYK(_) => {
                let rgb = [0, 1, 2]
                    .map(|c| ((255 - u16::from(p[c])) * (255 - u16::from(p[3])) / 255) as u8);
                [rgb[0], rgb[1], rgb[2], 255]
            }
            _ => unreachable!(),
        };
        rgba.extend_from_slice(&pixel);
    }
    Ok(DynamicImage::ImageRgba8(
        RgbaImage::from_raw(w, h, rgba).ok_or("TIFF 像素格式无效")?,
    ))
}
