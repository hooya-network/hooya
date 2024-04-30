use std::{path::PathBuf, process::Command};
use anyhow::Result;

pub fn preview(
    in_video: &PathBuf,
    out_file: &PathBuf,
    long_edge: u32,
) -> Result<(u32, u32)> {
    let in_video_str = in_video.to_str().ok_or_else(|| {
        anyhow::anyhow!("Invalid input path")
    })?;
    let out_file_str = out_file.to_str().ok_or_else(|| {
        anyhow::anyhow!("Invalid output path")
    })?;

    let video_metadata = extract_video_metadata(in_video)?;
    let width = video_metadata.width;
    let height = video_metadata.height;
    let duration = video_metadata.duration;

    // Calculate scale preserving aspect ratio
    let aspect_ratio = width as f64 / height as f64;
    let (scaled_width, scaled_height) = if aspect_ratio > 1.0 {
        // Landscape
        (long_edge, (long_edge as f64 / aspect_ratio) as u32)
    } else {
        // Portrait
        ((long_edge as f64 * aspect_ratio) as u32, long_edge)
    };

    // 8s preview with 2s snippets
    let step = duration / 5.0;
    let timestamps = [
        step,
        2.0 * step,
        3.0 * step,
        4.0 * step,
    ];

    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(in_video_str);
    command.arg("-y");

    /* We're really out here. We really do this.
     *
     * Maybe can gut the ffmpeg dependency when I pull in OpenCV but this
     * does exactly what I want for now.
     */
    command.args(&[
        "-filter_complex",
        &format!(
            "[0:v]trim=start={t1}:end={t2},setpts=PTS-STARTPTS,scale={w}:{h}[clip1];[0:v]trim=start={t3}:end={t4},setpts=PTS-STARTPTS,scale={w}:{h}[clip2];[0:v]trim=start={t5}:end={t6},setpts=PTS-STARTPTS,scale={w}:{h}[clip3];[0:v]trim=start={t7}:end={t8},setpts=PTS-STARTPTS,scale={w}:{h}[clip4];[clip1][clip2][clip3][clip4]concat=n=4:v=1:a=0[outv]",
            t1 = timestamps[0],
            t2 = timestamps[0] + 2.0,
            t3 = timestamps[1],
            t4 = timestamps[1] + 2.0,
            t5 = timestamps[2],
            t6 = timestamps[2] + 2.0,
            t7 = timestamps[3],
            t8 = timestamps[3] + 2.0,
            w = scaled_width,
            h = scaled_height
        ),
        "-map", "[outv]"
    ]);

    // Set format to MP4 and output file
    command.arg("-f").arg("mp4").arg("-movflags").arg("+faststart").arg(out_file_str);

    // Execute command
    let status = command.status()?;
    if status.success() {
        Ok((scaled_height, scaled_width))
    } else {
        Err(anyhow::anyhow!("ffmpeg command failed"))
    }
}

pub fn extract_video_metadata(in_video: &PathBuf) -> Result<VideoMetadata> {
    let in_video_str = in_video.to_str().ok_or_else(|| {
        anyhow::anyhow!("Invalid input path")
    })?;


    // Get video duration, width, and height using `ffprobe`
    let output = Command::new("ffprobe")
        .args(&[
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height,duration",
            "-of", "default=noprint_wrappers=1",
            in_video_str,
        ])
        .output();

    let output = output.map_err(|_| {
        anyhow::anyhow!("`ffprobe was not found. Check your PATH!`")
    })?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("Failed to fetch video information"));
    }

    let ffprobe_output = String::from_utf8(output.stdout)?;
    let mut width = 0;
    let mut height = 0;
    let mut duration = 0.0;

    for line in ffprobe_output.lines() {
        if line.starts_with("width=") {
            width = line[6..].parse::<u32>().map_err(|_| {
                anyhow::anyhow!("Invalid width format")
            })?;
        } else if line.starts_with("height=") {
            height = line[7..].parse::<u32>().map_err(|_| {
                anyhow::anyhow!("Invalid height format")
            })?;
        } else if line.starts_with("duration=") {
            duration = line[9..].parse::<f64>().map_err(|_| {
                anyhow::anyhow!("Invalid duration format")
            })?;
        }
    }

    Ok(VideoMetadata {
        width,
        height,
        duration
    })
}

pub struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
}
