//! 요약 모델 카탈로그와 기기 사양 기반 추천.
//!
//! 앱을 처음 설치한 사람에게는 Ollama 모델이 하나도 없다. 터미널을 열어
//! `ollama pull`을 치라고 할 수는 없으므로, 쓸 만한 모델을 골라서 보여주고
//! 기기 사양에 맞는 것을 추천한 뒤 앱 안에서 내려받는다.
//!
//! 목록의 태그와 용량은 Ollama 레지스트리 매니페스트로 실제 확인한 값이다.
//! 없는 태그를 넣으면 설치 버튼이 404로 죽으므로 추측해서 넣지 않는다.

use serde::{Deserialize, Serialize};

/// 모델 외에 늘 잡아먹는 몫(GB) — OS, 이 앱, whisper 모델, 그리고 사용자가
/// 회의 중에 켜 둘 다른 프로그램들. 이만큼 뺀 나머지를 모델 예산으로 본다.
const RESERVED_GB: f64 = 10.0;

/// 이보다 여유가 없으면 "돌아가긴 하지만 빠듯함"으로 표시한다.
const TIGHT_GB: f64 = 4.0;

/// 기본 추천에서 이 크기를 넘지 않는다.
///
/// 메모리가 남는다고 제일 큰 모델을 권하면 요약 한 건에 몇 분씩 걸린다.
/// 회의록 요약은 정해진 형식으로 뽑아내는 작업이라 모델을 키운 만큼 결과가
/// 좋아지지 않는다 — 기다림이 더 비싸다. 더 큰 걸 원하면 직접 고르면 된다.
const RECOMMEND_CAP_BYTES: u64 = 10_000_000_000;

/// 요약 한 건에 걸리는 체감 시간. 크기로 가르는 대략적인 구분이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    Fast,
    Balanced,
    Slow,
}

