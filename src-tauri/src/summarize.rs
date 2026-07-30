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

fn build_prompt(transcript: &str, context: &Context) -> String {
    let briefing = context.as_briefing();
    let owner_rule = if context.attendees.trim().is_empty() {
        "- 담당자나 기한이 언급되지 않았다면 '미정'으로 적으세요.\n"
    } else {
        "- 액션 아이템의 담당자는 위 참석자 명단의 이름으로 적으세요. \
         녹취록에서 누구인지 알 수 없으면 '미정'으로 두세요.\n"
    };

    format!(
        "{briefing}\
         다음은 회의 녹취록입니다. 이를 회의록으로 정리해 주세요.\n\
         \n\
         형식:\n\
         ## 한 줄 요약\n\
         ## 주요 논의\n\
         ## 결정 사항\n\
         ## 액션 아이템\n\
         - [담당자] 할 일 (기한)\n\
         \n\
         규칙:\n\
         - 녹취록에 없는 내용을 지어내지 마세요.\n\
         {owner_rule}\
         - 음성 인식 오류로 보이는 부분은 문맥에 맞게 자연스럽게 고쳐도 됩니다. \
         특히 위에 적힌 이름과 용어는 그 표기를 따르세요.\n\
         - 한국어로 작성하세요.\n\
         \n\
         ---\n\
         {transcript}"
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

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

async fn summarize_ollama(
    client: &reqwest::Client,
    model: &str,
    endpoint: Option<&str>,
    transcript: &str,
    context: &Context,
) -> Result<String, String> {
    let base = endpoint.unwrap_or(OLLAMA_DEFAULT_ENDPOINT).trim_end_matches('/');
    let url = format!("{base}/api/generate");

    let res = client
        .post(&url)
        .json(&serde_json::json!({
            "model": model,
            "prompt": build_prompt(transcript, context),
            "stream": false,
            // 추론 모델의 사고 과정은 회의록에 불필요하다.
            "think": false,
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
            "messages": [{ "role": "user", "content": build_prompt(transcript, context) }],
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
        // 로컬 모델은 긴 녹취록에서 수 분이 걸릴 수 있다.
        .timeout(std::time::Duration::from_secs(600))
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
}
