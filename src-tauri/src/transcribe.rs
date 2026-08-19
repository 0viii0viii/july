//! whisper.cpp 기반 로컬 전사.
//!
//! 오디오는 전부 이 머신에서 처리한다 — 네트워크로 나가는 게 없다.

use std::path::Path;

use serde::{Deserialize, Serialize};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    /// 시작 시각(초).
    pub start: f64,
    /// 종료 시각(초).
    pub end: f64,
    pub text: String,
    /// 이 발언의 화자. `diarize`가 채운다. 화자분리를 돌리지 않았거나 화자
    /// 구간에 걸치지 않으면 `None`이다 — 모르면 지어내지 않는다.
    #[serde(default)]
    pub speaker: Option<String>,
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
    ///
    /// 화자를 알면 발언자별로 묶어서 내보낸다. 이게 없으면 요약 모델은 녹취록을
    /// 한 사람이 쭉 말한 것으로 보게 되고, 액션 아이템의 담당자를 붙일 근거가
    /// 사라진다. 연속된 같은 화자의 세그먼트는 한 발언으로 합친다.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        let mut current: Option<&str> = None;

        for segment in &self.segments {
            let text = segment.text.trim();
            if text.is_empty() {
                continue;
            }
            match segment.speaker.as_deref() {
                Some(speaker) => {
                    if current != Some(speaker) {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(speaker);
                        out.push_str(": ");
                        current = Some(speaker);
                    } else {
                        out.push(' ');
                    }
                }
                // 화자를 모르면 예전처럼 이어 붙인다.
                None => {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    current = None;
                }
            }
            out.push_str(text);
        }
        out
    }
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

    let audio = crate::decode::decode_to_mono_16k(audio_path)?.samples;
    transcribe_samples(model_path, &audio, language, hint)
}

/// 이미 디코딩된 16kHz 모노 샘플을 전사한다.
///
/// 화자분리가 같은 샘플을 다시 쓰기 때문에 디코딩을 호출부로 끌어냈다. 같은
/// 파일을 두 번 디코딩하면 긴 회의에서 그만큼 더 기다린다.
pub fn transcribe_samples(
    model_path: &Path,
    audio: &[f32],
    language: Option<&str>,
    hint: Option<&str>,
) -> Result<Transcript, String> {
    if !model_path.exists() {
        return Err(format!("whisper 모델이 없습니다: {}", model_path.display()));
    }
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
        .full(params, audio)
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
            speaker: None,
            speaker_turn: segment.next_segment_speaker_turn(),
        });
    }

    Ok(Transcript {
        segments,
        elapsed: started.elapsed().as_secs_f64(),
    })
}
