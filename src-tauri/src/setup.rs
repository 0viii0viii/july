//! 첫 실행 준비 — 모델 내려받기와 환경 점검.
//!
//! 이 앱을 쓰는 사람이 터미널을 열 일은 없어야 한다. whisper 모델을 받는 것도,
//! Ollama가 떠 있는지 확인하는 것도 전부 앱 안에서 끝나야 한다.

use std::path::PathBuf;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

/// whisper 모델 카탈로그.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSize {
    /// 141MB. 빠르지만 한국어 정확도가 낮다 — 동작 확인용.
    Base,
    /// 1.6GB. 실사용 기본값.
    LargeV3Turbo,
}

impl ModelSize {
    pub fn filename(self) -> &'static str {
        match self {
            ModelSize::Base => "ggml-base.bin",
            ModelSize::LargeV3Turbo => "ggml-large-v3-turbo.bin",
        }
    }

    fn url(self) -> &'static str {
        match self {
            ModelSize::Base => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
            }
            ModelSize::LargeV3Turbo => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin"
            }
        }
    }

    /// 다운로드가 끝나기 전에 용량을 안내하기 위한 근사치(바이트).
    pub fn approx_bytes(self) -> u64 {
        match self {
            ModelSize::Base => 148_000_000,
            ModelSize::LargeV3Turbo => 1_624_000_000,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ModelSize::Base => "base",
            ModelSize::LargeV3Turbo => "large-v3-turbo",
        }
    }
}

/// 모델을 두는 곳.
///
/// 개발 중에는 저장소의 `models/`를 쓰고, 배포판에서는 앱 데이터 디렉터리를
/// 쓴다. `MEETNOTE_MODELS_DIR`로 강제할 수 있다.
pub fn models_dir(app: &AppHandle) -> PathBuf {
    if let Ok(dir) = std::env::var("MEETNOTE_MODELS_DIR") {
        return PathBuf::from(dir);
    }

    // 저장소 안에서 실행 중이고 models/가 이미 있으면 그걸 재사용한다.
    // 개발자가 받아둔 1.6GB를 또 받게 하지 않기 위한 것이다.
    let repo_models = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("models"));
    if let Some(dir) = repo_models {
        if dir.is_dir() {
            return dir;
        }
    }

    app.path()
        .app_data_dir()
        .map(|d| d.join("models"))
        .unwrap_or_else(|_| PathBuf::from("models"))
}

pub fn model_path(app: &AppHandle, model: ModelSize) -> PathBuf {
    models_dir(app).join(model.filename())
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub id: ModelSize,
    pub label: &'static str,
    pub installed: bool,
    pub approx_bytes: u64,
    /// 실제 파일 크기. 미설치면 0.
    pub bytes_on_disk: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub model: ModelSize,
    pub received: u64,
    /// 서버가 길이를 안 주면 None.
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub models: Vec<ModelStatus>,
    /// 전사가 가능한 상태인지 — 모델이 하나라도 있어야 한다.
    pub can_transcribe: bool,
    pub ollama_running: bool,
    pub ollama_models: Vec<String>,
    pub models_path: String,
    pub hardware: crate::catalog::Hardware,
    /// 기기 사양과 설치 여부가 반영된 요약 모델 목록.
    pub summarizers: Vec<crate::catalog::CatalogEntry>,
}

pub fn model_statuses(app: &AppHandle) -> Vec<ModelStatus> {
    [ModelSize::LargeV3Turbo, ModelSize::Base]
        .into_iter()
        .map(|m| {
            let path = model_path(app, m);
            let bytes = std::fs::metadata(&path).map(|md| md.len()).unwrap_or(0);
            // 받다 만 파일을 "설치됨"으로 보면 전사가 이상하게 실패한다.
            // 근사치의 90%는 넘어야 온전한 것으로 취급한다.
            let installed = bytes > m.approx_bytes() / 10 * 9;
            ModelStatus {
                id: m,
                label: m.label(),
                installed,
                approx_bytes: m.approx_bytes(),
                bytes_on_disk: bytes,
            }
        })
        .collect()
}

#[derive(Deserialize)]
struct OllamaTags {
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
}

