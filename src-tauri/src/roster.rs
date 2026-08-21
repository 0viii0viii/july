//! 사내 명단 — 누가 있고 누구 밑에 있는지.
//!
//! 팀에서 이 앱을 계속 쓰면 참석자는 매번 거의 같은 사람들인데, 지금은 회의마다
//! 이름을 손으로 다시 친다. 한 번 오타가 나면 요약의 담당자 이름이 회의마다
//! 달라지고, 나중에 "내가 뭘 맡았더라"를 모아 보려 해도 묶이지 않는다.
//!
//! 그래서 사람을 한 번 등록해두고 회의에서는 고르기만 한다.
//!
//! 상위자를 함께 두는 이유는 조직도로 그리기 위해서다. 명단이 스무 명을 넘으면
//! 이름만 나열된 목록에서는 원하는 사람을 찾지 못한다 — 팀과 계층으로 접어야
//! 찾을 수 있다.
//!
//! 목소리는 여기 없다. 성문 등록은 다음 단계이고, 그때 이 구조체에 프로필이
//! 붙는다. 지금은 사람이 화자를 직접 지정한다.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    /// 앱이 만드는 안정적인 식별자. 이름이 바뀌어도 회의 기록의 연결이 끊기지
    /// 않게 이름이 아니라 id로 참조한다.
    pub id: String,
    pub name: String,
    /// 부서·팀. 조직도에서 묶는 단위이자 동명이인을 구분하는 단서다.
    #[serde(default)]
    pub team: String,
    /// 직책. 요약 모델에 넘기면 액션 아이템의 담당자 배정이 눈에 띄게 나아진다.
    #[serde(default)]
    pub title: String,
    /// 상위자 id. 최상위이거나 모르면 `None`.
    #[serde(default)]
    pub manager: Option<String>,
}

