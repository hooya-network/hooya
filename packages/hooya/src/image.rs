use std::io::{BufReader, Read, Seek};
use std::path::PathBuf;

use anyhow::Result;
use image::ImageReader;
use image::{DynamicImage, ImageFormat};

pub fn thumbnail(
    in_image: &DynamicImage,
    orientation: Option<u32>,
    out_file: &PathBuf,
    long_edge: u32,
) -> Result<(u32, u32)> {
    let thumb = in_image.thumbnail(long_edge, long_edge);
    let transformed = match orientation {
        Some(1) => thumb,
        Some(3) => thumb.rotate180(),
        Some(6) => thumb.rotate90(),
        Some(8) => thumb.rotate270(),
        _ => thumb,
    }
    .into_rgba8();

    transformed.save_with_format(out_file, ImageFormat::Avif)?;

    Ok((transformed.height(), transformed.width()))
}

pub fn read(
    local_file: &PathBuf,
    mimetype: &str,
) -> Result<(DynamicImage, Option<exif::Exif>)> {
    let fh = std::fs::File::open(local_file)?;
    let format = ImageFormat::from_mime_type(mimetype).unwrap();

    let mut b_reader = BufReader::new(fh);

    let decoded_data = {
        let mut reader = ImageReader::new(b_reader.by_ref());
        reader.set_format(format);
        reader.decode()?
    };

    let exif_data = {
        b_reader.seek(std::io::SeekFrom::Start(0))?;
        exif::Reader::new()
            .read_from_container(b_reader.by_ref())
            .ok()
    };

    Ok((decoded_data, exif_data))
}
