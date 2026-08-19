//! 화자분리 — 누가 언제 말했는지.
//!
//! whisper는 무엇을 말했는지만 알려준다. 세그먼트의 `speaker_turn` 필드는
//! tinydiarize 계열(영어 전용 small 모델)에서만 채워져서 한국어 회의에는
//! 쓸 수 없다. 그래서 화자는 별도 파이프라인으로 구한다.
//!
//! speakrs가 pyannote community-1 파이프라인을 Rust로 구현한 것을 쓴다. 음성이
//! 기기 밖으로 나가지 않는다는 전제는 그대로다 — 전부 로컬 추론이다. CoreML로
//! 돌아서 19분 회의에 10초가 걸린다(실시간 대비 116배).
//!
//! **Apple Silicon에서만 동작한다.** ort-sys가 x86_64-apple-darwin용 사전 빌드
//! 바이너리를 내놓지 않아 유니버설 빌드의 인텔 아치가 깨지고, 윈도우는 MKL
//! 정적과 ONNX 정적이 겹쳐 rustc가 내부 패닉을 낸다. 그 외 플랫폼에서는
//! `diarize`가 에러를 돌려주고 호출부는 화자 없이 진행한다 — 화자 없는
//! 회의록이 회의록이 없는 것보다 낫다.

use std::path::{Path, PathBuf};

use crate::transcribe::Segment;

/// 한 화자가 연속으로 말한 구간.
#[derive(Debug, Clone)]
pub struct Turn {
    pub start: f64,
    pub end: f64,
    pub speaker: String,
}

/// 이보다 짧은 구간은 버린다.
///
/// 원시 출력에는 0.1초짜리 조각이 잔뜩 섞인다 — 맞장구나 숨소리가 다른 화자로
/// 튀는 것이다. 그대로 두면 한 문장 안에서 화자가 몇 번씩 바뀐 것처럼 보인다.
const MIN_TURN: f64 = 0.4;

/// 같은 화자의 두 구간 사이가 이보다 좁으면 하나로 잇는다.
///
/// 말하다 숨 쉬는 정도의 간격이다. 이어붙여야 요약 모델에 넘길 때 발언이
/// 토막나지 않는다.
const MERGE_GAP: f64 = 0.8;

/// 화자 이름을 UI에 보일 형태로. speakrs는 `SPEAKER_00`을 준다.
fn label(raw: &str) -> String {
    raw.rsplit('_')
        .next()
        .and_then(|n| n.parse::<u32>().ok())
        .map(|n| format!("화자 {}", n + 1))
        .unwrap_or_else(|| raw.to_string())
}