impl Person {
    /// 요약 모델과 화면에 보여줄 표기. "김서연(팀장)" 형태.
    ///
    /// 직책을 함께 넘기는 것이 중요하다. 모델은 "누가 결정권자인가"를 직책에서
    /// 읽는다 — 이름만 주면 발언의 무게를 구분하지 못한다.
    pub fn display(&self) -> String {
        if self.title.trim().is_empty() {
            self.name.clone()
        } else {
            format!("{}({})", self.name, self.title.trim())
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    people: Vec<Person>,
}

fn book_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("앱 데이터 폴더를 찾을 수 없습니다: {e}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("앱 데이터 폴더를 만들 수 없습니다: {e}"))?;
    Ok(dir.join("roster.json"))
}

fn read_book(app: &AppHandle) -> Result<Book, String> {
    let path = book_path(app)?;
    if !path.exists() {
        return Ok(Book::default());
    }
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("명단을 읽을 수 없습니다: {e}"))?;
    // 회의 기록과 같은 태도다 — 파일이 깨졌다고 앱 전체를 못 쓰게 만들지 않는다.
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn write_book(app: &AppHandle, book: &Book) -> Result<(), String> {
    let path = book_path(app)?;
    let json = serde_json::to_string_pretty(book)
        .map_err(|e| format!("명단을 직렬화할 수 없습니다: {e}"))?;
    // 쓰는 도중 죽어도 기존 명단이 날아가지 않게 임시 파일에 쓰고 교체한다.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("명단을 저장할 수 없습니다: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("명단을 교체할 수 없습니다: {e}"))?;
    Ok(())
}

/// 팀, 이름 순으로 정렬해 돌려준다. 목록은 항상 같은 순서로 보여야 한다.
pub fn list(app: &AppHandle) -> Result<Vec<Person>, String> {
    let mut people = read_book(app)?.people;
    people.sort_by(|a, b| a.team.cmp(&b.team).then_with(|| a.name.cmp(&b.name)));
    Ok(people)
}

/// 상위자를 따라 올라가다 `target`을 만나면 순환이다.
///
/// A의 상위자를 B로, B의 상위자를 A로 지정하면 조직도를 그리는 쪽이 무한히
/// 내려간다. 그리는 쪽에서 막을 수도 있지만 애초에 저장되지 않게 하는 편이
/// 낫다 — 이미 저장된 순환은 어느 화면에서 터질지 알 수 없다.
fn reaches(people: &[Person], from: &str, target: &str) -> bool {
    let mut seen = HashSet::new();
    let mut cursor = Some(from.to_string());
    while let Some(id) = cursor {
        if id == target {
            return true;
        }
        // 이미 저장된 파일이 순환을 담고 있어도 여기서 멈춘다.
        if !seen.insert(id.clone()) {
            return false;
        }
        cursor = people
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.manager.clone());
    }
    false
}

/// 사람을 추가하거나 고친다. id가 비어 있으면 새로 만든다.
///
/// 저장된 결과를 돌려주는 이유는 새로 만들 때 프론트엔드가 발급된 id를 알아야
/// 하기 때문이다.
pub fn save(app: &AppHandle, mut person: Person) -> Result<Person, String> {
    person.name = person.name.trim().to_string();
    person.team = person.team.trim().to_string();
    person.title = person.title.trim().to_string();

    if person.name.is_empty() {
        return Err("이름을 입력해 주세요.".into());
    }

    let mut book = read_book(app)?;

    if person.id.trim().is_empty() {
        person.id = new_id();
    }

    // 빈 문자열로 온 상위자는 "없음"이다. 그대로 두면 존재하지 않는 사람을
    // 가리키는 포인터가 된다.
    if let Some(manager) = person.manager.as_deref() {
        if manager.trim().is_empty() || manager == person.id {
            person.manager = None;
        }
    }
    if let Some(manager) = person.manager.clone() {
        if !book.people.iter().any(|p| p.id == manager) {
            return Err("상위자를 명단에서 찾을 수 없습니다.".into());
        }
        if reaches(&book.people, &manager, &person.id) {
            return Err("상위자를 그렇게 지정하면 조직도가 순환합니다.".into());
        }
    }

    match book.people.iter_mut().find(|p| p.id == person.id) {
        Some(existing) => *existing = person.clone(),
        None => book.people.push(person.clone()),
    }
    write_book(app, &book)?;
    Ok(person)
}

/// 명단에서 지운다.
///
/// 이 사람을 상위자로 가리키던 사람들은 상위자가 없는 상태가 된다. 남겨두면
/// 조직도에서 그 가지 전체가 사라진다 — 팀장이 퇴사했다고 팀원이 화면에서
/// 없어지면 안 된다.
///
/// 지난 회의의 화자 지정은 건드리지 않는다. 그 회의에서 그 사람이 말한 것은
/// 사실이고, 명단에서 빠졌다고 기록을 고쳐 쓸 이유가 없다. 화면에서는 이름을
/// 찾지 못해 원래 라벨("화자 1")로 되돌아간다.
pub fn remove(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut book = read_book(app)?;
    book.people.retain(|p| p.id != id);
    for person in &mut book.people {
        if person.manager.as_deref() == Some(id) {
            person.manager = None;
        }
    }
    write_book(app, &book)
}

/// `화자 라벨 → 사람 id` 지정을 `화자 라벨 → 표기`로 바꾼다.
///
/// 명단에 없는 id는 조용히 버린다 — 지워진 사람을 가리키는 지정이다. 버리면
/// 그 화자는 원래 라벨로 보인다.
pub fn resolve(
    people: &[Person],
    speakers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    speakers
        .iter()
        .filter_map(|(label, id)| {
            people
                .iter()
                .find(|p| &p.id == id)
                .map(|p| (label.clone(), p.display()))
        })
        .collect()
}

/// 의존성을 늘리지 않으려고 직접 만든다. 명단 id는 이 기기 안에서만 유일하면
/// 되므로 충돌 확률만 충분히 낮으면 된다.
fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    // 같은 나노초에 두 번 불릴 수 있다. 호출마다 오르는 수를 섞어 구분한다.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("p{nanos:x}-{seq:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: &str, name: &str, manager: Option<&str>) -> Person {
        Person {
            id: id.into(),
            name: name.into(),
            team: String::new(),
            title: String::new(),
            manager: manager.map(str::to_string),
        }
    }

    #[test]
    fn display_includes_title() {
        let mut p = person("1", "김서연", None);
        assert_eq!(p.display(), "김서연");
        p.title = "팀장".into();
        assert_eq!(p.display(), "김서연(팀장)");
    }

    /// 가→나→다 사슬에서 다를 가의 상위자로 두면 순환이다.
    #[test]
    fn detects_cycle_through_chain() {
        let people = vec![
            person("a", "가", Some("b")),
            person("b", "나", Some("c")),
            person("c", "다", None),
        ];
        assert!(reaches(&people, "a", "c"), "가의 위로 올라가면 다를 만난다");
        assert!(!reaches(&people, "c", "a"), "다의 위에는 아무도 없다");
    }

    /// 이미 저장된 파일이 순환을 담고 있어도 멈춰야 한다.
    #[test]
    fn survives_preexisting_cycle() {
        let people = vec![person("a", "가", Some("b")), person("b", "나", Some("a"))];
        assert!(!reaches(&people, "a", "없는사람"), "무한히 돌지 않는다");
    }

    #[test]
    fn resolve_drops_unknown_people() {
        let people = vec![person("a", "가", None)];
        let speakers = BTreeMap::from([
            ("화자 1".to_string(), "a".to_string()),
            ("화자 2".to_string(), "지워진사람".to_string()),
        ]);

        let names = resolve(&people, &speakers);
        assert_eq!(names.get("화자 1").map(String::as_str), Some("가"));
        assert_eq!(names.get("화자 2"), None, "명단에 없으면 라벨로 남는다");
    }

    #[test]
    fn ids_are_unique_across_calls() {
        assert_ne!(new_id(), new_id());
    }
}
