//! 요약만 따로 돌린다. 프롬프트를 손볼 때 매번 전사할 필요가 없다.
//!
//!     cargo run --release --example summary -- <녹취록파일> [ollama|api]
//!
//! 참석자·주제·용어는 JULY_ATTENDEES / JULY_TOPIC / JULY_TERMS 로 준다.

use july_lib::summarize::{self, Backend, Context};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("사용법: summary <녹취록파일> [ollama|api]")?;
    let backend_arg = args.next().unwrap_or_else(|| "ollama".into());

    let transcript = std::fs::read_to_string(&path)?;
    let context = Context {
        attendees: std::env::var("JULY_ATTENDEES").unwrap_or_default(),
        topic: std::env::var("JULY_TOPIC").unwrap_or_default(),
        terms: std::env::var("JULY_TERMS").unwrap_or_default(),
    };

    let backend = match backend_arg.as_str() {
        "ollama" => Backend::Ollama {
            model: std::env::var("JULY_OLLAMA_MODEL").unwrap_or_else(|_| "qwen3:8b".into()),
            endpoint: None,
        },
        "api" => Backend::Anthropic {
            api_key: std::env::var("ANTHROPIC_API_KEY")?,
            model: std::env::var("JULY_API_MODEL").unwrap_or_else(|_| "claude-opus-5".into()),
        },
        other => return Err(format!("알 수 없는 백엔드: {other}").into()),
    };

    let started = std::time::Instant::now();
    let out = summarize::summarize(&backend, &transcript, &context).await?;
    eprintln!(
        "녹취록 {}자 → 회의록 {}자 ({:.1}초)",
        transcript.chars().count(),
        out.chars().count(),
        started.elapsed().as_secs_f64()
    );
    println!("{out}");
    Ok(())
}
