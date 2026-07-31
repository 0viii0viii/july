//! 입력 장치가 왜 안 보이는지 들여다보는 진단용 예제.
//!
//!     cargo run --release --example devtest

use cpal::traits::{DeviceTrait, HostTrait};

fn main() {
    let host = cpal::default_host();

    println!("기본 입력 장치:");
    match host.default_input_device() {
        None => println!("  (없음)"),
        Some(d) => match d.description() {
            Ok(desc) => println!("  이름={:?} 방향={:?}", desc.name(), desc.direction()),
            Err(e) => println!("  description() 실패: {e}"),
        },
    }

    println!("\ninput_devices() 열거:");
    match host.input_devices() {
        Err(e) => println!("  실패: {e}"),
        Ok(devices) => {
            let mut n = 0;
            for d in devices {
                n += 1;
                match d.description() {
                    Ok(desc) => println!("  [{n}] {:?}", desc.name()),
                    Err(e) => println!("  [{n}] description() 실패: {e}"),
                }
            }
            if n == 0 {
                println!("  (0개)");
            }
        }
    }

    println!("\n전체 devices() 열거 (입력/출력 구분 없이):");
    match host.devices() {
        Err(e) => println!("  실패: {e}"),
        Ok(devices) => {
            for (i, d) in devices.enumerate() {
                let name = d
                    .description()
                    .map(|x| x.name().to_string())
                    .unwrap_or_else(|e| format!("<{e}>"));
                println!("  [{}] {name}  입력지원={}", i + 1, d.supports_input());
            }
        }
    }

    println!("\n우리 래퍼:");
    match july_lib::audio::input_devices() {
        Ok(ds) if ds.is_empty() => println!("  (빈 목록)"),
        Ok(ds) => {
            for d in ds {
                println!("  {} {}", if d.is_default { "*" } else { " " }, d.name);
            }
        }
        Err(e) => println!("  오류: {e}"),
    }

    let hw = july_lib::catalog::hardware();
    println!(
        "\n기기: {} · 메모리 {:.0}GB · 모델 예산 {:.1}GB",
        hw.cpu,
        hw.total_memory_bytes as f64 / 1e9,
        hw.model_budget_bytes as f64 / 1e9,
    );

    println!("\n요약 모델 카탈로그:");
    for e in july_lib::catalog::catalog(&hw, &[]) {
        println!(
            "  {:<18} {:>6.1}GB  {:?}{}{}",
            e.id,
            e.bytes as f64 / 1e9,
            e.fit,
            if e.recommended { "  ← 추천" } else { "" },
            if e.verified { "  (검증됨)" } else { "" },
        );
    }
}
