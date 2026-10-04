//! Explicit custom MV: a bounded, timestamp-selected consumer of ExportTimeline.
//! Video audio is never decoded. No looping or chart-end inference lives here.
use crate::{export_timeline::ExportTimeline, ffmpeg::FfmpegInstallation, formats};
use anyhow::{bail, Context, Result};
use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, ChildStdout, Command, Stdio},
    sync::{mpsc, Arc},
    thread::JoinHandle,
};

pub(crate) struct MvFrame {
    pub assets: formats::BackgroundAssets,
    pub quad: [[f64; 2]; 4],
    pub identity: crate::gpu_render::AtlasIdentity,
    pub media_pts: f64,
}

struct TimedFrame {
    frame: Arc<MvFrame>,
    duration: f64,
}

pub(crate) struct MvDecoder {
    timeline: ExportTimeline,
    background_percentage: f64,
    child: Child,
    stdout: Option<ChildStdout>,
    timestamps: Option<mpsc::Receiver<(f64, f64)>>,
    stderr: Option<JoinHandle<String>>,
    current: Option<TimedFrame>,
    next: Option<TimedFrame>,
    eof: bool,
    width: u32,
    height: u32,
    quad: [[f64; 2]; 4],
    configuration: formats::BackgroundConfiguration,
    nominal_duration: f64,
    source_end: f64,
    last_time: Option<f64>,
}

impl MvDecoder {
    pub fn set_background_percentage(&mut self, percent: f64) {
        self.background_percentage = percent;
    }

