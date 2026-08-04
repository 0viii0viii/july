pub mod audio;
pub mod decode;
pub mod catalog;
pub mod setup;
pub mod store;
pub mod summarize;
pub mod transcribe;

use std::path::PathBuf;

use audio::{InputDevice, Recorder, RecordingStatus};
use setup::{Environment, ModelSize};
use store::{Meeting, MeetingBrief};
use summarize::{Backend, Context};
use tauri::{AppHandle, Manager, State};
use transcribe::Transcript;

// ---------------------------------------------------------------- 준비 상태

#[tauri::command]
async fn inspect_environment(app: AppHandle) -> Environment {
    setup::inspect(&app).await
}

#[tauri::command]
async fn download_model(app: AppHandle, model: ModelSize) -> Result<(), String> {
    setup::download_model(&app, model).await
}

#[tauri::command]
async fn list_ollama_models(endpoint: Option<String>) -> Result<Vec<String>, String> {
    setup::ollama_models(endpoint.as_deref()).await
}

/// 요약 모델을 Ollama에 내려받는다. 진행 상황은 `ollama-pull` 이벤트로 온다.
#[tauri::command]
async fn pull_summarizer(
    app: AppHandle,
    model: String,
    endpoint: Option<String>,
) -> Result<(), String> {
    setup::pull_ollama_model(&app, &model, endpoint.as_deref()).await
}

// -------------------------------------------------------------------- 녹음

#[tauri::command]
fn list_input_devices() -> Result<Vec<InputDevice>, String> {
    audio::input_devices()
}

/// 녹음을 시작하고 저장될 파일 경로를 돌려준다.
///
/// 파일은 앱 데이터 디렉터리 아래 `recordings/`에 타임스탬프 이름으로 남는다.
/// 앱이 갑자기 종료돼도 녹음이 사라지지 않게 하려는 것이다. `stamp`는 파일
/// 이름에 쓸 타임스탬프로, 프론트엔드가 로컬 시간대로 만들어 넘긴다.
#[tauri::command]
fn start_recording(
    app: AppHandle,
    recorder: State<'_, Recorder>,
    device: Option<String>,
    stamp: String,
) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("앱 데이터 폴더를 찾을 수 없습니다: {e}"))?
        .join("recordings");

    // 경로 조작을 막기 위해 타임스탬프에서 안전한 문자만 남긴다.
    let safe: String = stamp
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    let name = if safe.is_empty() {
        "recording".to_string()
    } else {
        safe
    };

    let path = recorder.start(&dir.join(format!("{name}.wav")), device.as_deref())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn stop_recording(recorder: State<'_, Recorder>) -> Result<String, String> {
    Ok(recorder.stop()?.to_string_lossy().into_owned())
}

#[tauri::command]
fn recording_status(recorder: State<'_, Recorder>) -> RecordingStatus {
    recorder.status()
}

// ------------------------------------------------------------- 전사 / 요약

/// `context`는 그 회의의 참석자·주제·용어다. 전역 설정이 아니라 회의마다
/// 받아서, 전사에서는 표기 고정에 쓰고 요약에서는 배경 지식으로 쓴다.
#[tauri::command]
async fn transcribe_file(
    app: AppHandle,
    path: String,
    model: ModelSize,
    language: Option<String>,
    context: Option<Context>,
) -> Result<Transcript, String> {
    let model_path = setup::model_path(&app, model);
    let audio_path = PathBuf::from(path);
    let hint = context.and_then(|c| c.as_speech_hint());

    // whisper 추론은 CPU/GPU를 오래 점유하는 블로킹 작업이다. 전용 스레드로
    // 보내지 않으면 그동안 UI 이벤트가 멈춘다.
    tauri::async_runtime::spawn_blocking(move || {
        transcribe::transcribe(
            &model_path,
            &audio_path,
            language.as_deref(),
            hint.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("전사 작업이 중단되었습니다: {e}"))?
}

#[tauri::command]
async fn summarize_text(
    backend: Backend,
    transcript: String,
    context: Option<Context>,
) -> Result<String, String> {
    summarize::summarize(&backend, &transcript, &context.unwrap_or_default()).await
}

// -------------------------------------------------------------- 회의 보관함

#[tauri::command]
fn list_meetings(app: AppHandle) -> Result<Vec<MeetingBrief>, String> {
    store::list(&app)
}

#[tauri::command]
fn get_meeting(app: AppHandle, id: String) -> Result<Option<Meeting>, String> {
    store::get(&app, &id)
}

#[tauri::command]
fn save_meeting(app: AppHandle, meeting: Meeting) -> Result<(), String> {
    store::save(&app, meeting)
}

#[tauri::command]
fn rename_meeting(app: AppHandle, id: String, title: String) -> Result<(), String> {
    store::rename(&app, &id, &title)
}

#[tauri::command]
fn delete_meeting(app: AppHandle, id: String) -> Result<(), String> {
    store::remove(&app, &id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(Recorder::new())
        .invoke_handler(tauri::generate_handler![
            inspect_environment,
            download_model,
            list_ollama_models,
            pull_summarizer,
            list_input_devices,
            start_recording,
            stop_recording,
            recording_status,
            transcribe_file,
            summarize_text,
            list_meetings,
            get_meeting,
            save_meeting,
            rename_meeting,
            delete_meeting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
