//! 회의록 요약 — 로컬(Ollama)과 클라우드(Anthropic) 중 선택.
//!
//! 두 경로 모두 사용자 쪽에서 비용을 부담한다. 로컬은 공짜지만 품질이 낮고,
//! API는 사용자 본인 키를 쓴다. 앱은 어느 쪽으로도 서버를 두지 않는다.

use serde::{Deserialize, Serialize};

const OLLAMA_DEFAULT_ENDPOINT: &str = "http://localhost:11434";
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// 정책 거절 시 Anthropic이 권장 모델로 재시도하게 하는 베타 플래그.
const ANTHROPIC_FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// 요약 출력이 잘리지 않을 만큼 넉넉하게. Opus 5는 thinking이 기본으로 켜져
/// 있고 max_tokens가 thinking과 응답을 합쳐서 제한하므로 여유를 둔다.
const ANTHROPIC_MAX_TOKENS: u32 = 8_000;

/// 회의록 출력에 남겨둘 토큰. 컨텍스트는 프롬프트와 응답이 나눠 쓴다.
///
/// 안건별로 풀어 쓴 회의록은 5천 자를 넘기도 한다(실측 0.62토큰/자 기준 약
/// 3,100토큰). 2048로 두면 액션 아이템을 쓰다가 잘린다.
const OLLAMA_OUTPUT_HEADROOM: usize = 4_096;

/// 가장 짧은 회의라도 이만큼은 연다.
///
/// 출력 여유만으로 이미 4K를 쓰므로 4096으로 두면 녹취록 자리가 남지 않는다.
const OLLAMA_MIN_CTX: usize = 8_192;

/// 출력 여유를 빼고도 녹취록이 들어갈 자리가 남아야 한다. 4K면 실측
/// 0.62토큰/자로 6,600자 — 10분 남짓한 회의가 들어간다.
const _: () = assert!(OLLAMA_MIN_CTX - OLLAMA_OUTPUT_HEADROOM >= 4_096);

/// 카탈로그의 8B급 모델이 40960까지 지원하지만, 그만큼 KV 캐시를 잡는다.
///
/// 이 상한으로 못 담는 회의는 잘라내지 않고 구간을 나눠 처리한다 — 예전에는
/// 그냥 넘겨서 Ollama가 앞에서부터 버렸다. 67분짜리 회의(83,185자 ≈ 51,574토큰)
/// 하나로 이 상한을 넘겼고, 앞 25분이 통째로 사라졌다.
const OLLAMA_MAX_CTX: usize = 32_768;

/// 한 번에 모델에 넘길 녹취록 분량(글자).
///
/// 보수적인 추정치(0.85토큰/자)로도 지시문과 출력 여유를 더해 16,384 컨텍스트에
/// 들어가는 크기다. 더 크게 잡을 수도 있지만, 8B 모델은 컨텍스트가 길어질수록
/// 가운데 내용을 놓치는 편이라 굳이 키우지 않는다.
const OLLAMA_CHUNK_CHARS: usize = 12_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Backend {
    /// 완전 로컬. 인터넷도, 비용도 필요 없다.
    Ollama {
        model: String,
        /// 비워두면 localhost:11434.
        #[serde(default)]
        endpoint: Option<String>,
    },
    /// 사용자 본인의 Anthropic API 키를 쓴다.
    Anthropic { api_key: String, model: String },
}

impl Backend {
    /// UI에 표시할 이름.
    pub fn label(&self) -> String {
        match self {
            Backend::Ollama { model, .. } => format!("로컬 · {model}"),
            Backend::Anthropic { model, .. } => format!("API · {model}"),
        }
    }
}

/// 그 회의에만 해당하는 배경 정보.
///
/// 전역 설정이 아니라 회의마다 받는다 — 참석자도 안건도 매번 다르기 때문이다.
/// 요약 품질에서 차이가 큰 부분은 특히 참석자 명단이다. 이게 있으면 액션
/// 아이템의 담당자를 실제 이름으로 붙이고, 없으면 "담당자 미상"이 늘어난다.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Context {
    /// 참석자 이름. UI에서 쉼표로 구분해 받는다.
    #[serde(default)]
    pub attendees: String,
    /// 회의 주제나 안건.
    #[serde(default)]
    pub topic: String,
    /// 자주 틀리는 고유명사·사내 용어.
    #[serde(default)]
    pub terms: String,
}

