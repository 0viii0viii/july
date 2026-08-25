/** Rust 쪽 커맨드와 타입을 한곳에 모아둔다. */
import { invoke } from "@tauri-apps/api/core";

export type ModelSize = "base" | "large_v3_turbo";

export type ModelStatus = {
  id: ModelSize;
  label: string;
  installed: boolean;
  approx_bytes: number;
  bytes_on_disk: number;
};

export type Hardware = {
  totalMemoryBytes: number;
  cpu: string;
  /** 요약 모델에 쓸 수 있다고 보는 메모리 예산. */
  modelBudgetBytes: number;
};

/** 이 기기에서 편하게 도는지. */
export type Fit = "comfortable" | "tight" | "too_big";
/** 요약 한 건에 걸리는 체감 시간. */
export type Speed = "fast" | "balanced" | "slow";

export type Summarizer = {
  /** `ollama pull`에 그대로 넘기는 이름. */
  id: string;
  name: string;
  maker: string;
  bytes: number;
  note: string;
  /** 이 앱에서 실제로 한국어 회의록 요약을 돌려본 모델인지. */
  verified: boolean;
  fit: Fit;
  speed: Speed;
  recommended: boolean;
  installed: boolean;
};

export type Environment = {
  models: ModelStatus[];
  canTranscribe: boolean;
  ollamaRunning: boolean;
  ollamaModels: string[];
  modelsPath: string;
  hardware: Hardware;
  summarizers: Summarizer[];
};

export type PullProgress = {
  model: string;
  status: string;
  completed: number;
  total: number;
};

export const pullSummarizer = (model: string) =>
  invoke<void>("pull_summarizer", { model, endpoint: null });

export type DownloadProgress = {
  model: ModelSize;
  received: number;
  total: number | null;
};

export type InputDevice = { name: string; is_default: boolean };

export type RecordingStatus = {
  recording: boolean;
  /** 0.0 ~ 1.0 */
  level: number;
  seconds: number;
  /** 시작 후 계속 무음 — 마이크 권한이나 입력 선택 문제일 수 있다. */
  silent: boolean;
};

export type Segment = {
  start: number;
  end: number;
  text: string;
  /**
   * 화자분리가 붙인 라벨("화자 1"). 화자분리를 돌리지 않았거나 어느 화자
   * 구간에도 걸치지 않으면 null이다 — 모르면 지어내지 않는다.
   *
   * 이건 회의 안에서만 유효한 번호다. 어제의 "화자 1"과 오늘의 "화자 1"은
   * 다른 사람이다. 사람과 잇는 것은 `Meeting.speakers`가 한다.
   */
  speaker: string | null;
  speaker_turn: boolean;
};

export type Transcript = { segments: Segment[]; elapsed: number };

export type Backend =
  | { kind: "ollama"; model: string; endpoint?: string | null }
  | { kind: "anthropic"; api_key: string; model: string };

export const inspectEnvironment = () =>
  invoke<Environment>("inspect_environment");

export const downloadModel = (model: ModelSize) =>
  invoke<void>("download_model", { model });

export const listInputDevices = () =>
  invoke<InputDevice[]>("list_input_devices");

/// 마이크 권한 상태. 창을 띄우지 않는다.
export type MicPermission = "granted" | "not_determined" | "denied" | "not_required";

export const microphonePermission = () =>
  invoke<MicPermission>("microphone_permission");

export const openMicrophoneSettings = () =>
  invoke<void>("open_microphone_settings");

export const startRecording = (device: string | null, stamp: string) =>
  invoke<string>("start_recording", { device, stamp });

export const stopRecording = () => invoke<string>("stop_recording");

export const recordingStatus = () =>
  invoke<RecordingStatus>("recording_status");

/**
 * 그 회의에만 해당하는 배경 정보.
 *
 * 전역 설정이 아니라 회의마다 받는다 — 참석자도 안건도 매번 다르다. 전사에서는
 * 표기 고정에, 요약에서는 배경 지식에 쓰인다.
 */
