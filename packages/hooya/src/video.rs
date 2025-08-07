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

    // codec fallbacks
    let codec_configs = [
        (
            "libsvtav1",
            "aac",
            vec!["-b:v", "1M", "-preset", "8", "-r", "30"],
        ),
        (
            "libvpx-vp9",
            "libopus",
            vec!["-b:v", "1M", "-crf", "30", "-b:a", "96k"],
        ),
        ("libx265", "aac", vec!["-b:v", "1M", "-preset", "medium"]),
    ];

    for (video_codec, audio_codec, codec_params) in codec_configs {
        tracing::debug!("attempting video preview with codec: {}", video_codec);
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-i").arg(in_video_str).arg("-y");

        if duration > 16.0 {
            if audio_tracks > 0 {
                cmd.args([
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
                cmd.args([
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
            cmd.args(["-vf", &format!("scale={scaled_width}:{scaled_height}")]);
        }

        cmd.arg("-c:v").arg(video_codec);

        // codec-specific parameters
        cmd.args(codec_params);

        // Only add audio codec if we have audio tracks
        if audio_tracks > 0 {
            cmd.arg("-c:a").arg(audio_codec);
        }

        cmd.arg("-f").arg("mp4").arg(out_file_str);

        let output = cmd.output().map_err(|e| {
            tracing::error!("ffmpeg not found or failed to execute: {}", e);
            anyhow::anyhow!("ffmpeg was not found. Check your PATH!")
        })?;

        if output.status.success() {
            tracing::debug!(
                "video preview succeeded with codec: {}",
                video_codec
            );
            return Ok((scaled_height, scaled_width));
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(
                "FFMPEG FAILED with codec {}: exit_code={:?}, stderr={}",
                video_codec,
                output.status.code(),
                stderr
            );
        }
    }

    Err(anyhow::anyhow!(
        "ffmpeg command failed with all codec combinations"
    ))
}

pub fn extract_video_metadata(in_video: &Path) -> Result<VideoMetadata> {
    let in_video_str = in_video
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid input path"))?;

    // log file details for debugging WebM issues
    if let Ok(metadata) = std::fs::metadata(in_video) {
        tracing::debug!(
            "extracting metadata from: path={}, size={} bytes",
            in_video_str,
            metadata.len()
        );
    }

    // Get video duration, width, and height using `ffprobe`
    // Note: WebM files may have duration in format instead of stream
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,duration:format=duration",
            "-of",
            "default=noprint_wrappers=1",
            in_video_str,
        ])
        .output();

    let output = output.map_err(|e| {
        tracing::error!("ffprobe not found or failed to execute: {}", e);
        anyhow::anyhow!("`ffprobe was not found. Check your PATH!`")
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(
            "FFPROBE FAILED for {}: exit_code={:?}, stderr={}",
            in_video_str,
            output.status.code(),
            stderr
        );
        return Err(anyhow::anyhow!(
            "Failed to fetch video information: {}",
            stderr
        ));
    }

    let ffprobe_output = String::from_utf8(output.stdout)?;
    tracing::debug!("ffprobe output for {}: {}", in_video_str, ffprobe_output);

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
            // try to parse duration, but don't fail if it's "N/A" (common in WebM)
            if stripped != "N/A" {
                duration = stripped.parse::<f64>().map_err(|e| {
                    tracing::warn!(
                        "failed to parse duration '{}': {}",
                        stripped,
                        e
                    );
                    anyhow::anyhow!("Invalid duration format: {}", stripped)
                })?;
            } else {
                tracing::debug!("duration is N/A for {}, likely WebM without duration metadata", in_video_str);
            }
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

    let output = output.map_err(|e| {
        tracing::error!(
            "ffprobe (audio) not found or failed to execute: {}",
            e
        );
        anyhow::anyhow!("`ffprobe was not found. Check your PATH!`")
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(
            "FFPROBE AUDIO FAILED for {}: exit_code={:?}, stderr={}",
            in_video_str,
            output.status.code(),
            stderr
        );
        return Err(anyhow::anyhow!(
            "Failed to fetch audio information: {}",
            stderr
        ));
    }

    let ffprobe_output = String::from_utf8(output.stdout)?;
    let audio_tracks = ffprobe_output.lines().count() as u32;

    // validate extracted metadata
    if width == 0 || height == 0 {
        tracing::error!(
            "invalid video dimensions for {}: width={}, height={}",
            in_video_str,
            width,
            height
        );
        return Err(anyhow::anyhow!(
            "Invalid video dimensions: {}x{}",
            width,
            height
        ));
    }

    if duration == 0.0 {
        tracing::warn!(
            "video duration is 0 for {}, may be a livestream or malformed WebM",
            in_video_str
        );
        // for WebM files without duration, set a minimal duration to avoid division by zero
        if in_video_str.to_lowercase().ends_with(".webm") {
            duration = 1.0; // placeholder duration for WebM files
            tracing::debug!(
                "set placeholder duration for WebM file: {}",
                in_video_str
            );
        }
    }

    tracing::debug!("extracted video metadata for {}: {}x{}, duration={:.2}s, audio_tracks={}", 
                   in_video_str, width, height, duration, audio_tracks);

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