impl Context {
    pub fn is_empty(&self) -> bool {
        self.attendees.trim().is_empty()
            && self.topic.trim().is_empty()
            && self.terms.trim().is_empty()
    }

    /// whisper의 initial_prompt로 넘길 문장.
    ///
    /// whisper는 이걸 "직전 문맥"으로 취급하므로 목록이 아니라 자연스러운
    /// 문장이어야 표기가 제대로 유도된다.
    pub fn as_speech_hint(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.attendees.trim().is_empty() {
            parts.push(format!("회의 참석자: {}.", self.attendees.trim()));
        }
        if !self.topic.trim().is_empty() {
            parts.push(format!("회의 주제: {}.", self.topic.trim()));
        }
        if !self.terms.trim().is_empty() {
            parts.push(format!("용어: {}.", self.terms.trim()));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    }

    /// 요약 프롬프트에 끼워 넣을 배경 설명.
    fn as_briefing(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut out = String::from("이 회의에 대해 알려진 정보:\n");
        if !self.attendees.trim().is_empty() {
            out.push_str(&format!("- 참석자: {}\n", self.attendees.trim()));
        }
        if !self.topic.trim().is_empty() {
            out.push_str(&format!("- 주제: {}\n", self.topic.trim()));
        }
        if !self.terms.trim().is_empty() {
            out.push_str(&format!("- 관련 용어: {}\n", self.terms.trim()));
        }
        out.push('\n');
        out
    }
}

/// 마지막 단계에 넘기는 재료가 원본 녹취록인지, 구간별 메모인지.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Transcript,
    Notes,
}

