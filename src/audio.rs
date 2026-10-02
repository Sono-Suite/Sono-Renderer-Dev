//! Small deterministic PCM mixer for Sonolus EffectData clips used by offline video export.

use crate::{
    formats::EffectAssets,
    runtime::{
        AudioEffectEvent, ScheduledEffect, ScheduledLoopedEffect, ScheduledLoopedEffectStop,
    },
};
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SfxMixResult {
    pub wav: Option<Vec<u8>>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct PcmClip {
    rate: u32,
    channels: usize,
    samples: Vec<f32>,
}

/// Mix supported Watch play and loop events into stereo PCM16 WAV at 44.1 kHz.
/// Clip encodings outside uncompressed PCM16 mono/stereo are rejected explicitly.
pub fn mix_effects_wav(
    assets: &EffectAssets,
    bindings: &BTreeMap<i64, String>,
    events: &[AudioEffectEvent],
    scheduled: &[ScheduledEffect],
    loop_starts: &[ScheduledLoopedEffect],
    loop_stops: &[ScheduledLoopedEffectStop],
    timeline_start: f64,
    duration: f64,
) -> Result<SfxMixResult> {
    if !timeline_start.is_finite() || !duration.is_finite() || duration <= 0.0 {
        bail!("SFX mix timeline must be finite and duration positive");
    }
    let mut decoded = BTreeMap::new();
    let mut active = Vec::<(String, f64, Option<f64>, f64, bool)>::new();
    for event in events {
        match *event {
            AudioEffectEvent::Play {
                clip_id,
                minimum_distance,
                requested_at,
            } => {
                if !minimum_distance.is_finite() || !requested_at.is_finite() {
                    bail!("immediate effect event contains a non-finite parameter");
                }
                let name = resolve(bindings, clip_id)?;
                active.push((name, requested_at, None, minimum_distance, false));
            }
            AudioEffectEvent::StartLoop { .. } | AudioEffectEvent::StopLoop { .. } => {}
        }
    }
    for event in scheduled {
        if !event.time.is_finite()
            || !event.requested_at.is_finite()
            || !event.minimum_distance.is_finite()
        {
            bail!("scheduled effect event contains a non-finite parameter");
        }
        let name = resolve(bindings, event.clip_id)?;
        active.push((name, event.time, None, event.minimum_distance, false));
    }
    for start in loop_starts {
        if !start.start_time.is_finite() || !start.requested_at.is_finite() {
            bail!("scheduled loop start contains a non-finite parameter");
        }
        let name = resolve(bindings, start.clip_id)?;
        let end = loop_stops
            .iter()
            .find(|stop| stop.instance_id == start.instance_id)
            .map(|stop| stop.end_time);
        active.push((name, start.start_time, end, start.instance_id as f64, true));
    }
    // Pair immediate loop stops with starts by instance handle; event records preserve order.
    let mut immediate = HashMap::<i64, (String, f64)>::new();
    for event in events {
        match event {
            AudioEffectEvent::StartLoop {
                instance_id,
                clip_id,
                requested_at,
            } => {
                if !requested_at.is_finite() {
                    bail!("immediate loop start time must be finite");
                }
                immediate.insert(*instance_id, (resolve(bindings, *clip_id)?, *requested_at));
            }
            AudioEffectEvent::StopLoop {
                instance_id,
                requested_at,
            } => {
                if !requested_at.is_finite() {
                    bail!("immediate loop stop time must be finite");
                }
                if let Some((name, start)) = immediate.remove(instance_id) {
                    active.push((name, start, Some(*requested_at), *instance_id as f64, true));
                }
            }
            _ => {}
        }
    }
    active.extend(
        immediate
            .into_iter()
            .map(|(id, (name, start))| (name, start, None, id as f64, true)),
    );
    if active.is_empty() {
        return Ok(SfxMixResult {
            wav: None,
            warnings: Vec::new(),
        });
    }

    let sample_rate = 44_100_u32;
    let frames = (duration * f64::from(sample_rate)).ceil() as usize;
    if frames > sample_rate as usize * 60 * 30 {
        bail!("offline SFX mixing is limited to 30 minutes per export");
    }
    let mut mix = vec![[0.0_f32; 2]; frames];
    let mut last_play = BTreeMap::<String, f64>::new();
    let mut warned = BTreeSet::<String>::new();
    let mut warnings = Vec::new();
    let mut has_playable_event = false;
    active.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (name, start, end, minimum_distance, looping) in active {
        let Some(source_start) = start
            .max(timeline_start)
            .partial_cmp(&(timeline_start + duration))
            .filter(|o| *o == std::cmp::Ordering::Less)
            .map(|_| start)
        else {
            continue;
        };
        if let Some(previous) = last_play.get(&name) {
            if !looping && source_start - *previous < minimum_distance.max(0.0) {
                continue;
            }
        }
        last_play.insert(name.clone(), source_start);
        if !decoded.contains_key(&name) {
            let payload = assets
                .clips
                .get(&name)
                .and_then(Option::as_ref)
                .filter(|bytes| !bytes.is_empty());
            let Some(bytes) = payload else {
                if warned.insert(name.clone()) {
                    warnings.push(format!(
                        "SFX clip {name:?} is unavailable (missing resource binding or audio payload); skipping its events"
                    ));
                }
                continue;
            };
            decoded.insert(
                name.clone(),
                decode_wav(bytes).with_context(|| format!("decoding SFX clip {name:?}"))?,
            );
        }
        let clip = decoded.get(&name).unwrap();
        has_playable_event = true;
        let source_frame = ((source_start - start).max(0.0) * f64::from(clip.rate)) as usize;
        let output_frame = ((start - timeline_start).max(0.0) * f64::from(sample_rate)) as usize;
        let until = end
            .unwrap_or(timeline_start + duration)
            .min(timeline_start + duration);
        let max_output = ((until - timeline_start).max(0.0) * f64::from(sample_rate)) as usize;
        let source_frames = clip.samples.len() / clip.channels;
        let mut out = output_frame;
        let source_step = f64::from(clip.rate) / f64::from(sample_rate);
        while out < frames && out < max_output {
            let position = source_frame as f64 + (out - output_frame) as f64 * source_step;
            let mut src = position.floor() as usize;
            if looping {
                src %= source_frames.max(1);
            } else if src >= source_frames {
                break;
            }
            let next = if looping {
                (src + 1) % source_frames.max(1)
            } else {
                (src + 1).min(source_frames.saturating_sub(1))
            };
            let fraction = (position.fract()) as f32;
            let left0 = clip.samples[src * clip.channels];
            let right0 = if clip.channels == 2 {
                clip.samples[src * 2 + 1]
            } else {
                left0
            };
            let left1 = clip.samples[next * clip.channels];
            let right1 = if clip.channels == 2 {
                clip.samples[next * 2 + 1]
            } else {
                left1
            };
            let left = left0 + (left1 - left0) * fraction;
            let right = right0 + (right1 - right0) * fraction;
            mix[out][0] += left;
            mix[out][1] += right;
            out += 1;
        }
    }
    if !has_playable_event {
        return Ok(SfxMixResult {
            wav: None,
            warnings,
        });
    }
    let mut pcm = Vec::with_capacity(frames * 4);
    for frame in mix {
        for channel in frame {
            let value = channel.clamp(-1.0, 1.0);
            pcm.extend_from_slice(&((value * 32767.0).round() as i16).to_le_bytes());
        }
    }
    Ok(SfxMixResult {
        wav: Some(wav_header(sample_rate, 2, &pcm)),
        warnings,
    })
}

fn resolve(bindings: &BTreeMap<i64, String>, id: i64) -> Result<String> {
    Ok(bindings
        .get(&id)
        .cloned()
        .unwrap_or_else(|| format!("<unbound clip ID {id}>")))
}

fn decode_wav(bytes: &[u8]) -> Result<PcmClip> {
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("unsupported audio: expected RIFF/WAVE");
    }
    let mut offset = 12;
    let mut format = None;
    let mut data = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let begin = offset + 8;
        let end = begin
            .checked_add(size)
            .context("WAV chunk length overflow")?;
        if end > bytes.len() {
            bail!("truncated WAV chunk");
        }
        if id == b"fmt " && size >= 16 {
            format = Some((
                u16::from_le_bytes(bytes[begin..begin + 2].try_into().unwrap()),
                u16::from_le_bytes(bytes[begin + 2..begin + 4].try_into().unwrap()),
                u32::from_le_bytes(bytes[begin + 4..begin + 8].try_into().unwrap()),
                u16::from_le_bytes(bytes[begin + 14..begin + 16].try_into().unwrap()),
            ));
        } else if id == b"data" {
            data = Some(&bytes[begin..end]);
        }
        offset = end + (size & 1);
    }
    let (format, channels, rate, bits) = format.context("WAV has no fmt chunk")?;
    if format != 1 || !matches!(channels, 1 | 2) || bits != 16 {
        bail!("unsupported WAV: require PCM16 mono or stereo");
    }
    if rate == 0 {
        bail!("WAV sample rate must be positive");
    }
    let raw = data.context("WAV has no data chunk")?;
    if raw.len() % 2 != 0 {
        bail!("WAV PCM16 payload has odd byte count");
    }
    let samples: Vec<f32> = raw
        .chunks_exact(2)
        .map(|chunk| f32::from(i16::from_le_bytes([chunk[0], chunk[1]])) / 32768.0)
        .collect();
    if samples.is_empty() || samples.len() % channels as usize != 0 {
        bail!("WAV contains no complete PCM frames");
    }
    Ok(PcmClip {
        rate,
        channels: channels as usize,
        samples,
    })
}

