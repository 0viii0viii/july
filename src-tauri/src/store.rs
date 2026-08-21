//! 회의 기록 보관.
//!
//! 회의록 앱의 값어치는 지난 회의를 다시 찾아볼 수 있다는 데 있다. 그래서
//! 전사·요약 결과를 앱 데이터 디렉터리의 JSON 하나에 쌓아 둔다.
//!
//! DB를 쓰지 않는 이유는 단순하다. 회의는 하루에 몇 건이고, 몇 년을 써도
//! 수천 건이다. 파일 하나면 충분하고, 사용자가 직접 열어보거나 백업하기도
//! 쉽다.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::summarize::Context;
use crate::transcribe::Segment;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    /// 사용자가 붙인 제목. 비어 있으면 UI가 날짜로 대신 표시한다.
    #[serde(default)]
    pub title: String,
    /// ISO 8601 로컬 시각. 프론트엔드가 만들어 넘긴다.
    pub recorded_at: String,
    /// 원본 음성 경로. 파일이 지워졌을 수도 있으므로 존재를 보장하지 않는다.
    #[serde(default)]
    pub audio_path: Option<String>,
    pub duration: f64,
    pub segments: Vec<Segment>,
    /// 이 회의의 화자 라벨("화자 1")이 명단의 누구인지. 사용자가 지정한다.
    ///
    /// 회의마다 따로 두는 이유는 화자분리가 회의마다 번호를 새로 매기기
    /// 때문이다 — 어제의 "화자 1"과 오늘의 "화자 1"은 다른 사람이다.
    ///
    /// 이름이 아니라 id를 담는다. 명단에서 이름을 고쳐도 지난 회의의 연결이
    /// 그대로 따라오게 하려는 것이다. 지정하지 않은 화자는 여기 없다.
    #[serde(default)]
    pub speakers: BTreeMap<String, String>,
    /// 요약 마크다운. 요약 단계에서 실패했으면 None.
    #[serde(default)]
    pub summary: Option<String>,
    /// 처리 직전에 사용자가 입력한 그 회의의 배경 정보. 나중에 다시 요약할
    /// 때 같은 맥락을 그대로 쓰기 위해 함께 보관한다.
    #[serde(default)]
    pub context: Context,
}

impl Meeting {
    /// 목록에 보여줄 한 줄 미리보기.
    fn preview(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.trim())
            .find(|t| !t.is_empty())
            .unwrap_or("")
            .chars()
            .take(80)
            .collect()
    }
}

/// 목록 화면에 필요한 만큼만 담은 요약본. 전사 전문을 다 보내지 않는다.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingBrief {
    pub id: String,
    pub title: String,
    pub recorded_at: String,
    pub duration: f64,
    pub preview: String,
    pub has_summary: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Archive {
    #[serde(default)]
    meetings: Vec<Meeting>,
}

fn archive_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("앱 데이터 폴더를 찾을 수 없습니다: {e}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("앱 데이터 폴더를 만들 수 없습니다: {e}"))?;
    Ok(dir.join("meetings.json"))
}

fn read_archive(app: &AppHandle) -> Result<Archive, String> {
    let path = archive_path(app)?;
    if !path.exists() {
        return Ok(Archive::default());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("회의 기록을 읽을 수 없습니다: {e}"))?;
    // 파일이 손상됐다고 앱을 못 쓰게 만들 수는 없다. 빈 목록으로 시작하되
    // 원본은 남겨 둔다 (아래 write에서 백업을 만든다).
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn write_archive(app: &AppHandle, archive: &Archive) -> Result<(), String> {
    let path = archive_path(app)?;
    let json = serde_json::to_string_pretty(archive)
        .map_err(|e| format!("회의 기록을 직렬화할 수 없습니다: {e}"))?;

    // 임시 파일에 쓰고 교체한다. 쓰는 도중 앱이 죽어도 기존 기록이 날아가지
    // 않는다.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("회의 기록을 저장할 수 없습니다: {e}"))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("회의 기록을 교체할 수 없습니다: {e}"))?;
    Ok(())
}

/// 최근 회의부터 나열한다.
pub fn list(app: &AppHandle) -> Result<Vec<MeetingBrief>, String> {
    let archive = read_archive(app)?;
    let mut briefs: Vec<MeetingBrief> = archive
        .meetings
        .iter()
        .map(|m| MeetingBrief {
            id: m.id.clone(),
            title: m.title.clone(),
            recorded_at: m.recorded_at.clone(),
            duration: m.duration,
            preview: m.preview(),
            has_summary: m.summary.is_some(),
        })
        .collect();
    // ISO 8601은 문자열 정렬이 곧 시간 정렬이다.
    briefs.sort_by(|a, b| b.recorded_at.cmp(&a.recorded_at));
    Ok(briefs)
}

pub fn get(app: &AppHandle, id: &str) -> Result<Option<Meeting>, String> {
    Ok(read_archive(app)?.meetings.into_iter().find(|m| m.id == id))
}

/// 같은 id가 있으면 덮어쓴다 (제목 수정·요약 재생성 등).
pub fn save(app: &AppHandle, meeting: Meeting) -> Result<(), String> {
    let mut archive = read_archive(app)?;
    match archive.meetings.iter_mut().find(|m| m.id == meeting.id) {
        Some(existing) => *existing = meeting,
        None => archive.meetings.push(meeting),
    }
    write_archive(app, &archive)
}

pub fn rename(app: &AppHandle, id: &str, title: &str) -> Result<(), String> {
    let mut archive = read_archive(app)?;
    let target = archive
        .meetings
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or("해당 회의를 찾을 수 없습니다.")?;
    target.title = title.trim().to_string();
    write_archive(app, &archive)
}

/// 기록을 지운다. 원본 음성 파일도 함께 지운다.
pub fn remove(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut archive = read_archive(app)?;
    let Some(pos) = archive.meetings.iter().position(|m| m.id == id) else {
        return Ok(());
    };
    let gone = archive.meetings.remove(pos);
    write_archive(app, &archive)?;

    // 음성 파일 삭제 실패로 전체를 실패시키지는 않는다 — 기록은 이미 지웠다.
    if let Some(path) = gone.audio_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}