fn build_prompt(body: &str, context: &Context, source: Source) -> String {
    let briefing = context.as_briefing();
    let owner_rule = if context.attendees.trim().is_empty() {
        "- 담당자나 기한이 언급되지 않았다면 '미정'으로 적으세요.\n"
    } else {
        "- 액션 아이템의 담당자는 위 참석자 명단의 이름으로 적으세요. \
         녹취록에서 누구인지 알 수 없으면 '미정'으로 두세요.\n"
    };

    // 화자분리를 거친 녹취록은 "화자 1: ..." 형태로 들어온다. 모델이 이걸
    // 실제 이름과 연결할 수 있게 규칙을 준다 — 참석자 명단이 없으면 연결할
    // 근거가 없으므로 번호를 그대로 두게 한다.
    let speaker_rule = if context.attendees.trim().is_empty() {
        "- 녹취록의 '화자 1', '화자 2'는 서로 다른 참석자입니다. 누구인지 알 수 \
         없으므로 그대로 두세요.\n"
    } else {
        "- 녹취록의 '화자 1', '화자 2'는 서로 다른 참석자입니다. 발언 내용으로 \
         위 참석자 명단의 누구인지 짐작할 수 있으면 실제 이름으로 바꾸고, \
         확실하지 않으면 번호를 그대로 두세요. 억지로 배정하지 마세요.\n"
    };

    // 메모는 이미 한 번 걸러진 글이라 "화자 1"이 남아 있지 않을 수 있다.
    // 그래도 규칙을 빼지는 않는다 — 남아 있으면 처리해야 한다.
    let intro = match source {
        Source::Transcript => "다음은 회의 녹취록입니다. 이를 회의록으로 정리해 주세요.",
        Source::Notes => "다음은 긴 회의를 구간별로 나눠 받아적은 메모입니다. \
                          구간 경계는 회의의 안건 경계가 아니므로, 같은 안건이 여러 \
                          구간에 흩어져 있으면 하나로 합쳐서 회의록으로 정리해 주세요.",
    };
    let source_word = match source {
        Source::Transcript => "녹취록",
        Source::Notes => "메모",
    };

    format!(
        "{briefing}\
         {intro}\n\
         \n\
         회의에 참석하지 못한 사람이 이 회의록만 읽고도 무슨 이야기가 오갔는지 \
         알 수 있어야 합니다. 짧게 줄이는 것이 목적이 아닙니다 — 녹취록에 있는 \
         내용은 최대한 살리고, 없는 말만 쓰지 마세요.\n\
         \n\
         형식:\n\
         ## 한 줄 요약\n\
         회의 전체를 한 문장으로.\n\
         \n\
         ## 주요 논의\n\
         안건마다 샵 세 개짜리 소제목을 달아 나누고, 그 아래에 오간 이야기를 \
         문단으로 풀어 쓰세요. 안건 하나에 최소 서너 문장은 나와야 합니다.\n\
         - 무엇을 결론냈는지만이 아니라 **왜 그렇게 판단했는지**, 어떤 근거와 \
         제약이 언급됐는지까지 적으세요.\n\
         - 의견이 갈린 지점은 양쪽 입장을 모두 남기세요.\n\
         - 숫자, 날짜, 기간, 금액, 시스템·제품·서버 이름, 버전은 {source_word}에 \
         나온 그대로 옮기세요. 뭉뚱그리지 마세요.\n\
         - 논의가 길었던 안건은 길게, 짧게 지나간 안건은 짧게 씁니다.\n\
         - 안건은 빠짐없이 뽑으세요. 잠깐 지나간 이야기도 하나의 안건입니다.\n\
         - 안건마다 **최소 다섯 문장**을 쓰세요. 한 줄로 끝나면 회의록이 아니라 \
         목차입니다.\n\
         \n\
         ## 결정 사항\n\
         - 결정된 내용 (그렇게 정한 이유)\n\
         \n\
         ## 액션 아이템\n\
         - [담당자] 할 일 (기한)\n\
         \n\
         규칙:\n\
         - {source_word}에 없는 내용을 지어내지 마세요.\n\
         {speaker_rule}\
         {owner_rule}\
         - 결론이 나지 않은 채 끝난 논의도 '결론 없이 종료'라고 밝혀 남기세요. \
         결정된 것만 골라 적으면 회의록이 실제보다 매끄러워 보입니다.\n\
         - 음성 인식 오류로 보이는 부분은 문맥에 맞게 자연스럽게 고쳐도 됩니다. \
         특히 위에 적힌 이름과 용어는 그 표기를 따르세요.\n\
         - 한국어로 작성하세요.\n\
         \n\
         ---\n\
         {body}"
    )
}

/// 구간별 메모를 받아적게 하는 프롬프트.
///
/// 여기서는 요약하지 않는다. 요약은 마지막에 메모를 다시 넘겨 한 번만 한다 —
/// 두 번 압축하면 두 번째 단계가 살릴 내용 자체가 남지 않는다.
fn build_notes_prompt(chunk: &str, index: usize, total: usize, context: &Context) -> String {
    let briefing = context.as_briefing();
    format!(
        "{briefing}\
         다음은 회의 녹취록을 시간 순으로 {total}등분한 것 중 {index}번째 구간입니다. \
         이 구간에서 오간 이야기를 메모로 받아적으세요.\n\
         \n\
         - **요약하지 마세요.** 어떤 주제로 무슨 말이 오갔는지 순서대로 적으면 됩니다.\n\
         - 숫자, 날짜, 기간, 금액, 시스템·제품·서버 이름, 버전은 그대로 옮기세요.\n\
         - 결정된 것, 누가 무엇을 하기로 한 것, 확인이 더 필요하다고 한 것은 \
         빠뜨리지 마세요.\n\
         - 이 구간은 회의 도중에서 잘려 있습니다. 앞뒤가 끊겨 보여도 그대로 두고, \
         '회의를 마쳤다' 같은 마무리 문장은 쓰지 마세요.\n\
         - 녹취록에 없는 내용을 지어내지 마세요.\n\
         - 한국어로 작성하세요.\n\
         \n\
         ---\n\
         {chunk}"
    )
}