pub async fn ollama_models(endpoint: Option<&str>) -> Result<Vec<String>, String> {
    let base = endpoint
        .unwrap_or("http://localhost:11434")
        .trim_end_matches('/');

    let res = reqwest::Client::new()
        .get(format!("{base}/api/tags"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map_err(|e| format!("Ollama에 연결할 수 없습니다: {e}"))?;

    let tags: OllamaTags = res
        .json()
        .await
        .map_err(|e| format!("Ollama 응답을 해석할 수 없습니다: {e}"))?;

    Ok(tags.models.into_iter().map(|m| m.name).collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaPullProgress {
    pub model: String,
    /// Ollama가 알려주는 현재 단계 ("pulling manifest", "downloading ..." 등).
    pub status: String,
    pub completed: u64,
    pub total: u64,
}

/// Ollama에게 모델을 받아오게 하고 진행 상황을 `ollama-pull` 이벤트로 흘린다.
///
/// `/api/pull`은 NDJSON 스트림을 돌려준다 — 한 줄에 JSON 하나씩. 레이어마다
/// total/completed가 새로 시작하므로 그대로 전달하고 표시는 프론트엔드에
/// 맡긴다.
pub async fn pull_ollama_model(
    app: &AppHandle,
    model: &str,
    endpoint: Option<&str>,
) -> Result<(), String> {
    let base = endpoint
        .unwrap_or("http://localhost:11434")
        .trim_end_matches('/');

    let res = reqwest::Client::new()
        .post(format!("{base}/api/pull"))
        .json(&serde_json::json!({ "model": model, "stream": true }))
        .send()
        .await
        .map_err(|e| format!("Ollama에 연결할 수 없습니다: {e}"))?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("모델을 받을 수 없습니다 (HTTP {status}): {body}"));
    }

    let mut stream = res.bytes_stream();
    // 줄 단위로 끊어 읽어야 한다. 청크 경계가 줄 중간에 걸릴 수 있다.
    let mut buf = String::new();
    let mut last_error: Option<String> = None;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("다운로드가 끊겼습니다: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim().to_string();
            buf.drain(..=nl);
            if line.is_empty() {
                continue;
            }

            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };

            // Ollama는 실패를 200 응답 본문 안에 error 필드로 담아 보낸다.
            if let Some(err) = value.get("error").and_then(|e| e.as_str()) {
                last_error = Some(err.to_string());
                continue;
            }

            let _ = app.emit(
                "ollama-pull",
                OllamaPullProgress {
                    model: model.to_string(),
                    status: value
                        .get("status")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    completed: value.get("completed").and_then(|c| c.as_u64()).unwrap_or(0),
                    total: value.get("total").and_then(|t| t.as_u64()).unwrap_or(0),
                },
            );
        }
    }

    match last_error {
        Some(e) => Err(format!("모델을 받지 못했습니다: {e}")),
        None => Ok(()),
    }
}

/// 앱을 쓸 수 있는 상태인지 한 번에 점검한다.
pub async fn inspect(app: &AppHandle) -> Environment {
    let models = model_statuses(app);
    let can_transcribe = models.iter().any(|m| m.installed);
    let ollama = ollama_models(None).await;
    let installed = ollama.clone().unwrap_or_default();

    let hardware = crate::catalog::hardware();
    let summarizers = crate::catalog::catalog(&hardware, &installed);

    Environment {
        models,
        can_transcribe,
        ollama_running: ollama.is_ok(),
        ollama_models: installed,
        models_path: models_dir(app).to_string_lossy().into_owned(),
        hardware,
        summarizers,
    }
}

/// 모델을 내려받으면서 진행 상황을 `model-download` 이벤트로 흘려보낸다.
///
/// 부분 파일(`.part`)에 받은 뒤 완료 시점에 이름을 바꾼다. 중간에 앱이 죽어도
/// 반쪽짜리 모델이 정상인 척 남지 않게 하기 위해서다.
pub async fn download_model(app: &AppHandle, model: ModelSize) -> Result<(), String> {
    let dir = models_dir(app);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("모델 폴더를 만들 수 없습니다 ({}): {e}", dir.display()))?;

    let final_path = dir.join(model.filename());
    if final_path.exists() {
        return Ok(());
    }
    let part_path = final_path.with_extension("part");

    let res = reqwest::Client::builder()
        .build()
        .map_err(|e| format!("HTTP 클라이언트를 만들 수 없습니다: {e}"))?
        .get(model.url())
        .send()
        .await
        .map_err(|e| format!("모델 서버에 연결할 수 없습니다: {e}"))?;

    if !res.status().is_success() {
        return Err(format!("모델을 내려받을 수 없습니다 (HTTP {})", res.status()));
    }

    let total = res.content_length();
    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("모델 파일을 만들 수 없습니다: {e}"))?;

    let mut received: u64 = 0;
    let mut last_emit = 0u64;
    let mut stream = res.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("다운로드가 끊겼습니다: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("모델을 저장할 수 없습니다: {e}"))?;
        received += chunk.len() as u64;

        // 청크마다 이벤트를 쏘면 프론트엔드가 과부하된다. 2MB마다 한 번.
        if received - last_emit > 2_000_000 {
            last_emit = received;
            let _ = app.emit(
                "model-download",
                DownloadProgress {
                    model,
                    received,
                    total,
                },
            );
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("모델 저장을 마무리할 수 없습니다: {e}"))?;
    drop(file);

    tokio::fs::rename(&part_path, &final_path)
        .await
        .map_err(|e| format!("모델 파일 이름을 바꿀 수 없습니다: {e}"))?;

    let _ = app.emit(
        "model-download",
        DownloadProgress {
            model,
            received,
            total,
        },
    );

    Ok(())
}
