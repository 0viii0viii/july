pub mod audio;
pub mod decode;
pub mod diarize;
pub mod catalog;
pub mod setup;
pub mod store;
pub mod permission;
pub mod roster;
pub mod summarize;
pub mod terms;
pub mod transcribe;

use std::collections::BTreeMap;
use std::path::PathBuf;

use audio::{InputDevice, Recorder, RecordingStatus};
use setup::{Environment, ModelSize};
use store::{Meeting, MeetingBrief};
use permission::MicPermission;
use roster::Person;
use summarize::{Backend, Context};
use tauri::{AppHandle, Manager, State};
use transcribe::{Segment, Transcript};

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

/// 마이크 권한 상태. 창을 띄우지 않으므로 화면을 그릴 때 물어도 된다.
#[tauri::command]
fn microphone_permission() -> MicPermission {
    permission::status()
}

/// 시스템 설정의 마이크 항목을 연다. 한 번 거부하면 권한 창이 다시 안 뜬다.
#[tauri::command]
fn open_microphone_settings() -> Result<(), String> {
    permission::open_settings()
}

/// **`async`가 아니라 `(async)`인 것이 중요하다.**
///
/// tauri의 `#[tauri::command]`는 기본이 `ExecutionContext::Blocking`이라 동기
/// 함수를 **메인 스레드에서** 실행한다. 그런데 `permission::request()`는 사용자가
/// 권한 창에 답할 때까지 최대 60초를 기다린다. 메인 스레드가 막히면 런루프가
/// 돌지 못하고, 런루프가 멈추면 macOS가 그 권한 창을 띄우지 못한다 — 띄워야
/// 풀리는 것을 띄우지 못해 60초를 기다렸다가 "미결정"으로 돌아오는 교착이다.
///
/// 0.1.11에서 이 교착 때문에 권한 창이 아예 뜨지 않았다. `(async)`를 붙이면
/// 블로킹 스레드풀에서 돌아 메인 런루프가 계속 살아 있다.
#[tauri::command(async)]
fn list_input_devices() -> Result<Vec<InputDevice>, String> {
    // **권한을 먼저 받아야 한다.** macOS는 권한 없는 앱에게 입력 장치를 숨기므로,
    // 그냥 훑으면 빈 목록이 나오고 앱은 "마이크가 없다"고 판단한다. 그러면 녹음
    // 버튼이 막혀 스트림을 열 일이 없고, 권한 창도 영영 뜨지 않는다 — 시스템
    // 설정의 마이크 목록에 앱이 나타나지도 않는다.
    //
    // 이미 결정된 상태라면 창 없이 즉시 돌아온다.
    let permission = permission::request();
    if !permission.usable() {
        return Ok(Vec::new());
    }
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
    let models_dir = model_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let audio_path = PathBuf::from(path);
    let context = context.unwrap_or_default();
    let hint = context.as_speech_hint();
    let user_terms = context.terms.clone();

    // whisper 추론은 CPU/GPU를 오래 점유하는 블로킹 작업이다. 전용 스레드로
    // 보내지 않으면 그동안 UI 이벤트가 멈춘다.
    tauri::async_runtime::spawn_blocking(move || {
        // 디코딩은 한 번만 한다 — 전사와 화자분리가 같은 샘플을 쓴다.
        let audio = decode::decode_to_mono_16k(&audio_path)?.samples;

        let mut result = transcribe::transcribe_samples(
            &model_path,
            &audio,
            language.as_deref(),
            hint.as_deref(),
        )?;

        // 화자분리는 실패해도 전사를 버리지 않는다. 모델을 아직 못 받았거나
        // 이 플랫폼에서 안 돌 수도 있는데, 화자 없는 회의록이 회의록이 없는
        // 것보다 낫다.
        match diarize::diarize(&audio, &models_dir) {
            Ok(turns) => diarize::assign(&mut result.segments, &turns),
            Err(e) => eprintln!("화자분리를 건너뜁니다: {e}"),
        }

        // 힌트만으로는 부족하다. initial_prompt는 첫 윈도우 위주로 작용해서 긴
        // 회의 뒤쪽에서 표기가 다시 흐트러진다. 세그먼트를 직접 고쳐두면 요약뿐
        // 아니라 화면에 보이는 녹취록도 같이 정리된다.
        for segment in &mut result.segments {
            segment.text = terms::correct(&segment.text, &user_terms);
        }
        Ok(result)
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

/// 요약 모델에 넘길 녹취록 평문을 만든다.
///
/// 프론트엔드가 직접 이어 붙이지 않고 여기를 거치는 이유는, 화자를 사람으로
/// 바꾸는 규칙이 한 곳에만 있어야 하기 때문이다. 화면에 보이는 이름과 모델이
/// 받는 이름이 어긋나면 요약의 담당자가 왜 그렇게 나왔는지 설명할 수 없게 된다.
#[tauri::command]
fn transcript_text(
    app: AppHandle,
    segments: Vec<Segment>,
    speakers: Option<BTreeMap<String, String>>,
) -> Result<String, String> {
    let speakers = speakers.unwrap_or_default();
    let names = roster::resolve(&roster::list(&app)?, &speakers);
    Ok(transcribe::plain_text(&segments, &names))
}

// ---------------------------------------------------------------- 사내 명단

#[tauri::command]
fn list_people(app: AppHandle) -> Result<Vec<Person>, String> {
    roster::list(&app)
}

/// 사람을 추가하거나 고친다. 저장된 결과를 돌려준다 — 새로 만든 경우
/// 프론트엔드가 발급된 id를 알아야 화자 지정에 쓸 수 있다.
#[tauri::command]
fn save_person(app: AppHandle, person: Person) -> Result<Person, String> {
    roster::save(&app, person)
}

#[tauri::command]
fn delete_person(app: AppHandle, id: String) -> Result<(), String> {
    roster::remove(&app, &id)
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
            microphone_permission,
            open_microphone_settings,
            start_recording,
            stop_recording,
            recording_status,
            transcribe_file,
            summarize_text,
            transcript_text,
            list_people,
            save_person,
            delete_person,
            list_meetings,
            get_meeting,
            save_meeting,
            rename_meeting,
            delete_meeting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