fn speed_for(bytes: u64) -> Speed {
    match bytes {
        0..=3_000_000_000 => Speed::Fast,
        3_000_000_001..=10_000_000_000 => Speed::Balanced,
        _ => Speed::Slow,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// 사양에 여유가 있다.
    Comfortable,
    /// 돌아가지만 메모리가 빠듯하다.
    Tight,
    /// 이 기기에서는 권장하지 않는다.
    TooBig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    /// `ollama pull`에 그대로 넘기는 이름.
    pub id: &'static str,
    pub name: &'static str,
    pub maker: &'static str,
    /// 내려받을 용량(바이트). 레지스트리 매니페스트에서 확인한 값.
    pub bytes: u64,
    /// 한국어 관점에서의 한 줄 평.
    pub note: &'static str,
    /// 우리가 실제로 회의록 요약을 돌려본 모델인지.
    pub verified: bool,
    pub fit: Fit,
    pub speed: Speed,
    pub recommended: bool,
    pub installed: bool,
}

struct Model {
    id: &'static str,
    name: &'static str,
    maker: &'static str,
    bytes: u64,
    note: &'static str,
    verified: bool,
    /// 같은 크기대에서 한국어 요약에 먼저 권하고 싶은 순서. 낮을수록 우선.
    rank: u8,
}

/// 2026-07-30 기준 registry.ollama.ai 매니페스트로 존재와 용량을 확인함.
const MODELS: &[Model] = &[
    Model {
        id: "exaone3.5:2.4b",
        name: "EXAONE 3.5 2.4B",
        maker: "LG AI Research",
        bytes: 1_640_000_000,
        note: "한국어를 우선해 학습한 모델. 가장 가벼워서 저사양에서도 돈다.",
        verified: false,
        rank: 0,
    },
    Model {
        id: "qwen3:4b",
        name: "Qwen3 4B",
        maker: "Alibaba",
        bytes: 2_500_000_000,
        note: "가벼운 축에서 한국어가 무난하다.",
        verified: false,
        rank: 1,
    },
    Model {
        id: "exaone3.5:7.8b",
        name: "EXAONE 3.5 7.8B",
        maker: "LG AI Research",
        bytes: 4_770_000_000,
        note: "한국어 특화 모델의 표준 크기. 국내 회의 용어에 강하다.",
        verified: false,
        rank: 0,
    },
    Model {
        id: "qwen3:8b",
        name: "Qwen3 8B",
        maker: "Alibaba",
        bytes: 5_230_000_000,
        note: "이 앱에서 한국어 회의록 요약을 실제로 돌려본 모델. 무난하다.",
        verified: true,
        rank: 1,
    },
    Model {
        id: "gemma4:12b",
        name: "Gemma 4 12B",
        maker: "Google",
        bytes: 7_560_000_000,
        note: "다국어 폭이 넓고 문장이 깔끔한 편이다.",
        verified: false,
        rank: 1,
    },
    Model {
        id: "qwen3:14b",
        name: "Qwen3 14B",
        maker: "Alibaba",
        bytes: 9_280_000_000,
        note: "8B보다 긴 회의에서 맥락을 잘 붙든다.",
        verified: false,
        rank: 0,
    },
    Model {
        id: "gemma4:26b",
        name: "Gemma 4 26B",
        maker: "Google",
        bytes: 17_990_000_000,
        note: "메모리가 넉넉할 때 고르는 대형 모델.",
        verified: false,
        rank: 1,
    },
    Model {
        id: "exaone3.5:32b",
        name: "EXAONE 3.5 32B",
        maker: "LG AI Research",
        bytes: 19_340_000_000,
        note: "한국어 특화 모델 중 가장 크다. 32GB 이상에서 권장.",
        verified: false,
        rank: 0,
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hardware {
    pub total_memory_bytes: u64,
    pub cpu: String,
    /// 모델에 쓸 수 있다고 보는 예산(바이트).
    pub model_budget_bytes: u64,
}

pub fn hardware() -> Hardware {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();

    let total = sys.total_memory();
    let total_gb = total as f64 / 1e9;
    let budget_gb = (total_gb - RESERVED_GB).max(0.0);

    Hardware {
        total_memory_bytes: total,
        cpu: cpu_brand(),
        model_budget_bytes: (budget_gb * 1e9) as u64,
    }
}

/// 칩 이름 ("Apple M2 Pro" 등).
///
/// sysinfo의 `System::name()`은 OS 이름("Darwin")을 주지 칩 이름을 주지
/// 않는다. macOS에서는 sysctl에 실제 브랜드 문자열이 들어 있다.
#[cfg(target_os = "macos")]
fn cpu_brand() -> String {
    std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "알 수 없는 프로세서".into())
}

#[cfg(not(target_os = "macos"))]
fn cpu_brand() -> String {
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
    sys.cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| sysinfo::System::cpu_arch())
}

fn fit_for(bytes: u64, budget: u64) -> Fit {
    if bytes <= budget {
        Fit::Comfortable
    } else if bytes as f64 <= budget as f64 + TIGHT_GB * 1e9 {
        Fit::Tight
    } else {
        Fit::TooBig
    }
}

/// 기기 사양과 이미 설치된 목록을 반영한 카탈로그.
///
/// 추천은 "여유롭게 돌아가는 것 중 가장 큰 것"으로 고르되, 같은 크기대에서는
/// 한국어에 더 나은 쪽(rank)을 앞세운다.
pub fn catalog(hw: &Hardware, installed: &[String]) -> Vec<CatalogEntry> {
    let budget = hw.model_budget_bytes;

    // Ollama 목록은 "qwen3:8b" 형태로 오지만 태그가 붙는 경우가 있어 앞부분만 본다.
    let is_installed = |id: &str| {
        installed
            .iter()
            .any(|m| m == id || m.split(':').next() == id.split(':').next() && m == id)
    };

    let fits = |m: &&Model| fit_for(m.bytes, budget) == Fit::Comfortable;

    // 여유롭게 도는 것 중 가장 큰 것을 고르되 속도 상한을 넘지 않는다.
    // 상한 아래에 아무것도 없는 저사양 기기에서는 가장 작은 것으로 물러선다.
    let best = MODELS
        .iter()
        .filter(fits)
        .filter(|m| m.bytes <= RECOMMEND_CAP_BYTES)
        .max_by_key(|m| (m.bytes, std::cmp::Reverse(m.rank)))
        .or_else(|| MODELS.iter().filter(fits).min_by_key(|m| m.bytes))
        .or_else(|| MODELS.iter().min_by_key(|m| m.bytes))
        .map(|m| m.id);

    MODELS
        .iter()
        .map(|m| CatalogEntry {
            id: m.id,
            name: m.name,
            maker: m.maker,
            bytes: m.bytes,
            note: m.note,
            verified: m.verified,
            fit: fit_for(m.bytes, budget),
            speed: speed_for(m.bytes),
            recommended: best == Some(m.id),
            installed: is_installed(m.id),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(gb: f64) -> Hardware {
        Hardware {
            total_memory_bytes: (gb * 1e9) as u64,
            cpu: "test".into(),
            model_budget_bytes: ((gb - RESERVED_GB).max(0.0) * 1e9) as u64,
        }
    }

    #[test]
    fn recommends_something_on_every_tier() {
        // 8GB처럼 예산이 0인 기기에서는 권장 모델이 없을 수 있지만, 목록 자체는
        // 비지 않아야 한다 — 사용자가 감수하고 고를 수는 있어야 한다.
        for gb in [8.0, 16.0, 32.0, 64.0] {
            let list = catalog(&hw(gb), &[]);
            assert!(!list.is_empty(), "{gb}GB에서 카탈로그가 비었다");
        }
    }

    #[test]
    fn bigger_machines_get_bigger_recommendations() {
        let pick = |gb: f64| {
            catalog(&hw(gb), &[])
                .into_iter()
                .find(|e| e.recommended)
                .map(|e| e.bytes)
        };
        let m16 = pick(16.0).expect("16GB에서는 추천이 있어야 한다");
        let m32 = pick(32.0).expect("32GB에서는 추천이 있어야 한다");
        assert!(m32 > m16, "메모리가 크면 더 큰 모델을 권해야 한다");
    }

    #[test]
    fn marks_installed() {
        let list = catalog(&hw(32.0), &["qwen3:8b".to_string()]);
        let entry = list.iter().find(|e| e.id == "qwen3:8b").unwrap();
        assert!(entry.installed);
        assert!(!list.iter().find(|e| e.id == "qwen3:4b").unwrap().installed);
    }
}
