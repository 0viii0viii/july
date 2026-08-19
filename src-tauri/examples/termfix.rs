//! 용어 교정만 따로 돌린다. 임계값을 조정할 때 매번 전사할 필요가 없다.
//!
//!     cargo run --release --example termfix -- <텍스트파일> ["용어,목록"]
//!
//! 바뀐 자리만 출력한다. 거짓 양성을 세는 게 목적이라 전문은 찍지 않는다 —
//! 회의 내용을 터미널에 흘리지 않으려는 의도도 있다.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("사용법: termfix <텍스트파일> [\"용어,목록\"]")?;
    let user_terms = args.next().unwrap_or_default();

    let text = std::fs::read_to_string(&path)?;
    let mut changes: std::collections::BTreeMap<(String, String), usize> = Default::default();

    for line in text.lines() {
        // pipeline 출력의 머리말(모델 경로·파일명)은 녹취록이 아니다. 여기까지
        // 세면 교정 건수가 부풀고 없는 오교정이 있는 것처럼 보인다.
        let Some(body) = line.trim().strip_prefix('[').and_then(|l| l.split_once(']')).map(|(_, b)| b)
        else {
            continue;
        };
        let line = body.trim();
        let fixed = july_lib::terms::correct(line, &user_terms);
        if fixed == line {
            continue;
        }
        // 어떤 토큰이 무엇으로 바뀌었는지만 뽑는다.
        let before: Vec<&str> = line.split_whitespace().collect();
        let after: Vec<&str> = fixed.split_whitespace().collect();
        for (b, a) in before.iter().zip(after.iter()) {
            if b != a {
                *changes.entry((b.to_string(), a.to_string())).or_default() += 1;
            }
        }
    }

    let total: usize = changes.values().sum();
    println!("바뀐 토큰 {total}건 / {}종\n", changes.len());
    let mut rows: Vec<_> = changes.into_iter().collect();
    rows.sort_by_key(|((_, _), n)| std::cmp::Reverse(*n));
    for ((b, a), n) in rows {
        println!("  {b:20} → {a:20} ×{n}");
    }
    Ok(())
}
