//! 전사 → 요약 파이프라인을 UI 없이 확인한다.
//!
//!     cargo run --release --example pipeline -- <음성파일> [모델] [요약백엔드]
//!
//! 모델:      base | turbo            (기본 turbo)
//! 요약백엔드: none | ollama | api     (기본 none)
//!
//! api를 쓰려면 ANTHROPIC_API_KEY 환경변수가 필요하다.

use std::path::{Path, PathBuf};

use july_lib::{summarize, transcribe};

fn models_dir() -> PathBuf {
    std::env::var("JULY_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("크레이트 디렉터리에 부모가 있어야 합니다")
                .join("models")
        })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);

    let audio = args.next().unwrap_or_else(|| {
        eprintln!("사용법: cargo run --release --example pipeline -- <음성파일> [base|turbo] [none|ollama|api]");
        std::process::exit(2);
    });
    let model_arg = args.next().unwrap_or_else(|| "turbo".into());
    let summarizer = args.next().unwrap_or_else(|| "none".into());

    let model_file = match model_arg.as_str() {
        "base" => "ggml-base.bin",
        "turbo" => "ggml-large-v3-turbo.bin",
        other => {
            eprintln!("알 수 없는 모델: {other} (base 또는 turbo)");
            std::process::exit(2);
        }
    };

    let model_path = models_dir().join(model_file);
    println!("모델:   {}", model_path.display());
    println!("음성:   {audio}");
    println!("\n전사 중...\n");

    // 회의 배경 정보. 앱에서는 처리 직전에 화면으로 받는다.
    let context = summarize::Context {
        attendees: std::env::var("JULY_ATTENDEES").unwrap_or_default(),
        topic: std::env::var("JULY_TOPIC").unwrap_or_default(),
        terms: std::env::var("JULY_TERMS").unwrap_or_default(),
    };
    let hint = context.as_speech_hint();
    let audio_samples = july_lib::decode::decode_to_mono_16k(Path::new(&audio))?.samples;
    let mut result = transcribe::transcribe_samples(
        &model_path,
        &audio_samples,
        Some("ko"),
        hint.as_deref(),
    )?;

    // 앱과 같은 경로. JULY_NO_DIARIZE=1 로 끄면 전후 비교를 할 수 있다.
    if std::env::var("JULY_NO_DIARIZE").is_err() {
        let dir = models_dir();
        match july_lib::diarize::diarize(&audio_samples, &dir) {
            Ok(turns) => {
                eprintln!("화자 구간 {}개", turns.len());
                july_lib::diarize::assign(&mut result.segments, &turns);
            }
            Err(e) => eprintln!("화자분리를 건너뜁니다: {e}"),
        }
    }

    // 앱과 같은 경로를 밟는다 — 여기서 빠지면 예제로 확인한 결과가 실제와 다르다.
    let corrected = std::env::var("JULY_NO_TERM_FIX").is_err();
    if corrected {
        for seg in &mut result.segments {
            seg.text = july_lib::terms::correct(&seg.text, &context.terms);
        }
    }

    for seg in &result.segments {
        let who = seg.speaker.as_deref().unwrap_or("-");
        println!("[{:>6.1}s → {:>6.1}s] {:<6} {}", seg.start, seg.end, who, seg.text);
    }

    let audio_seconds = result
        .segments
        .last()
        .map(|s| s.end)
        .unwrap_or(0.0);
    let speed = if result.elapsed > 0.0 {
        audio_seconds / result.elapsed
    } else {
        0.0
    };
    println!(
        "\n전사 완료: {:.1}초 소요 (음성 {:.1}초, 실시간 대비 {:.1}배)",
        result.elapsed, audio_seconds, speed
    );

    let backend = match summarizer.as_str() {
        "none" => return Ok(()),
        "ollama" => summarize::Backend::Ollama {
            model: std::env::var("JULY_OLLAMA_MODEL").unwrap_or_else(|_| "qwen3:8b".into()),
            endpoint: None,
        },
        "api" => summarize::Backend::Anthropic {
            api_key: std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| "ANTHROPIC_API_KEY 환경변수가 필요합니다")?,
            model: std::env::var("JULY_ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-opus-5".into()),
        },
        other => {
            eprintln!("알 수 없는 요약 백엔드: {other} (none, ollama, api)");
            std::process::exit(2);
        }
    };

    println!("\n요약 중... ({})\n", backend.label());
    let started = std::time::Instant::now();
    // 개발용 파이프라인엔 명단이 없다 — 화자 라벨을 그대로 내보낸다.
    let plain = transcribe::plain_text(&result.segments, &std::collections::BTreeMap::new());
    let summary = summarize::summarize(&backend, &plain, &context).await?;
    println!("{summary}");
    println!("\n요약 완료: {:.1}초 소요", started.elapsed().as_secs_f64());

    // JULY_SAVE_TO가 있으면 앱 보관함 형식(JSON)으로 떨군다. UI를 실제
    // 데이터로 확인할 때 쓴다.
    if let Ok(dest) = std::env::var("JULY_SAVE_TO") {
        let meeting = serde_json::json!({
            "id": format!("dev-{}", result.segments.len()),
            "title": "",
            "recorded_at": std::env::var("JULY_RECORDED_AT")
                .unwrap_or_else(|_| "2026-07-30T22:10:00".into()),
            "audio_path": audio,
            "duration": result.segments.last().map(|s| s.end).unwrap_or(0.0),
            "segments": result.segments,
            "summary": summary,
        });
        let archive = serde_json::json!({ "meetings": [meeting] });
        std::fs::write(&dest, serde_json::to_string_pretty(&archive)?)?;
        println!("\n보관함에 저장: {dest}");
    }

    Ok(())
}