fn wav_header(rate: u32, channels: u16, pcm: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36_u32 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_compressed_or_non_pcm_effect_clip_formats() {
        assert!(decode_wav(b"OggS")
            .unwrap_err()
            .to_string()
            .contains("RIFF/WAVE"));
    }

    #[test]
    fn mixes_play_and_loop_requests_and_requires_declared_clip_binding() {
        let mut data = Vec::new();
        for _ in 0..441 {
            data.extend_from_slice(&12000_i16.to_le_bytes());
        }
        let wav = wav_header(44_100, 1, &data);
        let assets = EffectAssets {
            clips: [("hit".to_owned(), Some(wav))].into(),
        };
        let bindings = [(4, "hit".to_owned())].into();
        let events = [AudioEffectEvent::Play {
            clip_id: 4,
            minimum_distance: 0.0,
            requested_at: 0.0,
        }];
        let mixed = mix_effects_wav(&assets, &bindings, &events, &[], &[], &[], 0.0, 0.1)
            .unwrap()
            .wav
            .unwrap();
        assert_eq!(&mixed[..4], b"RIFF");
        assert!(mixed[44..].iter().any(|byte| *byte != 0));
        let looped = mix_effects_wav(
            &assets,
            &bindings,
            &[],
            &[],
            &[ScheduledLoopedEffect {
                instance_id: 8,
                clip_id: 4,
                start_time: 0.0,
                requested_at: 0.0,
                has_required_lead_time: true,
            }],
            &[ScheduledLoopedEffectStop {
                instance_id: 8,
                end_time: 0.05,
                requested_at: 0.0,
                has_required_lead_time: true,
            }],
            0.0,
            0.1,
        )
        .unwrap()
        .wav
        .unwrap();
        assert!(looped[44..44 + (0.05_f64 * 44_100.0) as usize * 4]
            .iter()
            .any(|byte| *byte != 0));
        assert!(looped[44 + (0.06_f64 * 44_100.0) as usize * 4..]
            .iter()
            .all(|byte| *byte == 0));
        let immediate_loop = mix_effects_wav(
            &assets,
            &bindings,
            &[
                AudioEffectEvent::StartLoop {
                    instance_id: 12,
                    clip_id: 4,
                    requested_at: 0.0,
                },
                AudioEffectEvent::StopLoop {
                    instance_id: 12,
                    requested_at: 0.05,
                },
            ],
            &[],
            &[],
            &[],
            0.0,
            0.1,
        )
        .unwrap()
        .wav
        .unwrap();
        assert!(immediate_loop[44..44 + (0.05_f64 * 44_100.0) as usize * 4]
            .iter()
            .any(|byte| *byte != 0));
        assert!(immediate_loop[44 + (0.06_f64 * 44_100.0) as usize * 4..]
            .iter()
            .all(|byte| *byte == 0));
        let unavailable =
            mix_effects_wav(&assets, &BTreeMap::new(), &events, &[], &[], &[], 0.0, 0.1).unwrap();
        assert!(unavailable.wav.is_none());
        assert_eq!(unavailable.warnings.len(), 1);
    }

    #[test]
    fn missing_and_empty_payload_clips_warn_once_and_valid_clip_still_mixes() {
        let mut data = Vec::new();
        for _ in 0..441 {
            data.extend_from_slice(&9000_i16.to_le_bytes());
        }
        let assets = EffectAssets {
            clips: [
                ("valid".to_owned(), Some(wav_header(44_100, 1, &data))),
                ("empty".to_owned(), None),
            ]
            .into(),
        };
        let bindings = [
            (1, "valid".to_owned()),
            (2, "empty".to_owned()),
            (3, "absent".to_owned()),
        ]
        .into();
        let events = [
            AudioEffectEvent::Play {
                clip_id: 2,
                minimum_distance: 0.0,
                requested_at: 0.0,
            },
            AudioEffectEvent::Play {
                clip_id: 2,
                minimum_distance: 0.0,
                requested_at: 0.01,
            },
            AudioEffectEvent::Play {
                clip_id: 3,
                minimum_distance: 0.0,
                requested_at: 0.0,
            },
            AudioEffectEvent::Play {
                clip_id: 1,
                minimum_distance: 0.0,
                requested_at: 0.0,
            },
        ];
        let result = mix_effects_wav(&assets, &bindings, &events, &[], &[], &[], 0.0, 0.1).unwrap();
        let wav = result.wav.unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert!(wav[44..].iter().any(|byte| *byte != 0));
        assert_eq!(result.warnings.len(), 2);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("empty")));
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("absent")));
    }

    #[test]
    fn malformed_or_unsupported_present_clip_remains_fatal() {
        let assets = EffectAssets {
            clips: [("bad".to_owned(), Some(b"not a wav".to_vec()))].into(),
        };
        let events = [AudioEffectEvent::Play {
            clip_id: 1,
            minimum_distance: 0.0,
            requested_at: 0.0,
        }];
        let error = mix_effects_wav(
            &assets,
            &[(1, "bad".to_owned())].into(),
            &events,
            &[],
            &[],
            &[],
            0.0,
            0.1,
        )
        .unwrap_err();
        assert!(error.to_string().contains("decoding SFX clip"));
    }
}