/// 짧은 조각을 버리고 같은 화자의 인접 구간을 잇는다.
fn smooth(mut turns: Vec<Turn>) -> Vec<Turn> {
    turns.retain(|t| t.end - t.start >= MIN_TURN);
    turns.sort_by(|a, b| a.start.total_cmp(&b.start));

    let mut out: Vec<Turn> = Vec::with_capacity(turns.len());
    for turn in turns {
        match out.last_mut() {
            Some(prev) if prev.speaker == turn.speaker && turn.start - prev.end <= MERGE_GAP => {
                prev.end = prev.end.max(turn.end);
            }
            _ => out.push(turn),
        }
    }
    out
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod imp {
    use super::{smooth, label, Turn};
    use speakrs::{ExecutionMode, OwnedDiarizationPipeline};
    use std::path::{Path, PathBuf};

    /// whisper의 Metal/CPU 분기와 같은 이유로 CoreML을 쓴다.
    const MODE: ExecutionMode = ExecutionMode::CoreMl;

    pub fn ensure_models(models_dir: &Path) -> Result<PathBuf, String> {
        let cache = models_dir.join("diarize");
        speakrs::ModelManager::with_cache_dir(cache)
            .map_err(|e| format!("화자분리 모델 저장소를 열 수 없습니다: {e}"))?
            .ensure(MODE)
            .map_err(|e| format!("화자분리 모델을 내려받을 수 없습니다: {e}"))
    }

    pub fn diarize(samples: &[f32], models_dir: &Path) -> Result<Vec<Turn>, String> {
        let dir = ensure_models(models_dir)?;

        let mut pipeline = OwnedDiarizationPipeline::from_dir(&dir, MODE)
            .map_err(|e| format!("화자분리 모델을 불러올 수 없습니다: {e}"))?;

        let result = pipeline
            .run(samples)
            .map_err(|e| format!("화자분리에 실패했습니다: {e}"))?;

        // 겹쳐 말한 구간에서 화자를 하나로 정한다. 회의록에는 한 줄에 한 명이어야
        // 읽히기 때문이다.
        let mut exclusive = result.discrete_diarization.clone();
        exclusive.make_exclusive();

        let turns = exclusive
            .to_segments()
            .into_iter()
            .map(|s| Turn {
                start: s.start,
                end: s.end,
                speaker: label(&s.speaker),
            })
            .collect();

        Ok(smooth(turns))
    }
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod imp {
    use super::Turn;
    use std::path::{Path, PathBuf};

    const UNSUPPORTED: &str = "이 플랫폼에서는 화자분리를 지원하지 않습니다.";

    pub fn ensure_models(_models_dir: &Path) -> Result<PathBuf, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn diarize(_samples: &[f32], _models_dir: &Path) -> Result<Vec<Turn>, String> {
        Err(UNSUPPORTED.into())
    }
}

/// 화자분리 모델을 내려받고 저장된 위치를 돌려준다.
///
/// whisper 모델과 같은 디렉터리 아래에 둔다. 이미 있으면 네트워크를 타지 않는다.
pub fn ensure_models(models_dir: &Path) -> Result<PathBuf, String> {
    imp::ensure_models(models_dir)
}

/// 16kHz 모노 샘플에서 화자 구간을 뽑는다.
pub fn diarize(samples: &[f32], models_dir: &Path) -> Result<Vec<Turn>, String> {
    imp::diarize(samples, models_dir)
}

/// 전사 세그먼트마다 화자를 붙인다.
///
/// 겹치는 시간이 가장 긴 구간의 화자를 고른다. 세그먼트 경계와 화자 경계는
/// 서로 독립이라 정확히 맞아떨어지지 않는다 — 한 문장이 두 화자에 걸치면
/// 더 많이 말한 쪽으로 준다.
pub fn assign(segments: &mut [Segment], turns: &[Turn]) {
    for segment in segments {
        let mut best: Option<(&str, f64)> = None;
        for turn in turns {
            let overlap = turn.end.min(segment.end) - turn.start.max(segment.start);
            if overlap <= 0.0 {
                continue;
            }
            if best.is_none_or(|(_, b)| overlap > b) {
                best = Some((&turn.speaker, overlap));
            }
        }
        segment.speaker = best.map(|(s, _)| s.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(start: f64, end: f64, speaker: &str) -> Turn {
        Turn { start, end, speaker: speaker.into() }
    }

    #[test]
    fn labels_are_human_readable() {
        assert_eq!(label("SPEAKER_00"), "화자 1");
        assert_eq!(label("SPEAKER_03"), "화자 4");
    }

    /// 원시 출력의 0.1초짜리 조각은 맞장구나 숨소리다. 남기면 한 문장 안에서
    /// 화자가 몇 번씩 바뀐 것처럼 보인다.
    #[test]
    fn drops_flicker_segments() {
        let out = smooth(vec![
            turn(0.0, 5.0, "화자 1"),
            turn(5.0, 5.1, "화자 2"),
            turn(5.1, 9.0, "화자 1"),
        ]);
        assert_eq!(out.len(), 1, "조각을 버린 뒤 같은 화자끼리 이어져야 한다");
        assert_eq!(out[0].end, 9.0);
    }

    #[test]
    fn merges_across_breath_pauses() {
        let out = smooth(vec![turn(0.0, 3.0, "화자 1"), turn(3.5, 6.0, "화자 1")]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn keeps_genuine_speaker_change() {
        let out = smooth(vec![turn(0.0, 3.0, "화자 1"), turn(3.1, 6.0, "화자 2")]);
        assert_eq!(out.len(), 2);
    }

    /// 긴 침묵 뒤에 같은 사람이 다시 말하면 별개 발언이다.
    #[test]
    fn keeps_same_speaker_across_long_silence() {
        let out = smooth(vec![turn(0.0, 3.0, "화자 1"), turn(20.0, 25.0, "화자 1")]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn assigns_dominant_speaker() {
        let turns = vec![turn(0.0, 4.0, "화자 1"), turn(4.0, 10.0, "화자 2")];
        let mut segs = vec![
            Segment { start: 0.0, end: 3.0, text: "가".into(), speaker: None, speaker_turn: false },
            // 3.0~4.0은 화자 1, 4.0~8.0은 화자 2 — 더 긴 쪽으로 간다.
            Segment { start: 3.0, end: 8.0, text: "나".into(), speaker: None, speaker_turn: false },
        ];
        assign(&mut segs, &turns);
        assert_eq!(segs[0].speaker.as_deref(), Some("화자 1"));
        assert_eq!(segs[1].speaker.as_deref(), Some("화자 2"));
    }

    /// 화자 구간 밖의 세그먼트는 비워 둔다 — 없는 화자를 지어내지 않는다.
    #[test]
    fn leaves_unmatched_segment_unassigned() {
        let mut segs = vec![Segment {
            start: 50.0, end: 51.0, text: "다".into(), speaker: None, speaker_turn: false,
        }];
        assign(&mut segs, &[turn(0.0, 4.0, "화자 1")]);
        assert_eq!(segs[0].speaker, None);
    }
}