    pub fn new(
        tools: &FfmpegInstallation,
        path: &Path,
        timeline: ExportTimeline,
        width: u32,
        height: u32,
        configuration: formats::BackgroundConfiguration,
    ) -> Result<Self> {
        let probe = crate::export_control::hide_child_window(&mut Command::new(&tools.ffprobe))
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries",
                "stream=width,height,sample_aspect_ratio,avg_frame_rate,start_time,duration:stream_side_data=rotation:format=start_time,duration", "-of", "json"])
            .arg(path).output().context("probing MV")?;
        if !probe.status.success() {
            bail!(
                "MV probe failed: {}",
                String::from_utf8_lossy(&probe.stderr)
            );
        }
        let value: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        let stream = &value["streams"][0];
        let mut sw = stream["width"].as_u64().context("MV has no video width")? as f64;
        let mut sh = stream["height"]
            .as_u64()
            .context("MV has no video height")? as f64;
        let mut sar =
            ratio(stream["sample_aspect_ratio"].as_str().unwrap_or("1:1"), ':').unwrap_or(1.0);
        let rotation = stream["side_data_list"]
            .as_array()
            .and_then(|values| values.iter().find_map(|v| v["rotation"].as_f64()))
            .unwrap_or(0.0);
        if (rotation.rem_euclid(180.0) - 90.0).abs() < 1e-6 {
            std::mem::swap(&mut sw, &mut sh);
            sar = 1.0 / sar;
        } else if rotation.rem_euclid(90.0).abs() > 1e-6 {
            bail!("MV display rotation must be a multiple of 90 degrees");
        }
        let aspect = sw * sar / sh;
        if !aspect.is_finite() || aspect <= 0.0 {
            bail!("MV has invalid dimensions");
        }
        let viewport_aspect = f64::from(width) / f64::from(height);
        let scale = (f64::from(width) / (sw * sar)).min(f64::from(height) / sh);
        let width = (sw * sar * scale).round().max(1.0) as u32;
        let height = (sh * scale).round().max(1.0) as u32;
        let data = formats::BackgroundData {
            aspect_ratio: None,
            fit: "contain".into(),
            color: "#000".into(),
            scale_x: None,
            scale_y: None,
        };
        let quad = data.runtime_quad(viewport_aspect, f64::from(width) / f64::from(height))?;
        let fps = ratio(stream["avg_frame_rate"].as_str().unwrap_or("0/0"), '/').unwrap_or(30.0);
        let video_duration = stream["duration"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|t| t.is_finite() && *t > 0.0);
        let container_duration = value["format"]["duration"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|t| t.is_finite() && *t > 0.0);
        let origin = value["format"]["start_time"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|t| t.is_finite())
            .unwrap_or(0.0);
        let stream_start = stream["start_time"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|t| t.is_finite())
            .unwrap_or(origin);
        let source_end = video_duration
            .map(|duration| duration + stream_start - origin)
            .or(container_duration)
            .filter(|t| t.is_finite() && *t > 0.0)
            .context("MV has no finite positive duration")?;
        // Seek to a preceding keyframe and retain source PTS. Never reset the
        // first *decoded* frame to zero, which would misalign a nonzero export.
        let seek = (timeline.output_to_media(0.0) - 1.0 / fps)
            .max(0.0)
            .min(source_end);
        let mut child = crate::export_control::hide_child_window(&mut Command::new(&tools.ffmpeg))
            .args([
                "-nostdin",
                "-hide_banner",
                "-nostats",
                "-loglevel",
                "info",
                "-ss",
                &seek.to_string(),
                "-noaccurate_seek",
                "-copyts",
                "-i",
            ])
            .arg(path)
            .args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"])
            .arg(format!(
                "scale={width}:{height},setsar=1,format=rgba,showinfo"
            ))
            .args([
                "-fps_mode",
                "passthrough",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting MV decoder")?;
        let stdout = child.stdout.take();
        let stderr_pipe = child
            .stderr
            .take()
            .context("MV decoder stderr unavailable")?;
        let (sender, receiver) = mpsc::sync_channel(8);
        let stderr = std::thread::spawn(move || {
            let mut tail = std::collections::VecDeque::new();
            let mut time_base = f64::NAN;
            for line in BufReader::new(stderr_pipe).lines() {
                let Ok(line) = line else { break };
                if line.contains("Parsed_showinfo") {
                    if let Some((_, rest)) = line.split_once("config in time_base:") {
                        time_base = ratio(rest.split(',').next().unwrap_or("").trim(), '/')
                            .unwrap_or(f64::NAN);
                    }
                    if let Some(pts) = field(&line, "pts:") {
                        if sender
                            .send((
                                pts * time_base - origin,
                                field(&line, "duration:").unwrap_or(0.0) * time_base,
                            ))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                if tail.len() == 32 {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
            tail.into_iter().collect::<Vec<_>>().join("\n")
        });
        Ok(Self {
            timeline,
            background_percentage: 100.0,
            child,
            stdout,
            timestamps: Some(receiver),
            stderr: Some(stderr),
            current: None,
            next: None,
            eof: false,
            width,
            height,
            quad,
            configuration,
            nominal_duration: 1.0 / fps,
            source_end,
            last_time: None,
        })
    }

    fn read_frame(&mut self) -> Result<Option<TimedFrame>> {
        let mut rgba = vec![0; self.width as usize * self.height as usize * 4];
        let stdout = self
            .stdout
            .as_mut()
            .context("MV decoder output unavailable")?;
        if stdout.read(&mut rgba[..1])? == 0 {
            self.eof = true;
            let status = self.child.wait()?;
            self.timestamps.take();
            let errors = self
                .stderr
                .take()
                .and_then(|h| h.join().ok())
                .unwrap_or_default();
            if !status.success() {
                bail!("MV decode failed: {errors}");
            }
            return Ok(None);
        }
        stdout
            .read_exact(&mut rgba[1..])
            .context("truncated MV frame")?;
        let (media_pts, duration) = self
            .timestamps
            .as_ref()
            .context("MV timestamps unavailable")?
            .recv()
            .context("MV frame has no timestamp")?;
        if !media_pts.is_finite() {
            bail!("invalid MV timestamp");
        }
        scale_background(&mut rgba, self.background_percentage);
        let identity = crate::gpu_render::atlas_identity(&rgba, self.width, self.height);
        Ok(Some(TimedFrame {
            frame: Arc::new(MvFrame {
                assets: formats::BackgroundAssets {
                    width: self.width,
                    height: self.height,
                    rgba,
                    data: formats::BackgroundData {
                        aspect_ratio: None,
                        fit: "contain".into(),
                        color: "#000".into(),
                        scale_x: None,
                        scale_y: None,
                    },
                    configuration: self.configuration.clone(),
                },
                quad: self.quad,
                identity,
                media_pts,
            }),
            duration: if duration > 0.0 {
                duration
            } else {
                self.nominal_duration
            },
        }))
    }

    pub fn at_watch_time(&mut self, watch_time: f64) -> Result<Option<Arc<MvFrame>>> {
        let time = self.timeline.watch_to_media(watch_time);
        if !time.is_finite() || self.last_time.is_some_and(|last| time < last) {
            bail!("MV time must advance monotonically");
        }
        self.last_time = Some(time);
        if time < 0.0 || time >= self.source_end {
            return Ok(None);
        }
        if self.next.is_none() && !self.eof {
            self.next = self.read_frame()?;
        }
        while self
            .next
            .as_ref()
            .is_some_and(|f| f.frame.media_pts <= time + 1e-9)
        {
            let next = self.next.take().unwrap();
            if self
                .current
                .as_ref()
                .is_some_and(|f| next.frame.media_pts < f.frame.media_pts)
            {
                bail!("MV presentation timestamps are not ordered");
            }
            self.current = Some(next);
            self.next = self.read_frame()?;
        }
        Ok(self
            .current
            .as_ref()
            .filter(|f| !self.eof || time < f.frame.media_pts + f.duration - 1e-9)
            .map(|f| f.frame.clone()))
    }
}

impl Drop for MvDecoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        self.stdout.take();
        self.timestamps.take();
        let _ = self.child.wait();
        if let Some(thread) = self.stderr.take() {
            let _ = thread.join();
        }
    }
}

fn field(line: &str, key: &str) -> Option<f64> {
    line.split_once(key)?
        .1
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}
fn ratio(value: &str, separator: char) -> Option<f64> {
    let (a, b) = value.split_once(separator)?;
    let result = a.parse::<f64>().ok()? / b.parse::<f64>().ok()?;
    (result.is_finite() && result > 0.0).then_some(result)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn video_presentation_ignores_decoded_alpha_and_dims_only_rgb() {
        let source = [201, 77, 129, 0, 20, 80, 160, 128, 255, 1, 3, 255];
        for percent in [100.0, 50.0, 0.0] {
            let mut rgba = source;
            scale_background(&mut rgba, percent);
            for (a, b) in rgba.chunks_exact(4).zip(source.chunks_exact(4)) {
                assert_eq!(a[3], 255);
                for c in 0..3 {
                    assert_eq!(a[c], (f64::from(b[c]) * percent / 100.0).round() as u8);
                }
            }
        }
    }
    #[test]
    fn opaque_video_and_partial_gameplay_follow_source_over_on_cpu_and_wgpu() {
        let gpu = crate::gpu_render::GpuRenderer::shared().unwrap();
        let transform = std::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 });
        let mut sprite_transform = [[0.0; 8]; 8];
        for (i, row) in sprite_transform.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        let skin = formats::SkinAssets {
            width: 1,
            height: 1,
            interpolation: false,
            rgba: vec![201, 77, 129, 128],
            sprites: [(
                "partial".into(),
                formats::SkinSpriteAsset {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    transform: sprite_transform,
                },
            )]
            .into(),
        };
        let bindings = [(0, "partial".into())].into();
        let full = [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]];
        let list = crate::runtime::DisplayList {
            sprites: vec![crate::runtime::SpriteDraw {
                sprite_id: 0,
                corners: full,
                z: [0.0; 4],
                alpha: 0.5,
                provenance: None,
                trace: None,
            }],
        };
        for percent in [None, Some(100.0), Some(50.0), Some(0.0)] {
            let mut presented = vec![40, 100, 220, 0];
            scale_background(&mut presented, percent.unwrap_or(0.0));
            let bg = formats::BackgroundAssets {
                width: 1,
                height: 1,
                rgba: presented.clone(),
                data: formats::BackgroundData {
                    aspect_ratio: None,
                    fit: "contain".into(),
                    color: "#000".into(),
                    scale_x: None,
                    scale_y: None,
                },
                configuration: formats::BackgroundConfiguration {
                    blur: 0.0,
                    mask: "#0000".into(),
                },
            };
            let cpu = list
                .render_skin_rgb_with_runtime_transform_and_background(
                    4,
                    4,
                    1.0,
                    &skin,
                    &bindings,
                    &transform,
                    percent.map(|_| (&bg, full)),
                )
                .unwrap();
            let render = |list: &crate::runtime::DisplayList| {
                crate::gpu_render::render_display_list_rgb(
                    &gpu,
                    list,
                    4,
                    4,
                    1.0,
                    &skin,
                    crate::gpu_render::atlas_identity(&skin.rgba, 1, 1),
                    &bindings,
                    &transform,
                    percent.map(|_| (&bg, full, crate::gpu_render::atlas_identity(&bg.rgba, 1, 1))),
                    None,
                    None,
                )
                .unwrap()
            };
            let (_, background_rgba) =
                crate::gpu_render::capture_rgba(|| render(&crate::runtime::DisplayList::default()));
            assert!(background_rgba.chunks_exact(4).all(|p| p[3] == 255));
            let (rgb, rgba) = crate::gpu_render::capture_rgba(|| render(&list));
            assert!(rgba.chunks_exact(4).all(|p| p[3] == 255));
            let alpha = 128.0 / 255.0 * 0.5;
            let expected: [u8; 3] = std::array::from_fn(|c| {
                (f64::from(presented[c]) * (1.0 - alpha) + f64::from(skin.rgba[c]) * alpha).round()
                    as u8
            });
            assert!(cpu.chunks_exact(3).all(|p| p == expected));
            assert!(rgb
                .chunks_exact(3)
                .all(|p| p.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 1)));
            assert_eq!(list.sprites[0].alpha, 0.5);
            assert_eq!(skin.rgba[3], 128);
        }
    }

    pub fn tools() -> Option<FfmpegInstallation> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("dependencies/ffmpeg/8.1");
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        let tools = FfmpegInstallation {
            source: crate::ffmpeg::FfmpegSource::Managed,
            ffmpeg: root.join(format!("ffmpeg{suffix}")),
            ffprobe: root.join(format!("ffprobe{suffix}")),
        };
        if !tools.ffmpeg.is_file() {
            eprintln!("MV integration test skipped: managed FFmpeg not installed");
            return None;
        }
        Some(tools)
    }

    // Each source frame carries an exact PTS label and a distinct color. FFV1
    // preserves pixels, so decoder/frame-selection assertions need no codec tolerance.
    pub fn fixture(tools: &FfmpegInstallation, variable: bool) -> tempfile::TempDir {
        fixture_at_fps(tools, variable, 4)
    }

    fn fixture_at_fps(tools: &FfmpegInstallation, variable: bool, fps: u32) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(if fps == 30 {
            "timestamped.mov"
        } else {
            "timestamped.mkv"
        });
        let mut command = Command::new(&tools.ffmpeg);
        crate::export_control::hide_child_window(&mut command);
        command.args([
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            "32x16",
            "-framerate",
            &fps.to_string(),
            "-i",
            "pipe:0",
            "-an",
        ]);
        if variable {
            command.args([
                "-vf",
                r"settb=1/1000,setpts=if(eq(N\,0)\,0\,if(eq(N\,1)\,125\,if(eq(N\,2)\,500\,875)))",
                "-enc_time_base",
                "1:1000",
            ]);
        }
        command.args(["-fps_mode", "passthrough"]);
        if fps == 30 {
            command.args(["-c:v", "png", "-video_track_timescale", "30000"]);
        } else {
            command.args(["-c:v", "ffv1"]);
        }
        let mut child = command
            .arg(path)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let digits = [
            [7, 5, 5, 5, 7],
            [2, 6, 2, 2, 7],
            [7, 1, 7, 4, 7],
            [7, 1, 7, 1, 7],
            [5, 5, 7, 1, 1],
            [7, 4, 7, 1, 7],
            [7, 4, 7, 5, 7],
            [7, 1, 1, 1, 1],
            [7, 5, 7, 5, 7],
            [7, 5, 7, 1, 7],
        ];
        let mut stdin = child.stdin.take().unwrap();
        for (i, pts) in (if variable {
            [0, 125, 500, 875]
        } else {
            [0, 1000 / fps, 2000 / fps, 3000 / fps]
        })
        .into_iter()
        .enumerate()
        {
            let color = [
                [220, 20, 30, 255],
                [20, 220, 30, 255],
                [20, 30, 220, 255],
                [220, 220, 20, 255],
            ][i];
            let mut rgba = color.repeat(32 * 16);
            for (n, digit) in format!("{pts:03}").bytes().enumerate() {
                for y in 0..5 {
                    for x in 0..3 {
                        if digits[(digit - b'0') as usize][y] & (1 << (2 - x)) != 0 {
                            let at = ((y + 4) * 32 + 4 + n * 4 + x) * 4;
                            rgba[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
                        }
                    }
                }
            }
            stdin.write_all(&rgba).unwrap();
        }
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        dir
    }

    #[test]
    fn mv_percentage_composes_before_opaque_gameplay_on_cpu_and_wgpu() {
        let Some(tools) = tools() else { return };
        let fixture = fixture(&tools, false);
        let gpu = crate::gpu_render::GpuRenderer::shared().unwrap();
        let transform = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let mut sprite_transform = [[0.0; 8]; 8];
        for (i, row) in sprite_transform.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        let skin = formats::SkinAssets {
            width: 1,
            height: 1,
            interpolation: false,
            rgba: vec![19, 83, 227, 255],
            sprites: [(
                "test".into(),
                formats::SkinSpriteAsset {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    transform: sprite_transform,
                },
            )]
            .into(),
        };
        let bindings = [(0, "test".into())].into();
        let display = crate::runtime::DisplayList {
            sprites: vec![crate::runtime::SpriteDraw {
                sprite_id: 0,
                corners: [[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]],
                z: [0.0; 4],
                alpha: 1.0,
                provenance: None,
                trace: None,
            }],
        };
        let mut original_pixel = None;
        for percent in [100.0, 40.0, 0.0] {
            let range = crate::offline::FrameRange::new(0.0, 0.25, 8).unwrap();
            let mut decoder = MvDecoder::new(
                &tools,
                &fixture.path().join("timestamped.mkv"),
                ExportTimeline::new(0.0, range, 0.0).unwrap(),
                8,
                8,
                formats::BackgroundConfiguration {
                    blur: 0.0,
                    mask: "#0000".into(),
                },
            )
            .unwrap();
            decoder.set_background_percentage(percent);
            let frame = decoder.at_watch_time(0.0).unwrap().unwrap();
            let cpu = display
                .render_skin_rgb_with_runtime_transform_and_background(
                    8,
                    8,
                    1.0,
                    &skin,
                    &bindings,
                    &transform,
                    Some((&frame.assets, frame.quad)),
                )
                .unwrap();
            let gpu = crate::gpu_render::render_display_list_rgb(
                &gpu,
                &display,
                8,
                8,
                1.0,
                &skin,
                crate::gpu_render::atlas_identity(&skin.rgba, 1, 1),
                &bindings,
                &transform,
                Some((&frame.assets, frame.quad, frame.identity)),
                None,
                None,
            )
            .unwrap();
            assert_eq!(&cpu[(4 * 8 + 4) * 3..(4 * 8 + 4) * 3 + 3], &[19, 83, 227]);
            assert_eq!(&gpu[(4 * 8 + 4) * 3..(4 * 8 + 4) * 3 + 3], &[19, 83, 227]);
            for (&a, &b) in cpu.iter().zip(&gpu) {
                assert!(a.abs_diff(b) <= 2);
            }
            // Left edge away from skin is the dimmed MV, never the gameplay layer.
            let pixel = &cpu[(4 * 8) * 3..(4 * 8) * 3 + 3];
            if let Some(base) = &original_pixel {
                let base: &Vec<u8> = base;
                for (&a, &b) in pixel.iter().zip(base) {
                    assert!(a.abs_diff((f64::from(b) * percent / 100.0).round() as u8) <= 1);
                }
            } else {
                original_pixel = Some(pixel.to_vec());
            }
        }
    }

    #[test]
    fn mv_brightness_preserves_pts_alpha_quad_and_selected_frames() {
        let Some(tools) = tools() else { return };
        let fixture = fixture(&tools, false);
        let range = crate::offline::FrameRange::new(0.0, 1.25, 8).unwrap();
        let timeline = ExportTimeline::new(0.0, range, -0.125).unwrap();
        let mut baseline = None;
        for percent in [100.0, 40.0, 0.0] {
            let mut decoder = MvDecoder::new(
                &tools,
                &fixture.path().join("timestamped.mkv"),
                timeline,
                64,
                36,
                formats::BackgroundConfiguration {
                    blur: 0.0,
                    mask: "#0000".into(),
                },
            )
            .unwrap();
            decoder.set_background_percentage(percent);
            let frames: Vec<_> = (0..range.frame_count)
                .map(|i| {
                    decoder
                        .at_watch_time(range.time(range.index(i).unwrap()))
                        .unwrap()
                })
                .collect();
            if let Some(reference) = &baseline {
                let reference: &Vec<Option<Arc<MvFrame>>> = reference;
                for (a, b) in frames.iter().zip(reference) {
                    assert_eq!(a.is_some(), b.is_some());
                    if let (Some(a), Some(b)) = (a, b) {
                        assert_eq!(a.media_pts, b.media_pts);
                        assert_eq!(a.quad, b.quad);
                        for (x, y) in a
                            .assets
                            .rgba
                            .chunks_exact(4)
                            .zip(b.assets.rgba.chunks_exact(4))
                        {
                            assert_eq!(x[3], y[3]);
                            for c in 0..3 {
                                assert_eq!(x[c], (f64::from(y[c]) * percent / 100.0).round() as u8);
                            }
                        }
                    }
                }
            } else {
                baseline = Some(frames);
            }
        }
    }

    #[test]
    fn mv_samples_canonical_media_time_with_offset_preroll_and_fps_mismatch() {
        let Some(tools) = tools() else { return };
        let fixture = fixture(&tools, false);
        for (start, offset, fps) in [
            (0.0, 0.0, 8),
            (0.0, -0.125, 8),
            (0.0, 0.125, 8),
            (0.13, 0.125, 8),
            (0.0, 0.0, 2),
            (2.0, 0.0, 8),
            (0.0, -2.0, 8),
        ] {
            let range = crate::offline::FrameRange::new(start, 1.5, fps).unwrap();
            let timeline = ExportTimeline::new(start, range, offset).unwrap();
            let mut decoder = MvDecoder::new(
                &tools,
                &fixture.path().join("timestamped.mkv"),
                timeline,
                64,
                36,
                formats::BackgroundConfiguration {
                    blur: 0.0,
                    mask: "#0000".into(),
                },
            )
            .unwrap();
            for n in 0..range.frame_count {
                let watch = range.time(range.index(n).unwrap());
                let media = timeline.output_to_media(n as f64 / f64::from(fps));
                let frame = decoder.at_watch_time(watch).unwrap();
                if !(0.0..1.0).contains(&media) {
                    assert!(frame.is_none());
                    continue;
                }
                let frame = frame.unwrap();
                let i = (media * 4.0).floor() as usize;
                assert_eq!(frame.media_pts, i as f64 / 4.0);
                assert_eq!(
                    &frame.assets.rgba[..4],
                    &[
                        [220, 20, 30, 255],
                        [20, 220, 30, 255],
                        [20, 30, 220, 255],
                        [220, 220, 20, 255]
                    ][i]
                );
                assert_eq!(frame.assets.width, 64);
                assert_eq!(frame.assets.height, 32);
                assert_eq!(frame.quad[0], [-64.0 / 36.0, -32.0 / 36.0]);
            }
        }
    }

    #[test]
    fn mv_selects_by_presentation_timestamp_not_frame_index() {
        let Some(tools) = tools() else { return };
        let fixture = fixture(&tools, true);
        let range = crate::offline::FrameRange::new(0.0, 1.25, 8).unwrap();
        let timeline = ExportTimeline::new(0.0, range, 0.0).unwrap();
        let mut decoder = MvDecoder::new(
            &tools,
            &fixture.path().join("timestamped.mkv"),
            timeline,
            64,
            36,
            formats::BackgroundConfiguration {
                blur: 0.0,
                mask: "#0000".into(),
            },
        )
        .unwrap();
        for (time, expected) in [
            (0.0, 0.0),
            (0.124, 0.0),
            (0.125, 0.125),
            (0.375, 0.125),
            (0.5, 0.5),
            (0.874, 0.5),
            (0.875, 0.875),
            (1.124, 0.875),
        ] {
            assert_eq!(
                decoder.at_watch_time(time).unwrap().unwrap().media_pts,
                expected
            );
        }
        assert!(decoder.at_watch_time(1.125).unwrap().is_none());
        assert!(decoder.at_watch_time(1.0).is_err());
        // Preserve rational PTS: rounding 2/30 upward to microseconds would
        // incorrectly keep frame 1 when the requested time is exactly 2/30.
        let fixture = fixture_at_fps(&tools, false, 30);
        let range = crate::offline::FrameRange::new(0.0, 0.2, 30).unwrap();
        let mut decoder = MvDecoder::new(
            &tools,
            &fixture.path().join("timestamped.mov"),
            ExportTimeline::new(0.0, range, 0.0).unwrap(),
            64,
            36,
            formats::BackgroundConfiguration {
                blur: 0.0,
                mask: "#0000".into(),
            },
        )
        .unwrap();
        for n in 0..4 {
            let time = n as f64 / 30.0;
            let pts = time;
            assert!((decoder.at_watch_time(pts).unwrap().unwrap().media_pts - pts).abs() < 1e-9);
        }
    }
}

/// Present video as an opaque RGB background, dimmed toward black.
/// Decoded alpha is not a presentation opacity or a gameplay blend input.
fn scale_background(rgba: &mut [u8], percent: f64) {
    for pixel in rgba.chunks_exact_mut(4) {
        if percent != 100.0 {
            for channel in &mut pixel[..3] {
                *channel = (f64::from(*channel) * percent / 100.0).round() as u8;
            }
        }
        pixel[3] = 255;
    }
}
