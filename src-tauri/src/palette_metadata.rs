use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

// Only declared palette sizes are reported; true-color pixels are never counted.
pub fn color_count(path: &Path) -> Option<u32> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 54];
    let read = file.read(&mut header).ok()?;
    if read >= 26 && &header[..8] == b"\x89PNG\r\n\x1a\n" {
        if header[25] != 3 {
            return None;
        }
        file.seek(SeekFrom::Start(8)).ok()?;
        loop {
            let mut chunk = [0u8; 8];
            file.read_exact(&mut chunk).ok()?;
            let len = u32::from_be_bytes(chunk[..4].try_into().ok()?);
            match &chunk[4..] {
                b"PLTE" => return (len > 0 && len <= 768 && len % 3 == 0).then_some(len / 3),
                b"IDAT" | b"IEND" => return None,
                _ => {
                    file.seek(SeekFrom::Current(i64::from(len) + 4)).ok()?;
                }
            }
        }
    }
    if read >= 30 && &header[..2] == b"BM" {
        let dib = u32::from_le_bytes(header[14..18].try_into().ok()?);
        let bits = if dib == 12 {
            u16::from_le_bytes(header[24..26].try_into().ok()?)
        } else if dib >= 40 {
            u16::from_le_bytes(header[28..30].try_into().ok()?)
        } else {
            return None;
        };
        if !matches!(bits, 1 | 4 | 8) {
            return None;
        }
        let declared = if dib >= 40 && read >= 50 {
            u32::from_le_bytes(header[46..50].try_into().ok()?)
        } else {
            0
        };
        return Some(if declared == 0 {
            1u32 << bits
        } else {
            declared.min(1u32 << bits)
        });
    }
    if read >= 13 && (&header[..6] == b"GIF87a" || &header[..6] == b"GIF89a") {
        let mut count = 0;
        file.seek(SeekFrom::Start(13)).ok()?;
        if header[10] & 0x80 != 0 {
            count = 1u32 << ((header[10] & 7) + 1);
            file.seek(SeekFrom::Current(i64::from(count) * 3)).ok()?;
        }
        fn byte(file: &mut File) -> Option<u8> {
            let mut b = [0];
            file.read_exact(&mut b).ok()?;
            Some(b[0])
        }
        fn blocks(file: &mut File) -> Option<()> {
            loop {
                let n = byte(file)?;
                if n == 0 {
                    return Some(());
                }
                file.seek(SeekFrom::Current(i64::from(n))).ok()?;
            }
        }
        loop {
            match byte(&mut file)? {
                0x3b => return (count > 0).then_some(count),
                0x21 => {
                    byte(&mut file)?;
                    blocks(&mut file)?;
                }
                0x2c => {
                    let mut descriptor = [0u8; 9];
                    file.read_exact(&mut descriptor).ok()?;
                    if descriptor[8] & 0x80 != 0 {
                        let n = 1u32 << ((descriptor[8] & 7) + 1);
                        count = count.max(n);
                        file.seek(SeekFrom::Current(i64::from(n) * 3)).ok()?;
                    }
                    byte(&mut file)?;
                    blocks(&mut file)?;
                }
                _ => return None,
            }
        }
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tga"))
        && read >= 18
        && header[1] == 1
        && matches!(header[2], 1 | 9)
    {
        let count = u16::from_le_bytes([header[5], header[6]]) as u32;
        return (count > 0).then_some(count);
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tif") || e.eq_ignore_ascii_case("tiff"))
    {
        file.seek(SeekFrom::Start(0)).ok()?;
        let mut decoder = tiff::decoder::Decoder::new(file).ok()?;
        let mut count = 0;
        loop {
            // A non-indexed page means the shared source has no palette limit.
            let map = decoder.get_tag_u16_vec(tiff::tags::Tag::ColorMap).ok()?;
            if map.is_empty() || map.len() % 3 != 0 {
                return None;
            }
            count = count.max((map.len() / 3) as u32);
            if !decoder.more_images() {
                break;
            }
            decoder.next_image().ok()?;
        }
        return Some(count);
    }
    None
}
