//! 임의의 음성 파일을 whisper가 먹을 수 있는 형태로 바꾼다.
//!
//! whisper는 16kHz 모노 f32만 받는다. 그런데 사람들이 실제로 가진 녹음은
//! 그런 형태가 아니다 — macOS 음성 메모는 m4a(AAC)로 저장하고, 회의 도구들은
//! mp3나 스테레오 48kHz WAV를 뱉는다. "16kHz 모노 WAV만 됩니다"는 사실상
//! 아무도 못 쓴다는 뜻이다.
//!
//! ffmpeg를 끌어오는 대신 symphonia로 직접 디코딩한다. 순수 Rust라 외부
//! 바이너리를 함께 배포할 필요가 없고, 코드 서명·공증 문제도 안 생긴다.

use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// whisper가 요구하는 샘플레이트.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// 디코딩 결과.
pub struct Decoded {
    /// 16kHz 모노 f32 샘플.
    pub samples: Vec<f32>,
    /// 원본 샘플레이트. 진단용으로 남긴다.
    pub source_rate: u32,
    pub source_channels: usize,
}

impl Decoded {
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / TARGET_SAMPLE_RATE as f64
    }
}

/// 음성 파일을 읽어 16kHz 모노로 변환한다.
///
/// 지원 형태는 symphonia가 켜둔 코덱에 달려 있다 — wav, m4a(AAC/ALAC), mp3,
/// flac, ogg 등. 컨테이너는 확장자로 힌트만 주고 실제 판별은 내용으로 한다.
/// 확장자가 틀려도 열리게 하기 위해서다.
pub fn decode_to_mono_16k(path: &Path) -> Result<Decoded, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("파일을 열 수 없습니다: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| {
            format!(
                "이 파일 형식을 읽을 수 없습니다. wav, m4a, mp3, flac 등을 \
                 지원합니다 ({e})"
            )
        })?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or("파일에 음성 트랙이 없습니다.")?;
    let track_id = track.id;

    let params = match track.codec_params.as_ref() {
        Some(CodecParameters::Audio(p)) => p.clone(),
        _ => return Err("음성 코덱 정보를 읽을 수 없습니다.".into()),
    };

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("이 음성 코덱은 지원하지 않습니다: {e}"))?;

    let mut interleaved: Vec<f32> = Vec::new();
    let mut source_rate = 0u32;
    let mut channels = 0usize;
    let mut chunk: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => return Err(format!("파일을 읽는 중 오류가 났습니다: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = buf.spec();
                source_rate = spec.rate();
                channels = spec.channels().count();
                chunk.clear();
                buf.copy_to_vec_interleaved(&mut chunk);
                interleaved.extend_from_slice(&chunk);
            }
            // 손상된 패킷 하나로 전체를 포기하지 않는다. 긴 녹음에서 앞부분이
            // 조금 깨졌다고 나머지를 못 쓰게 만들 이유가 없다.
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("음성을 디코딩할 수 없습니다: {e}")),
        }
    }

    if interleaved.is_empty() || source_rate == 0 || channels == 0 {
        return Err("음성 데이터를 찾지 못했습니다.".into());
    }

    let mono = downmix(&interleaved, channels);
    let samples = resample(&mono, source_rate, TARGET_SAMPLE_RATE);

    Ok(Decoded {
        samples,
        source_rate,
        source_channels: channels,
    })
}

/// 다채널을 평균해서 모노로 만든다.
fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// 선형 보간 리샘플링.
///
/// 음성 대역에는 충분하다. 고품질 리샘플러를 쓰면 조금 나아지겠지만, whisper의
/// 전처리가 어차피 멜 스펙트로그램이라 체감 차이가 거의 없다.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input[idx];
        let b = *input.get(idx + 1).unwrap_or(&a);
        out.push(a + (b - a) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        // 스테레오 두 프레임: (0.0, 1.0), (1.0, 0.0) → 각각 0.5
        let out = downmix(&[0.0, 1.0, 1.0, 0.0], 2);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    #[test]
    fn downmix_passes_mono_through() {
        assert_eq!(downmix(&[0.1, 0.2], 1), vec![0.1, 0.2]);
    }

    #[test]
    fn resample_halves_length_when_rate_halves() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = resample(&input, 32_000, 16_000);
        assert_eq!(out.len(), 50);
        // 선형 보간이므로 값은 대략 두 배 간격으로 늘어난다.
        assert!((out[10] - 20.0).abs() < 0.001);
    }

    #[test]
    fn resample_is_identity_at_same_rate() {
        let input = vec![0.0, 0.5, 1.0];
        assert_eq!(resample(&input, 16_000, 16_000), input);
    }
}