/// qwen3 계열은 추론 과정을 <think> 태그로 감싸 내보낸다. 회의록에 섞이면
/// 안 되므로 걷어낸다. `think: false`로도 대부분 막히지만 모델에 따라 새는
/// 경우가 있어 방어적으로 한 번 더 처리한다.
fn strip_think_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(open) = rest.find("<think>") {
        out.push_str(&rest[..open]);
        match rest[open..].find("</think>") {
            Some(close) => rest = &rest[open + close + "</think>".len()..],
            // 닫는 태그가 없으면 그 뒤는 전부 추론 과정으로 보고 버린다.
            None => return out.trim().to_string(),
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// 프롬프트를 담을 수 있는 컨텍스트 크기를 고른다.
///
/// **이걸 넘기지 않으면 Ollama는 기본 4096으로 돌고, 넘치는 프롬프트를 앞에서부터
/// 버린다.** 회의록 프롬프트는 배경 정보와 출력 형식이 맨 앞에 있어서 그게 통째로
/// 날아가고 녹취록 꼬리만 남는다 — 요약이 형식도 안 맞고 참석자 이름도 못 붙이는
/// 원인이었다.
///
/// 항상 최대로 잡지 않는 건 KV 캐시 때문이다. 8B 모델을 32K로 열면 몇 GB를 더
/// 잡는데, 10분짜리 회의에는 필요 없다. 2의 거듭제곱으로 올려서 같은 길이의
/// 회의가 같은 컨텍스트를 쓰게 한다 — 그래야 Ollama가 모델을 다시 안 올린다.
/// 프롬프트와 출력에 필요한 토큰 수를 어림한다.
///
/// qwen3 토크나이저 실측으로 한국어는 글자당 0.62토큰이었다(24,625자 →
/// 15,366토큰). 영어·숫자는 이보다 낮으니 한국어가 최악이고, 0.85로 잡으면
/// 4할 가까운 여유가 남는다. 정확한 토큰 수는 세어볼 방법이 없으므로 모자라서
/// 잘리는 것보다 남아서 메모리를 조금 더 쓰는 쪽을 택한다.
fn estimate_tokens(prompt: &str) -> usize {
    prompt.chars().count() * 85 / 100 + OLLAMA_OUTPUT_HEADROOM
}

/// 이 프롬프트가 한 번에 들어가는지.
fn fits_in_one_context(prompt: &str) -> bool {
    estimate_tokens(prompt) <= OLLAMA_MAX_CTX
}

fn choose_num_ctx(prompt: &str) -> usize {
    let estimated = estimate_tokens(prompt);

    let mut ctx = OLLAMA_MIN_CTX;
    while ctx < estimated && ctx < OLLAMA_MAX_CTX {
        ctx *= 2;
    }
    ctx.min(OLLAMA_MAX_CTX)
}

/// 녹취록을 줄 경계에서 자른다.
///
/// 화자분리를 거치면 한 줄이 한 사람의 발언이다. 줄 가운데를 자르면 발언이
/// 토막나므로 줄 단위로 모은다. 한 줄이 상한보다 길면 그 줄만 따로 낸다 —
/// 억지로 쪼개서 문장을 깨는 것보다 조금 큰 구간을 넘기는 편이 낫다.
/// 상한을 넘는 한 줄을 공백 경계에서 쪼갠다.
///
/// 화자분리에 실패한 녹취록은 전체가 한 줄로 온다(`plain_text`가 화자를 모르면
/// 공백으로 이어 붙인다). 줄 단위로만 나누면 그런 회의는 아예 나눠지지 않아
/// 예전처럼 앞부분이 잘려나간다.
fn split_long_line(line: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= max_chars {
        return vec![line.to_string()];
    }

    let mut out = Vec::new();
    let mut start = 0;
    while chars.len() - start > max_chars {
        let hard = start + max_chars;
        // 공백에서 끊어야 낱말이 갈라지지 않는다. 절반까지만 되짚고, 그 안에
        // 공백이 없으면(띄어쓰기 없는 글) 그냥 자른다.
        let cut = (start + max_chars / 2..hard)
            .rev()
            .find(|&i| chars[i].is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(hard);
        out.push(chars[start..cut].iter().collect());
        start = cut;
    }
    out.push(chars[start..].iter().collect());
    out
}

fn split_transcript(transcript: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in transcript.lines().flat_map(|l| split_long_line(l, max_chars)) {
        let line = line.as_str();
        let would_be = current.chars().count() + line.chars().count() + 1;
        if !current.is_empty() && would_be > max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

/// 프롬프트 하나를 Ollama에 넘기고 답을 받는다.
async fn ollama_generate(
    client: &reqwest::Client,
    model: &str,
    base: &str,
    prompt: &str,
) -> Result<String, String> {
    let url = format!("{base}/api/generate");

    let res = client
        .post(&url)
        .json(&serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            // 추론 모델의 사고 과정은 회의록에 불필요하다.
            "think": false,
            "options": {
                "num_ctx": choose_num_ctx(prompt),
                // Ollama 기본값은 버전에 따라 128로 잘리기도 한다. 컨텍스트에
                // 잡아둔 여유만큼은 쓰게 명시한다.
                "num_predict": OLLAMA_OUTPUT_HEADROOM,
                // 카탈로그 모델들의 기본값은 0.6 언저리다. 회의록은 녹취록에
                // 있는 내용만 옮기는 작업이라 낮을수록 지어내는 게 준다.
                "temperature": 0.3,
            },
        }))
        .send()
        .await
        .map_err(|e| {
            format!("Ollama에 연결할 수 없습니다 ({base}). 실행 중인지 확인해 주세요: {e}")
        })?;

    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Ollama 오류 {status}: {body}"));
    }

    let parsed: OllamaResponse = res
        .json()
        .await
        .map_err(|e| format!("Ollama 응답을 해석할 수 없습니다: {e}"))?;

    Ok(strip_think_tags(&parsed.response))
}

async fn summarize_ollama(
    client: &reqwest::Client,
    model: &str,
    endpoint: Option<&str>,
    transcript: &str,
    context: &Context,
) -> Result<String, String> {
    let base = endpoint.unwrap_or(OLLAMA_DEFAULT_ENDPOINT).trim_end_matches('/');

    // 한 번에 들어가면 그대로 간다.
    //
    // 들어가더라도 나누는 편이 나을까 싶어 재봤지만 아니었다. 31,584자 회의를
    // 3구간으로 나누니 1,719자(1,175초), 한 번에 넘기니 1,809자(322초)였다 —
    // 네 배 가까이 느리고 결과는 조금 짧았다. 나누는 건 상한을 넘을 때뿐이다.
    let single = build_prompt(transcript, context, Source::Transcript);
    if fits_in_one_context(&single) {
        return ollama_generate(client, model, base, &single).await;
    }

    // 안 들어가면 구간별로 받아적은 뒤 그 메모로 회의록을 쓴다.
    let chunks = split_transcript(transcript, OLLAMA_CHUNK_CHARS);
    let total = chunks.len();
    let mut notes = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let prompt = build_notes_prompt(chunk, i + 1, total, context);
        let note = ollama_generate(client, model, base, &prompt).await?;
        notes.push_str(&format!("[{}/{} 구간]\n{}\n\n", i + 1, total, note.trim()));
    }

    ollama_generate(
        client,
        model,
        base,
        &build_prompt(notes.trim(), context, Source::Notes),
    )
    .await
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicBlock>,
    stop_reason: Option<String>,
    #[serde(default)]
    stop_details: Option<AnthropicStopDetails>,
}

#[derive(Deserialize)]
struct AnthropicBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct AnthropicStopDetails {
    #[serde(default)]
    category: Option<String>,
}

async fn summarize_anthropic(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    transcript: &str,
    context: &Context,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("Anthropic API 키가 설정되지 않았습니다.".into());
    }

    let res = client
        .post(ANTHROPIC_ENDPOINT)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", ANTHROPIC_FALLBACK_BETA)
        .json(&serde_json::json!({
            "model": model,
            "max_tokens": ANTHROPIC_MAX_TOKENS,
            // 정책상 거절될 경우 권장 모델로 자동 재시도한다.
            "fallbacks": "default",
            "messages": [{ "role": "user", "content": build_prompt(transcript, context, Source::Transcript) }],
        }))
        .send()
        .await
        .map_err(|e| format!("Anthropic API에 연결할 수 없습니다: {e}"))?;

    let status = res.status();
    let body = res
        .text()
        .await
        .map_err(|e| format!("Anthropic 응답을 읽을 수 없습니다: {e}"))?;

    if !status.is_success() {
        // 키 문제는 사용자가 직접 고쳐야 하므로 따로 안내한다.
        let hint = match status.as_u16() {
            401 => " — API 키가 올바른지 확인해 주세요.",
            429 => " — 요청 한도에 걸렸습니다. 잠시 후 다시 시도해 주세요.",
            _ => "",
        };
        return Err(format!("Anthropic 오류 {status}{hint}: {body}"));
    }

    let parsed: AnthropicResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Anthropic 응답을 해석할 수 없습니다: {e}"))?;

    // 거절된 경우 content가 비어 있거나 부분적이다. 먼저 확인해야 한다.
    if parsed.stop_reason.as_deref() == Some("refusal") {
        let category = parsed
            .stop_details
            .and_then(|d| d.category)
            .unwrap_or_else(|| "미상".into());
        return Err(format!(
            "안전 정책에 의해 요약이 거절되었습니다 (분류: {category}). \
             로컬 요약으로 전환하거나 녹취 내용을 확인해 주세요."
        ));
    }

    let text: String = parsed
        .content
        .iter()
        .filter(|b| b.block_type == "text")
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("");

    if text.trim().is_empty() {
        return Err("Anthropic이 빈 응답을 반환했습니다.".into());
    }

    Ok(text.trim().to_string())
}