export type Context = {
  attendees: string;
  topic: string;
  terms: string;
};

export const EMPTY_CONTEXT: Context = { attendees: "", topic: "", terms: "" };

export const isContextEmpty = (c: Context) =>
  !c.attendees.trim() && !c.topic.trim() && !c.terms.trim();

export const transcribeFile = (args: {
  path: string;
  model: ModelSize;
  language: string | null;
  context: Context | null;
}) => invoke<Transcript>("transcribe_file", args);

export const summarizeText = (
  backend: Backend,
  transcript: string,
  context: Context | null,
) => invoke<string>("summarize_text", { backend, transcript, context });

/**
 * 요약 모델에 넘길 녹취록 평문을 만든다.
 *
 * 화면에서 이어 붙이지 않고 Rust를 거치는 이유는 화자를 사람 이름으로 바꾸는
 * 규칙이 한 곳에만 있어야 하기 때문이다. 화면에 보이는 이름과 모델이 받는
 * 이름이 어긋나면 요약의 담당자가 왜 그렇게 나왔는지 설명할 수 없게 된다.
 */
export const transcriptText = (
  segments: Segment[],
  speakers: Record<string, string>,
) => invoke<string>("transcript_text", { segments, speakers });

// ---------------------------------------------------------------- 사내 명단

/**
 * 명단에 등록된 한 사람.
 *
 * 회의마다 참석자 이름을 다시 치지 않게 하려는 것이고, `manager`로 조직도를
 * 그린다. 목소리는 아직 여기 없다 — 화자는 사람이 직접 지정한다.
 */
export type Person = {
  /** 앱이 발급한다. 새로 만들 때는 빈 문자열로 보낸다. */
  id: string;
  name: string;
  team: string;
  title: string;
  /** 상위자 id. 최상위이거나 모르면 null. */
  manager: string | null;
};

export const EMPTY_PERSON: Person = {
  id: "",
  name: "",
  team: "",
  title: "",
  manager: null,
};

/** 요약 모델과 화면에 쓰는 표기. Rust의 `Person::display`와 같아야 한다. */
export const personLabel = (p: Person) =>
  p.title.trim() ? `${p.name}(${p.title.trim()})` : p.name;

export const listPeople = () => invoke<Person[]>("list_people");

/** 저장된 결과를 돌려준다 — 새로 만들면 발급된 id가 여기 들어 있다. */
export const savePerson = (person: Person) =>
  invoke<Person>("save_person", { person });

export const deletePerson = (id: string) =>
  invoke<void>("delete_person", { id });

/** 보관된 회의 한 건. */
export type Meeting = {
  id: string;
  title: string;
  recorded_at: string;
  audio_path: string | null;
  duration: number;
  segments: Segment[];
  /**
   * 화자 라벨("화자 1") → 명단의 사람 id.
   *
   * 이름이 아니라 id를 담는다. 명단에서 이름을 고쳐도 지난 회의의 연결이
   * 따라오게 하려는 것이다. 지정하지 않은 화자는 여기 없다.
   */
  speakers: Record<string, string>;
  summary: string | null;
  context: Context;
};

/** 목록에 필요한 만큼만. */
export type MeetingBrief = {
  id: string;
  title: string;
  recordedAt: string;
  duration: number;
  preview: string;
  hasSummary: boolean;
};

export const listMeetings = () => invoke<MeetingBrief[]>("list_meetings");

export const getMeeting = (id: string) =>
  invoke<Meeting | null>("get_meeting", { id });

export const saveMeeting = (meeting: Meeting) =>
  invoke<void>("save_meeting", { meeting });

export const renameMeeting = (id: string, title: string) =>
  invoke<void>("rename_meeting", { id, title });

export const deleteMeeting = (id: string) =>
  invoke<void>("delete_meeting", { id });

/**
 * 파일을 Finder(탐색기)에서 보여준다.
 *
 * 원본 음성(앱 데이터 폴더에 있어서 이 길이 없으면 들어볼 방법이 없다)과
 * 내보낸 회의록(저장 직후 보여줘야 바로 끌어다 공유한다)에 쓴다.
 */
