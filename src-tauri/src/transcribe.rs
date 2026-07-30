//! whisper.cpp 기반 로컬 전사.
//!
//! 오디오는 전부 이 머신에서 처리한다 — 네트워크로 나가는 게 없다.

use std::path::Path;

use serde::{Deserialize, Serialize};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// whisper가 요구하는 입력 형식.
const TARGET_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    /// 시작 시각(초).
    pub start: f64,
    /// 종료 시각(초).
    pub end: f64,
    pub text: String,
    /// 이 세그먼트 다음에 화자가 바뀌는지. tdrz 계열 모델에서만 채워지고
    /// 그 외 모델에서는 항상 false다.
    pub speaker_turn: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub segments: Vec<Segment>,
    /// 전사 소요 시간(초). 체감 속도를 UI에 보여주는 용도.
    pub elapsed: f64,
}

impl Transcript {
    /// 요약 모델에 넘길 평문. 타임스탬프는 뺀다.
    pub fn plain_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// WAV 파일을 16kHz 모노 f32 샘플로 읽는다.
///
/// 스테레오는 채널 평균으로 모노화한다. 샘플레이트가 다르면 에러 — 리샘플링은
/// 호출부에서 처리할 문제고, 조용히 품질을 떨어뜨리는 것보다 낫다.
fn load_wav(path: &Path) -> Result<Vec<f32>, String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("WAV 파일을 열 수 없습니다: {e}"))?;
    let spec = reader.spec();

    if spec.sample_rate != TARGET_SAMPLE_RATE {
        return Err(format!(
            "샘플레이트가 {}Hz입니다. {}Hz 모노 WAV로 변환해서 넣어주세요.",
            spec.sample_rate, TARGET_SAMPLE_RATE
        ));
    }

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            // 비트 깊이와 무관하게 [-1.0, 1.0]으로 정규화한다.
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("WAV 샘플을 읽을 수 없습니다: {e}"))?
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("WAV 샘플을 읽을 수 없습니다: {e}"))?,
    };

    if spec.channels == 1 {
        return Ok(samples);
    }

    let channels = spec.channels as usize;
    Ok(samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect())
}

/// 오디오 파일을 전사한다.
///
/// `language`가 `None`이면 whisper가 자동 감지한다. 한국어 회의라면 "ko"를
/// 명시하는 쪽이 훨씬 정확하다 — 자동 감지는 짧은 발화에서 자주 틀린다.
///
/// `hint`는 참석자 이름이나 사내 용어처럼 모델이 틀리기 쉬운 고유명사를
/// 미리 알려주는 용도다. whisper는 이걸 직전 문맥으로 취급해서 해당 표기로
/// 유도된다. 없으면 "박대리"가 "박태리"로 나가는 식의 오류가 반복된다.
pub fn transcribe(
    model_path: &Path,
    audio_path: &Path,
    language: Option<&str>,
    hint: Option<&str>,
) -> Result<Transcript, String> {
    if !model_path.exists() {
        return Err(format!(
            "whisper 모델이 없습니다: {}",
            model_path.display()
        ));
    }

    let audio = load_wav(audio_path)?;
    let started = std::time::Instant::now();

    let ctx = WhisperContext::new_with_params(
        model_path.to_str().ok_or("모델 경로에 유효하지 않은 문자가 있습니다")?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| format!("whisper 모델을 불러올 수 없습니다: {e}"))?;

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("whisper 상태를 만들 수 없습니다: {e}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    if let Some(lang) = language {
        params.set_language(Some(lang));
    }
    if let Some(hint) = hint.map(str::trim).filter(|h| !h.is_empty()) {
        params.set_initial_prompt(hint);
    }

    // 물리 코어만 쓴다. 효율 코어까지 붙이면 오히려 느려지는 경우가 있다.
    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).max(1))
        .unwrap_or(4);
    params.set_n_threads(threads as i32);

    // 진행 로그는 UI로 따로 보내므로 stdout은 조용히 둔다.
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state
        .full(params, &audio)
        .map_err(|e| format!("전사에 실패했습니다: {e}"))?;

    let n = state.full_n_segments();
    let mut segments = Vec::with_capacity(n.max(0) as usize);

    for i in 0..n {
        let Some(segment) = state.get_segment(i) else {
            continue;
        };

        // 인식 결과에 유효하지 않은 UTF-8이 섞이는 경우가 드물게 있다. 전체를
        // 실패시키는 것보다 해당 부분만 대체 문자로 두는 쪽이 낫다.
        let text = segment
            .to_str_lossy()
            .map_err(|e| format!("세그먼트 {i} 텍스트를 읽을 수 없습니다: {e}"))?;

        // whisper의 타임스탬프 단위는 10ms다.
        segments.push(Segment {
            start: segment.start_timestamp() as f64 / 100.0,
            end: segment.end_timestamp() as f64 / 100.0,
            text: text.trim().to_string(),
            speaker_turn: segment.next_segment_speaker_turn(),
        });
    }

    Ok(Transcript {
        segments,
        elapsed: started.elapsed().as_secs_f64(),
    })
}
