use std::io;

/// PS1 VRAM is 1024x512 16-bit pixels.
pub const VRAM_WIDTH: usize = 1024;
pub const VRAM_HEIGHT: usize = 512;

/// Tag at the start of a file holding several images (the game checks for 0x20).
const PACKED_TAG: u32 = 0x20;
/// `struct VramHeader`: 0xC bytes of TIM header, then a RECT, then pixels.
const HEADER_SIZE: usize = 0x14;
const RECT_OFFSET: usize = 0xC;

/// One image to upload, in 16-bit pixels (`rect.w` is in pixels, not bytes).
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VramImage {
    pub x: i16,
    pub y: i16,
    pub w: i16,
    pub h: i16,
    /// `w * h` little-endian 16-bit pixels, row by row.
    pub pixels: Vec<u8>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn word(file: &[u8], at: usize) -> io::Result<u32> {
    file.get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid("VRAM file truncated"))
}

fn image_at(file: &[u8], at: usize) -> io::Result<VramImage> {
    let hdr = file
        .get(at..at + HEADER_SIZE)
        .ok_or_else(|| invalid("VRAM header truncated"))?;
    let rect = |i: usize| i16::from_le_bytes(hdr[RECT_OFFSET + i * 2..][..2].try_into().unwrap());
    let (x, y, w, h) = (rect(0), rect(1), rect(2), rect(3));
    if w < 0 || h < 0 {
        return Err(invalid(format!("negative VRAM rect {w}x{h}")));
    }
    let len = w as usize * h as usize * 2;
    let pixels = file
        .get(at + HEADER_SIZE..at + HEADER_SIZE + len)
        .ok_or_else(|| invalid("VRAM pixel data truncated"))?
        .to_vec();
    Ok(VramImage { x, y, w, h, pixels })
}

/// Parses a `LT_VRAM` subfile into the images `LOAD_VramFileCallback` would
/// hand to `LoadImage`: either a single image, or a packed list
/// `0x20, (size, image of `size` bytes)..., 0`.
pub fn parse_vram_file(file: &[u8]) -> io::Result<Vec<VramImage>> {
    if word(file, 0)? != PACKED_TAG {
        return Ok(vec![image_at(file, 0)?]);
    }
    let mut images = Vec::new();
    let mut at = 4;
    loop {
        let size = word(file, at)? as usize;
        if size == 0 {
            return Ok(images);
        }
        at += 4;
        images.push(image_at(file, at)?);
        at += size;
    }
}

/// A software copy of PS1 VRAM that images can be uploaded into.
pub struct Vram {
    pub pixels: Vec<u16>,
}

impl Default for Vram {
    fn default() -> Self {
        Self {
            pixels: vec![0; VRAM_WIDTH * VRAM_HEIGHT],
        }
    }
}

impl Vram {
    /// Equivalent of the PSX `LoadImage`.
    pub fn upload(&mut self, img: &VramImage) -> io::Result<()> {
        let (x, y, w, h) = (
            img.x as usize,
            img.y as usize,
            img.w as usize,
            img.h as usize,
        );
        if x + w > VRAM_WIDTH || y + h > VRAM_HEIGHT {
            return Err(invalid(format!(
                "image {w}x{h} at ({x},{y}) is outside VRAM"
            )));
        }
        for row in 0..h {
            let dst = &mut self.pixels[(y + row) * VRAM_WIDTH + x..][..w];
            for (px, src) in dst
                .iter_mut()
                .zip(img.pixels[row * w * 2..][..w * 2].as_chunks::<2>().0)
            {
                *px = u16::from_le_bytes(*src);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(x: i16, y: i16, w: i16, h: i16, fill: u8) -> Vec<u8> {
        let mut v = vec![0u8; RECT_OFFSET];
        for n in [x, y, w, h] {
            v.extend(n.to_le_bytes());
        }
        v.extend(vec![fill; w as usize * h as usize * 2]);
        v
    }

    #[test]
    fn single_image() {
        let imgs = parse_vram_file(&image(512, 8, 2, 3, 7)).unwrap();
        assert_eq!(imgs.len(), 1);
        assert_eq!((imgs[0].x, imgs[0].y, imgs[0].w, imgs[0].h), (512, 8, 2, 3));
        assert_eq!(imgs[0].pixels.len(), 12);
    }

    #[test]
    fn packed_images() {
        let (a, b) = (image(0, 0, 4, 1, 1), image(16, 16, 1, 2, 2));
        let mut f = PACKED_TAG.to_le_bytes().to_vec();
        for img in [&a, &b] {
            f.extend((img.len() as u32).to_le_bytes());
            f.extend(img.iter());
        }
        f.extend(0u32.to_le_bytes());
        let imgs = parse_vram_file(&f).unwrap();
        assert_eq!(imgs.len(), 2);
        assert_eq!((imgs[1].x, imgs[1].y), (16, 16));
    }

    #[test]
    fn upload_writes_rect() {
        let mut vram = Vram::default();
        let img = parse_vram_file(&image(10, 20, 2, 2, 0xAB))
            .unwrap()
            .remove(0);
        vram.upload(&img).unwrap();
        assert_eq!(vram.pixels[20 * VRAM_WIDTH + 10], 0xABAB);
        assert_eq!(vram.pixels[21 * VRAM_WIDTH + 11], 0xABAB);
        assert_eq!(vram.pixels[20 * VRAM_WIDTH + 12], 0);
    }

    #[test]
    fn rejects_truncated_and_out_of_bounds() {
        let mut f = image(0, 0, 4, 4, 0);
        f.truncate(f.len() - 1);
        assert!(parse_vram_file(&f).is_err());
        let img = parse_vram_file(&image(1023, 0, 2, 1, 0)).unwrap().remove(0);
        assert!(Vram::default().upload(&img).is_err());
    }
}