export const revealFile = (path: string) =>
  invoke<void>("reveal_file", { path });

/** 공유용 텍스트를 파일로 쓴다. 위치는 저장 대화상자가 정한다. */
export const writeExport = (path: string, content: string) =>
  invoke<void>("write_export", { path, content });

/**
 * 공유용 문서 — 회의록(요약)과 전사 원문을 한 마크다운으로 묶는다.
 *
 * 화면에 보이는 것과 공유되는 것이 같아야 하므로, 화자 이름은 화면과 같은
 * 규칙(`speakerName`)을 넘겨받아 쓴다.
 */
export function exportMarkdown(
  m: Meeting,
  speakerName: (label: string) => string,
): string {
  const lines: string[] = [];
  lines.push(`# ${m.title.trim() || "제목 없는 회의"}`);
  lines.push("");
  lines.push(`- 일시: ${formatFullDate(m.recorded_at)}`);
  if (m.duration > 0) lines.push(`- 길이: ${formatClock(m.duration)}`);
  if (m.context.attendees.trim()) {
    lines.push(`- 참석자: ${m.context.attendees.trim()}`);
  }
  lines.push("");
  // 요약이 아직 없어도 원문은 공유할 수 있어야 한다 — 회의 직후 급하게
  // 스크립트만 돌려보는 경우가 있다.
  lines.push(m.summary?.trim() || "(요약이 아직 없습니다)");
  lines.push("");
  lines.push("---");
  lines.push("");
  lines.push("## 전사 원문");
  lines.push("");
  for (const s of m.segments) {
    const text = s.text.trim();
    if (!text) continue;
    const who = s.speaker ? `${speakerName(s.speaker)}: ` : "";
    lines.push(`[${formatClock(s.start)}] ${who}${text}`);
  }
  return lines.join("\n");
}

/** 바이트를 사람이 읽는 단위로. 모델 용량 안내에 쓴다. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

/** 초 → M:SS 또는 H:MM:SS */
export function formatClock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
}

/** 파일 이름에 쓸 로컬 시각 타임스탬프. */
export function localStamp(d = new Date()): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}` +
    `-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`
  );
}

/** 정렬 가능한 로컬 시각(ISO 형태, 시간대 표기 없음). */
export function localIso(d = new Date()): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}` +
    `T${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
  );
}

const WEEKDAYS = ["일", "월", "화", "수", "목", "금", "토"];

/** "2026년 7월 30일 목요일 오후 10:44" */
export function formatFullDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const hour = d.getHours();
  const ampm = hour < 12 ? "오전" : "오후";
  const h12 = hour % 12 === 0 ? 12 : hour % 12;
  return (
    `${d.getFullYear()}년 ${d.getMonth() + 1}월 ${d.getDate()}일 ` +
    `${WEEKDAYS[d.getDay()]}요일 ${ampm} ${h12}:${String(d.getMinutes()).padStart(2, "0")}`
  );
}

/** 목록용 짧은 표기. 오늘이면 시각만, 올해면 월·일, 그 외엔 연도까지. */
export function formatShortDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) {
    return `${String(d.getHours()).padStart(2, "0")}:${String(
      d.getMinutes(),
    ).padStart(2, "0")}`;
  }
  if (d.getFullYear() === now.getFullYear()) {
    return `${d.getMonth() + 1}월 ${d.getDate()}일`;
  }
  return `${d.getFullYear()}.${d.getMonth() + 1}.${d.getDate()}`;
}

/** 목록을 오늘 / 어제 / 이번 주 / 지난 기록으로 묶는다. */
export function bucketOf(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "지난 기록";
  const now = new Date();
  const startOfToday = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate(),
  ).getTime();
  const day = 86_400_000;
  const t = d.getTime();
  if (t >= startOfToday) return "오늘";
  if (t >= startOfToday - day) return "어제";
  if (t >= startOfToday - day * 7) return "이번 주";
  return "지난 기록";
}
