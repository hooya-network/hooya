use std::io::BufReader;
use std::path::PathBuf;

use anyhow::Result;
use image::io::Reader as ImageReader;
use image::{DynamicImage, ImageFormat};

pub fn thumbnail(
    local_file: &PathBuf,
    out_file: &PathBuf,
    mimetype: &str,
    long_edge: u32,
) -> Result<(u32, u32)> {
    let mut decoded = read(local_file, mimetype)?;
    decoded = decoded.thumbnail(long_edge, long_edge);
    decoded.save_with_format(out_file, ImageFormat::Jpeg)?;

    Ok((decoded.height(), decoded.width()))
}

pub fn read(local_file: &PathBuf, mimetype: &str) -> Result<DynamicImage> {
    let fh = std::fs::File::open(local_file)?;
    let b_reader = BufReader::new(fh);
    let format = ImageFormat::from_mime_type(mimetype).unwrap();

    let mut reader = ImageReader::new(b_reader);
    reader.set_format(format);
    Ok(reader.decode()?)
}
