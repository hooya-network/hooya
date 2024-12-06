use anyhow::Result;
use std::{path::Path, process::Command};

pub fn preview(
    in_video: &Path,
    out_file: &Path,
    long_edge: u32,
) -> Result<(u32, u32)> {
    let in_video_str = in_video
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid input path"))?;
    let out_file_str = out_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid output path"))?;

    let video_metadata = extract_video_metadata(in_video)?;
    let width = video_metadata.width;
    let height = video_metadata.height;
    let duration = video_metadata.duration;
    let audio_tracks = video_metadata.audio_tracks;

    // Calculate scale preserving aspect ratio
    let aspect_ratio = width as f64 / height as f64;
    let (mut scaled_width, mut scaled_height) = if aspect_ratio > 1.0 {
        // Landscape
        (long_edge, (long_edge as f64 / aspect_ratio) as u32)
    } else {
        // Portrait
        ((long_edge as f64 * aspect_ratio) as u32, long_edge)
    };

    // hacky sidestep around when MP4 width / height not divisible by 2
    if scaled_width % 2 > 0 {
        scaled_width += 1;
    } else if scaled_height % 2 > 0 {
        scaled_height += 1;
    }

    // 8s preview with 2s snippets
    let step = duration / 5.0;
    let timestamps = [step, 2.0 * step, 3.0 * step, 4.0 * step];

    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(in_video_str).arg("-y");

    // Don't snip previews if less than this 16s
    if duration > 16.0 {
        if audio_tracks > 0 {
            // Sound on
            command.args([
                "-filter_complex",
                &format!(
                    "[0:v]trim=start={t1}:end={t2},setpts=PTS-STARTPTS,scale={w}:{h}[v1];\
                     [0:a]atrim=start={t1}:end={t2},asetpts=PTS-STARTPTS[a1];\
                     [0:v]trim=start={t3}:end={t4},setpts=PTS-STARTPTS,scale={w}:{h}[v2];\
                     [0:a]atrim=start={t3}:end={t4},asetpts=PTS-STARTPTS[a2];\
                     [0:v]trim=start={t5}:end={t6},setpts=PTS-STARTPTS,scale={w}:{h}[v3];\
                     [0:a]atrim=start={t5}:end={t6},asetpts=PTS-STARTPTS[a3];\
                     [0:v]trim=start={t7}:end={t8},setpts=PTS-STARTPTS,scale={w}:{h}[v4];\
                     [0:a]atrim=start={t7}:end={t8},asetpts=PTS-STARTPTS[a4];\
                     [v1][a1][v2][a2][v3][a3][v4][a4]concat=n=4:v=1:a=1[outv][outa]",
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
                "-map", "[outv]",
                "-map", "[outa]"
            ]);
        } else {
            // No sound
            command.args([
                "-filter_complex",
                &format!(
                    "[0:v]trim=start={t1}:end={t2},setpts=PTS-STARTPTS,scale={w}:{h}[v1];\
                     [0:v]trim=start={t3}:end={t4},setpts=PTS-STARTPTS,scale={w}:{h}[v2];\
                     [0:v]trim=start={t5}:end={t6},setpts=PTS-STARTPTS,scale={w}:{h}[v3];\
                     [0:v]trim=start={t7}:end={t8},setpts=PTS-STARTPTS,scale={w}:{h}[v4];\
                     [v1][v2][v3][v4]concat=n=4:v=1:a=0[outv]",
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
        }
    } else {
        // Don't snip here because duration is short
        command.args([
            "-vf",
            &format!("scale={w}:{h}", w = scaled_width, h = scaled_height),
        ]);
    }

    command
        .arg("-c:v")
        .arg("libsvtav1")
        .arg("-f")
        .arg("mp4")
        .arg(out_file_str);

    // Execute command
    let status = command.status()?;
    if status.success() {
        Ok((scaled_height, scaled_width))
    } else {
        Err(anyhow::anyhow!("ffmpeg command failed"))
    }
}

pub fn extract_video_metadata(in_video: &Path) -> Result<VideoMetadata> {
    let in_video_str = in_video
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid input path"))?;

    // Get video duration, width, and height using `ffprobe`
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,duration",
            "-of",
            "default=noprint_wrappers=1",
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
        if let Some(stripped) = line.strip_prefix("width=") {
            width = stripped
                .parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Invalid width format"))?;
        } else if let Some(stripped) = line.strip_prefix("height=") {
            height = stripped
                .parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Invalid height format"))?;
        } else if let Some(stripped) = line.strip_prefix("duration=") {
            duration = stripped
                .parse::<f64>()
                .map_err(|_| anyhow::anyhow!("Invalid duration format"))?;
        }
    }

    // Determine the number of audio tracks
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=channels",
            "-select_streams",
            "a",
            "-of",
            "default=noprint_wrappers=1",
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
    let audio_tracks = ffprobe_output.lines().count() as u32;

    Ok(VideoMetadata {
        width,
        height,
        duration,
        audio_tracks,
    })
}

pub struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub audio_tracks: u32,
}