/// 선택된 백엔드로 회의록을 요약한다.
pub async fn summarize(
    backend: &Backend,
    transcript: &str,
    context: &Context,
) -> Result<String, String> {
    if transcript.trim().is_empty() {
        return Err("요약할 내용이 없습니다.".into());
    }

    let client = reqwest::Client::builder()
        // 로컬 모델은 긴 녹취록에서 수 분이 걸린다. 구간을 나눠 돌리면 호출이
        // 여러 번이라 더 길어진다 — 여기서 끊기면 회의록을 통째로 잃는다.
        .timeout(std::time::Duration::from_secs(1_800))
        .build()
        .map_err(|e| format!("HTTP 클라이언트를 만들 수 없습니다: {e}"))?;

    match backend {
        Backend::Ollama { model, endpoint } => {
            summarize_ollama(&client, model, endpoint.as_deref(), transcript, context).await
        }
        Backend::Anthropic { api_key, model } => {
            summarize_anthropic(&client, api_key, model, transcript, context).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_think_block() {
        assert_eq!(strip_think_tags("<think>고민</think>결과"), "결과");
    }

    #[test]
    fn strips_unclosed_think_block() {
        assert_eq!(strip_think_tags("앞<think>끝나지 않음"), "앞");
    }

    #[test]
    fn leaves_plain_text_alone() {
        assert_eq!(strip_think_tags("  회의록  "), "회의록");
    }

    #[test]
    fn short_meeting_stays_at_minimum_ctx() {
        assert_eq!(choose_num_ctx("짧은 회의"), OLLAMA_MIN_CTX);
    }

    #[test]
    fn ctx_covers_a_long_meeting() {
        // 실측 픽스처: 24,625자가 15,366토큰이었다. 기본 4096으로 돌리면
        // Ollama가 2,050토큰만 읽고 나머지를 앞에서부터 버렸다.
        let long = "가".repeat(24_625);
        let ctx = choose_num_ctx(&long);
        assert!(ctx > 15_366 + OLLAMA_OUTPUT_HEADROOM, "실측 토큰 수를 못 담는다: {ctx}");
    }

    #[test]
    fn short_meeting_does_not_reserve_the_maximum() {
        // 10분 남짓한 회의. 이런 건 32K를 잡을 이유가 없다.
        let ctx = choose_num_ctx(&"가".repeat(8_000));
        assert!(ctx < OLLAMA_MAX_CTX, "짧은 회의에 KV 캐시를 낭비한다: {ctx}");
    }

    #[test]
    fn splits_on_line_boundaries() {
        let text = "화자 1: 가나다\n화자 2: 라마바\n화자 1: 사아자";
        let chunks = split_transcript(text, 12);
        assert!(chunks.len() > 1, "잘리지 않았다");
        for chunk in &chunks {
            assert!(chunk.lines().all(|l| l.starts_with("화자")), "줄 가운데를 잘랐다: {chunk:?}");
        }
    }

    /// 나눠도 내용이 사라지면 안 된다. 잘라 버리는 예전 동작으로 돌아가는 걸
    /// 막는 회귀 테스트다.
    #[test]
    fn split_keeps_every_line() {
        let text: String = (0..200).map(|i| format!("화자 1: 발언 {i}\n")).collect();
        let chunks = split_transcript(&text, 100);
        let rejoined = chunks.join("\n");
        for i in 0..200 {
            assert!(rejoined.contains(&format!("발언 {i}")), "{i}번째 발언이 사라졌다");
        }
    }

    /// 화자분리에 실패하면 녹취록 전체가 한 줄로 온다. 그래도 나눠져야 한다 —
    /// 안 나눠지면 컨텍스트를 넘겨 앞부분이 잘린다.
    #[test]
    fn splits_a_single_line_transcript() {
        let long: String = (0..2_000).map(|i| format!("낱말{i} ")).collect();
        let chunks = split_transcript(&long, 500);
        assert!(chunks.len() > 1, "한 줄짜리 녹취록이 나눠지지 않았다");
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 500, "구간이 상한을 넘는다");
        }
        let rejoined: String = chunks.join("");
        for i in 0..2_000 {
            assert!(rejoined.contains(&format!("낱말{i} ")), "낱말{i}이 깨졌다");
        }
    }

    /// 띄어쓰기가 없어 끊을 자리를 못 찾아도 버리지는 않는다.
    #[test]
    fn splits_text_without_any_whitespace() {
        let solid = "가".repeat(1_000);
        let chunks = split_transcript(&solid, 100);
        assert_eq!(chunks.concat(), solid, "글자가 사라졌다");
    }

    #[test]
    fn short_transcript_is_not_split() {
        let chunks = split_transcript("화자 1: 짧다", OLLAMA_CHUNK_CHARS);
        assert_eq!(chunks.len(), 1);
    }

    /// 구간 하나가 컨텍스트 상한에 들어가야 나누는 의미가 있다.
    #[test]
    fn a_chunk_fits_in_one_context() {
        let chunk = "가".repeat(OLLAMA_CHUNK_CHARS);
        let prompt = build_notes_prompt(&chunk, 1, 5, &Context::default());
        assert!(fits_in_one_context(&prompt), "구간을 나눠도 여전히 상한에 걸린다");
    }

    /// 실측한 67분 회의는 31,584자로 한 번에 들어갔다. 나누는 길은 그보다 긴
    /// 회의를 위한 것이다 — 이 크기면 상한을 넘는다.
    #[test]
    fn a_long_meeting_needs_splitting() {
        let transcript = "가".repeat(83_185);
        let prompt = build_prompt(&transcript, &Context::default(), Source::Transcript);
        assert!(!fits_in_one_context(&prompt), "상한에 걸리지 않는다");
        assert!(split_transcript(&transcript, OLLAMA_CHUNK_CHARS).len() > 1);
    }

    #[test]
    fn ctx_is_capped() {
        assert_eq!(choose_num_ctx(&"가".repeat(500_000)), OLLAMA_MAX_CTX);
    }
}
