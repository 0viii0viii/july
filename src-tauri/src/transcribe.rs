//! whisper.cpp 기반 로컬 전사.
//!
//! 오디오는 전부 이 머신에서 처리한다 — 네트워크로 나가는 게 없다.

use std::collections::BTreeMap;
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

/// 요약 모델에 넘길 평문. 타임스탬프는 뺀다.
///
/// 화자를 알면 발언자별로 묶어서 내보낸다. 이게 없으면 요약 모델은 녹취록을
/// 한 사람이 쭉 말한 것으로 보게 되고, 액션 아이템의 담당자를 붙일 근거가
/// 사라진다. 연속된 같은 화자의 세그먼트는 한 발언으로 합친다.
///
/// `names`는 화자 라벨을 실제 사람으로 바꾸는 표다. 명단에서 화자를 지정해두면
/// "화자 1" 대신 "김서연(팀장)"이 들어가고, 그때부터 요약의 담당자가 번호가
/// 아니라 사람이 된다. 지정하지 않은 화자는 라벨 그대로 나간다 — 모르는 것을
/// 아는 척하지 않는다.
///
/// 두 화자를 같은 사람으로 지정하는 일도 있다. 화자분리가 한 사람을 둘로
/// 쪼개는 경우인데, 그러면 이름이 같아지므로 여기서 자연스럽게 한 발언으로
/// 합쳐진다.
pub fn plain_text(segments: &[Segment], names: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut current: Option<&str> = None;

    for segment in segments {
        let text = segment.text.trim();
        if text.is_empty() {
            continue;
        }
        match segment.speaker.as_deref() {
            Some(label) => {
                let who = names.get(label).map(String::as_str).unwrap_or(label);
                if current != Some(who) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(who);
                    out.push_str(": ");
                    current = Some(who);
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

    // 창 사이의 텍스트 조건화를 끊는다.
    //
    // whisper는 30초 창마다 직전 창의 디코딩 결과를 다음 창의 프롬프트로 넣는다.
    // 무음이나 잡음 구간에서 환각 문장이 하나 나오면 그게 다음 창의 "직전 문맥"이
    // 되어 같은 문장이 창을 넘어 계속 복제된다 — 실제로 40분 회의에서 한 문장이
    // 20분 가까이 반복된 사고가 있었다. `no_context`는 이름과 달리 **호출 사이**의
    // 문맥만 끊고 한 호출 안의 창 사이 조건화는 그대로 두므로 소용이 없다. 이
    // 경로를 실제로 막는 스위치는 n_max_text_ctx다.
    //
    // 부작용: 위의 initial_prompt도 같은 경로로 전달되므로 함께 무력화된다.
    // 어차피 힌트는 첫 창에만 작용했고, 표기 교정은 `terms::correct` 후처리가
    // 전 구간을 맡고 있어 잃는 것이 작다. 힌트 설정은 나중에 whisper-rs가
    // carry_initial_prompt를 노출하면 되살아나도록 남겨둔다.
    params.set_n_max_text_ctx(0);

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

    collapse_repeats(&mut segments);

    Ok(Transcript {
        segments,
        elapsed: started.elapsed().as_secs_f64(),
    })
}

/// 같은 문장이 연달아 반복되는 환각 구간을 하나로 접는다.
///
/// 조건화를 끊어도 창 하나(30초) 안에서의 반복까지 막지는 못하고, 디코더가
/// 어떤 경로로든 루프에 빠지면 결과는 늘 같은 모양이다 — 동일한 문장이 세그먼트
/// 수십 개로 이어진다. 이 상태로 요약까지 가면 회의록이 그 문장으로 도배된다.
///
/// 세 번부터 접는 이유: 사람도 "네. 네." 정도는 연달아 말하지만, 같은 문장을
/// 토씨 하나 안 틀리고 세 번 이상 말하는 일은 사실상 없다. 접을 때는 첫
/// 세그먼트를 남기고 시간 범위만 마지막까지 늘린다 — 구간이 사라진 것처럼
/// 보이면 안 되기 때문이다.
fn collapse_repeats(segments: &mut Vec<Segment>) {
    const MIN_RUN: usize = 3;

    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    let mut i = 0;
    while i < segments.len() {
        let text = segments[i].text.trim();
        let mut j = i + 1;
        while j < segments.len() && segments[j].text.trim() == text && !text.is_empty() {
            j += 1;
        }

        if j - i >= MIN_RUN {
            let mut kept = segments[i].clone();
            kept.end = segments[j - 1].end;
            kept.speaker_turn = segments[j - 1].speaker_turn;
            out.push(kept);
        } else {
            out.extend(segments[i..j].iter().cloned());
        }
        i = j;
    }
    *segments = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(text: &str, speaker: Option<&str>) -> Segment {
        Segment {
            start: 0.0,
            end: 1.0,
            text: text.into(),
            speaker: speaker.map(str::to_string),
            speaker_turn: false,
        }
    }

    /// 화자를 모르면 예전처럼 한 줄로 이어 붙인다.
    #[test]
    fn joins_unlabelled_segments() {
        let segments = vec![segment("가나다", None), segment("라마바", None)];
        assert_eq!(plain_text(&segments, &BTreeMap::new()), "가나다 라마바");
    }

    #[test]
    fn groups_consecutive_segments_by_speaker() {
        let segments = vec![
            segment("가나다", Some("화자 1")),
            segment("라마바", Some("화자 1")),
            segment("사아자", Some("화자 2")),
        ];
        assert_eq!(
            plain_text(&segments, &BTreeMap::new()),
            "화자 1: 가나다 라마바\n화자 2: 사아자"
        );
    }

    /// 명단에서 지정한 화자는 이름으로 나가야 한다. 요약의 담당자 배정이
    /// 번호에서 사람으로 바뀌는 지점이 여기다.
    #[test]
    fn substitutes_assigned_names() {
        let segments = vec![
            segment("가나다", Some("화자 1")),
            segment("라마바", Some("화자 2")),
        ];
        let names = BTreeMap::from([("화자 1".to_string(), "김서연(팀장)".to_string())]);
        assert_eq!(
            plain_text(&segments, &names),
            "김서연(팀장): 가나다\n화자 2: 라마바",
            "지정하지 않은 화자는 라벨 그대로 둔다"
        );
    }

    /// 화자분리가 한 사람을 둘로 쪼개는 일이 있다. 둘 다 같은 사람으로
    /// 지정하면 한 발언으로 합쳐져야 한다 — 같은 이름이 두 줄로 나뉘면
    /// 모델이 주고받은 대화로 읽는다.
    #[test]
    fn merges_speakers_assigned_to_same_person() {
        let segments = vec![
            segment("가나다", Some("화자 1")),
            segment("라마바", Some("화자 3")),
        ];
        let names = BTreeMap::from([
            ("화자 1".to_string(), "김서연".to_string()),
            ("화자 3".to_string(), "김서연".to_string()),
        ]);
        assert_eq!(plain_text(&segments, &names), "김서연: 가나다 라마바");
    }

    fn timed(text: &str, start: f64, end: f64) -> Segment {
        Segment {
            start,
            end,
            text: text.into(),
            speaker: None,
            speaker_turn: false,
        }
    }

    /// 환각 루프의 전형 — 같은 문장이 수십 개 세그먼트로 이어진다. 하나로
    /// 접히고 시간 범위는 끝까지 보존되어야 한다.
    #[test]
    fn collapses_hallucination_runs() {
        let mut segments: Vec<Segment> = (0..40)
            .map(|i| timed("시청해 주셔서 감사합니다.", i as f64, i as f64 + 1.0))
            .collect();
        segments.push(timed("이제 본론으로 갑시다.", 40.0, 42.0));

        collapse_repeats(&mut segments);

        assert_eq!(segments.len(), 2, "반복이 접히지 않았다");
        assert_eq!(segments[0].start, 0.0);
        assert_eq!(segments[0].end, 40.0, "접힌 구간의 시간 범위가 줄었다");
        assert_eq!(segments[1].text, "이제 본론으로 갑시다.");
    }

    /// 두 번 연달아 말하는 건 실제 대화에서 흔하다. 건드리면 안 된다.
    #[test]
    fn keeps_natural_double_repeats() {
        let mut segments = vec![
            timed("네.", 0.0, 1.0),
            timed("네.", 1.0, 2.0),
            timed("알겠습니다.", 2.0, 3.0),
        ];
        collapse_repeats(&mut segments);
        assert_eq!(segments.len(), 3, "자연스러운 반복까지 접었다");
    }

    /// 빈 세그먼트끼리는 같은 텍스트라도 접지 않는다 — 반복이 아니라 무음이다.
    #[test]
    fn does_not_collapse_empty_segments() {
        let mut segments = vec![
            timed("", 0.0, 1.0),
            timed("", 1.0, 2.0),
            timed("", 2.0, 3.0),
            timed("본론", 3.0, 4.0),
        ];
        collapse_repeats(&mut segments);
        assert_eq!(segments.len(), 4);
    }

    /// 같은 문장이라도 사이에 다른 말이 끼면 별개의 발언이다.
    #[test]
    fn separated_repeats_survive() {
        let mut segments = vec![
            timed("좋습니다.", 0.0, 1.0),
            timed("일정 얘기로 넘어가죠.", 1.0, 2.0),
            timed("좋습니다.", 2.0, 3.0),
        ];
        collapse_repeats(&mut segments);
        assert_eq!(segments.len(), 3);
    }

    /// 빈 세그먼트가 발언을 끊지 않는다. 끊기면 한 사람의 말이 여러 줄로
    /// 쪼개져 나간다.
    #[test]
    fn empty_segments_do_not_break_a_turn() {
        let segments = vec![
            segment("가나다", Some("화자 1")),
            segment("   ", Some("화자 2")),
            segment("사아자", Some("화자 1")),
        ];
        assert_eq!(
            plain_text(&segments, &BTreeMap::new()),
            "화자 1: 가나다 사아자"
        );
    }
}
