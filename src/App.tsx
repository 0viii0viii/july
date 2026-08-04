import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import {
  EMPTY_CONTEXT,
  bucketOf,
  deleteMeeting,
  downloadModel,
  formatClock,
  formatFullDate,
  formatShortDate,
  getMeeting,
  inspectEnvironment,
  listInputDevices,
  listMeetings,
  localIso,
  localStamp,
  pullSummarizer,
  recordingStatus,
  renameMeeting,
  saveMeeting,
  startRecording,
  stopRecording,
  summarizeText,
  transcribeFile,
  type Backend,
  type Context,
  type DownloadProgress,
  type Environment,
  type InputDevice,
  type Meeting,
  type MeetingBrief,
  type ModelSize,
  type PullProgress,
  type RecordingStatus,
} from "./api";
import { Brief } from "./Brief";
import { Config, DEFAULT_SETTINGS, type Settings } from "./Config";
import { Gate } from "./Gate";
import { Minutes } from "./Minutes";
import { Updater } from "./Updater";
import "./App.css";

const STORE_KEY = "july.settings";
/** 직전 회의의 참석자·용어. 팀은 대개 반복되므로 다음 회의에 미리 채워준다. */
const LAST_CONTEXT_KEY = "july.lastContext";
const METER_BARS = 32;

type Phase =
  | "booting"
  | "idle"
  /** 녹음 중 */
  | "recording"
  /** 음성은 확보했고, 회의 정보를 받는 중 */
  | "briefing"
  | "transcribing"
  | "summarizing";

/** 처리를 기다리는 음성. */
type Pending = { path: string; seconds: number | null };

function loadLastContext(): Context {
  try {
    const raw = localStorage.getItem(LAST_CONTEXT_KEY);
    return raw ? { ...EMPTY_CONTEXT, ...JSON.parse(raw) } : EMPTY_CONTEXT;
  } catch {
    return EMPTY_CONTEXT;
  }
}

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    return raw ? { ...DEFAULT_SETTINGS, ...JSON.parse(raw) } : DEFAULT_SETTINGS;
  } catch {
    return DEFAULT_SETTINGS;
  }
}

/**
 * 피크 값을 눈에 보이는 높이로 편다.
 *
 * 사람 말소리의 피크는 대개 0.05~0.3에 머문다. 선형으로 그리면 미터가 거의
 * 안 움직여서 죽은 것처럼 보인다.
 */
function meterScale(peak: number): number {
  if (peak <= 0) return 0;
  const db = 20 * Math.log10(Math.max(peak, 1e-5));
  return Math.min(1, Math.max(0, (db + 60) / 60));
}

export default function App() {
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [showConfig, setShowConfig] = useState(false);

  const [env, setEnv] = useState<Environment | null>(null);
  const [devices, setDevices] = useState<InputDevice[]>([]);
  const [download, setDownload] = useState<DownloadProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [pulling, setPulling] = useState<string | null>(null);
  const [pullProgress, setPullProgress] = useState<PullProgress | null>(null);

  const [phase, setPhase] = useState<Phase>("booting");
  const [rec, setRec] = useState<RecordingStatus | null>(null);
  const [dragging, setDragging] = useState(false);
  const [pending, setPending] = useState<Pending | null>(null);
  const [lastContext, setLastContext] = useState<Context>(loadLastContext);

  const [meetings, setMeetings] = useState<MeetingBrief[]>([]);
  const [current, setCurrent] = useState<Meeting | null>(null);
  const [showRaw, setShowRaw] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    localStorage.setItem(STORE_KEY, JSON.stringify(settings));
  }, [settings]);

  const refreshEnv = useCallback(async () => {
    const next = await inspectEnvironment();
    setEnv(next);
    setSettings((s) =>
      !s.ollamaModel && next.ollamaModels.length
        ? { ...s, ollamaModel: next.ollamaModels[0] }
        : s,
    );
    return next;
  }, []);

  const refreshList = useCallback(async () => {
    setMeetings(await listMeetings().catch(() => []));
  }, []);

  useEffect(() => {
    (async () => {
      try {
        await refreshEnv();
        await refreshList();
        setDevices(await listInputDevices().catch(() => []));
      } catch (e) {
        // 여기서 삼키면 아무것도 안 그려진 빈 창만 남는다. 반드시 드러낸다.
        setError(`앱 상태를 확인할 수 없습니다: ${e}`);
      } finally {
        setPhase("idle");
      }
    })();
  }, [refreshEnv, refreshList]);

  useEffect(() => {
    const stops = [
      listen<DownloadProgress>("model-download", (e) => setDownload(e.payload)),
      listen<PullProgress>("ollama-pull", (e) => setPullProgress(e.payload)),
    ];
    return () => {
      stops.forEach((s) => s.then((f) => f()));
    };
  }, []);

  // 녹음 중에만 상태를 폴링한다.
  useEffect(() => {
    if (phase !== "recording") return;
    let alive = true;
    const tick = async () => {
      try {
        const s = await recordingStatus();
        if (alive) setRec(s);
      } catch {
        /* 정지 직후의 경합은 무시해도 된다 */
      }
    };
    const id = setInterval(tick, 90);
    tick();
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [phase]);

  const backend = useMemo<Backend>(
    () =>
      settings.summarizer === "ollama"
        ? { kind: "ollama", model: settings.ollamaModel, endpoint: null }
        : {
            kind: "anthropic",
            api_key: settings.apiKey,
            model: settings.anthropicModel,
          },
    [settings],
  );

  /** 음성 파일 하나를 회의록으로 만들어 보관한다. */
  const process = useCallback(
    async (audioPath: string, context: Context) => {
      setError(null);
      setShowRaw(false);
      setPending(null);

      // 다음 회의에 미리 채워줄 값으로 기억해 둔다.
      setLastContext(context);
      localStorage.setItem(LAST_CONTEXT_KEY, JSON.stringify(context));

      const id = crypto.randomUUID();
      const recordedAt = localIso();

      let meeting: Meeting;
      try {
        setPhase("transcribing");
        const result = await transcribeFile({
          path: audioPath,
          model: "large_v3_turbo",
          language: "ko",
          context,
        });

        if (!result.segments.some((s) => s.text.trim())) {
          setError(
            "인식된 음성이 없습니다. 녹음이 제대로 되었는지 확인해 주세요.",
          );
          setPhase("idle");
          return;
        }

        meeting = {
          id,
          title: context.topic.trim(),
          recorded_at: recordedAt,
          audio_path: audioPath,
          duration: result.segments[result.segments.length - 1]?.end ?? 0,
          segments: result.segments,
          summary: null,
          context,
        };
        setCurrent(meeting);
      } catch (e) {
        setError(String(e));
        setPhase("idle");
        return;
      }

      try {
        setPhase("summarizing");
        meeting.summary = await summarizeText(
          backend,
          meeting.segments.map((s) => s.text).join(" "),
          context,
        );
      } catch (e) {
        // 요약이 실패해도 전사는 살린다. 다시 요약할 수 있게 그대로 보관한다.
        setError(`요약에 실패했습니다: ${e}`);
      }

      try {
        await saveMeeting(meeting);
        await refreshList();
      } catch (e) {
        setError(`회의 기록을 저장하지 못했습니다: ${e}`);
      }

      setCurrent({ ...meeting });
      setPhase("idle");
    },
    [backend, refreshList],
  );

  /**
   * 음성을 확보했다 — 바로 돌리지 않고 회의 정보부터 받는다.
   *
   * 준비가 안 끝났으면(모델 미설치) 화면을 넘기지 않고 파일만 들고 있는다.
   * 준비 화면이 다른 모든 화면보다 우선해서 그려지기 때문에, 여기서 phase를
   * 바꿔봐야 아무 변화도 안 보인다 — 드롭이 조용히 삼켜진 것처럼 느껴진다.
   * 대신 무엇을 받아뒀는지 알려주고, 준비가 끝나면 그 파일로 이어서 간다.
   */
  const intake = useCallback((path: string, seconds: number | null) => {
    setError(null);
    setCurrent(null);
    setPending({ path, seconds });
    // 반드시 둘 중 하나로 확정한다. 조건부로만 바꾸면 녹음을 멈춘 뒤에도
    // phase가 "recording"에 남아 녹음 화면이 그대로 떠 있게 된다.
    setPhase(needsSetupRef.current ? "idle" : "briefing");
  }, []);

  const pickFile = useCallback(async () => {
    try {
      const picked = await openFileDialog({
        multiple: false,
        directory: false,
        filters: [{ name: "음성 파일", extensions: ["wav"] }],
      });
      if (typeof picked === "string") intake(picked, null);
    } catch (e) {
      setError(`파일을 열 수 없습니다: ${e}`);
    }
  }, [intake]);

  // 네이티브 드래그앤드롭 — 브라우저 이벤트와 달리 실제 파일 경로를 준다.
  // 등록이 실패해도 앱 전체를 내리지는 않는다. 드롭이 안 될 뿐 녹음은 된다.
  useEffect(() => {
    let stop: Promise<() => void> | null = null;
    try {
      stop = getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "over") {
          setDragging(true);
        } else if (event.payload.type === "drop") {
          setDragging(false);
          const file = event.payload.paths[0];
          if (file) intake(file, null);
        } else {
          setDragging(false);
        }
      });
    } catch (e) {
      console.error("드래그앤드롭을 등록하지 못했습니다:", e);
    }
    return () => {
      stop?.then((f) => f()).catch(() => {});
    };
  }, [intake]);

  const toggleRecord = useCallback(async () => {
    if (phase === "recording") {
      const seconds = rec?.seconds ?? null;
      try {
        const path = await stopRecording();
        setRec(null);
        intake(path, seconds);
      } catch (e) {
        setError(String(e));
        setPhase("idle");
      }
      return;
    }

    setError(null);
    try {
      await startRecording(settings.device, localStamp());
      setCurrent(null);
      setPending(null);
      setPhase("recording");
    } catch (e) {
      setError(String(e));
    }
  }, [phase, rec?.seconds, settings.device, intake]);

  const openMeeting = useCallback(async (id: string) => {
    setShowRaw(false);
    setError(null);
    try {
      setCurrent(await getMeeting(id));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const onRename = useCallback(
    async (title: string) => {
      if (!current) return;
      setCurrent({ ...current, title });
      try {
        await renameMeeting(current.id, title);
        await refreshList();
      } catch (e) {
        setError(String(e));
      }
    },
    [current, refreshList],
  );

  const onDelete = useCallback(async () => {
    if (!current) return;
    try {
      await deleteMeeting(current.id);
      setCurrent(null);
      await refreshList();
    } catch (e) {
      setError(String(e));
    }
  }, [current, refreshList]);

  const onDownloadModel = useCallback(
    async (model: ModelSize) => {
      setBusy(true);
      setError(null);
      try {
        await downloadModel(model);
        await refreshEnv();
      } catch (e) {
        setError(String(e));
      } finally {
        setDownload(null);
        setBusy(false);
      }
    },
    [refreshEnv],
  );

  const onPullSummarizer = useCallback(
    async (id: string) => {
      setPulling(id);
      setPullProgress(null);
      setError(null);
      try {
        await pullSummarizer(id);
        const next = await refreshEnv();
        // 방금 받은 모델을 바로 쓰도록 골라준다. 한 번 더 고르게 할 이유가 없다.
        if (next.ollamaModels.includes(id)) {
          setSettings((s) => ({ ...s, ollamaModel: id, summarizer: "ollama" }));
        }
      } catch (e) {
        setError(String(e));
      } finally {
        setPulling(null);
        setPullProgress(null);
      }
    },
    [refreshEnv],
  );

  const working = phase === "transcribing" || phase === "summarizing";
  const recording = phase === "recording";
  // 전사 모델이 있어야 뭐라도 할 수 있다.
  const ready = env?.canTranscribe ?? false;
  // 요약까지 가능한 상태인지. API 키를 쓰기로 했다면 로컬 모델은 없어도 된다.
  const canSummarize =
    settings.summarizer === "anthropic"
      ? settings.apiKey.trim().length > 0
      : !!settings.ollamaModel && (env?.ollamaModels.includes(settings.ollamaModel) ?? false);
  // env를 아직 못 읽었을 때도 "준비 안 됨"으로 본다. 그래야 부팅 중에 들어온
  // 드롭이 화면을 앞질러 가지 않고 얌전히 대기한다.
  const needsSetup = !env || !ready || !canSummarize;

  // intake()는 드래그앤드롭 리스너 안에서 불리므로 등록 시점의 값에 갇힌다.
  // 최신 값을 ref로 따로 들고 있어야 한다.
  const needsSetupRef = useRef(needsSetup);
  useEffect(() => {
    needsSetupRef.current = needsSetup;
    // 준비가 끝났는데 받아둔 파일이 있으면 그때 이어서 간다. 사용자가 파일을
    // 다시 끌어다 놓게 만들 이유가 없다.
    if (!needsSetup && pending && phase === "idle") setPhase("briefing");
  }, [needsSetup, pending, phase]);
  // Mac mini·Studio처럼 내장 마이크가 없는 기기가 있다. 눌러보고 알게 하는
  // 대신 미리 막고 이유를 알려준다.
  const noMic = devices.length === 0;

  // 목록을 오늘/어제/이번 주/지난 기록으로 묶는다.
  const grouped = useMemo(() => {
    const order = ["오늘", "어제", "이번 주", "지난 기록"];
    const map = new Map<string, MeetingBrief[]>();
    for (const m of meetings) {
      const key = bucketOf(m.recordedAt);
      const list = map.get(key);
      if (list) list.push(m);
      else map.set(key, [m]);
    }
    return order
      .filter((k) => map.has(k))
      .map((k) => ({ label: k, items: map.get(k)! }));
  }, [meetings]);

  const level = recording ? meterScale(rec?.level ?? 0) : 0;
  const litBars = Math.round(level * METER_BARS);

  return (
    <div className="app">
      {dragging && <div className="drop-veil">음성 파일을 놓으세요</div>}

      <aside className="rail">
        <div className="rail-top">
          <h1 className="brand">July</h1>
          <button
            className={`new-btn${recording ? " live" : ""}`}
            onClick={toggleRecord}
            disabled={working || (!recording && (noMic || !ready))}
          >
            {recording ? (
              <>
                <span className="dot" />
                녹음 정지 · {formatClock(rec?.seconds ?? 0)}
              </>
            ) : (
              <>● 새 회의 녹음</>
            )}
          </button>

          {/* 드래그앤드롭만 있으면 그런 기능이 있는지도 모른다 */}
          {/* 준비 전에도 고를 수 있게 둔다. 끌어다 놓는 쪽은 받아주면서
              버튼만 막아두면 일관성이 없다. 고른 파일은 대기시킨다. */}
          <button
            className="open-btn"
            onClick={pickFile}
            disabled={working || recording}
          >
            음성 파일 열기
          </button>
        </div>

        <div className="rail-list">
          {grouped.length === 0 ? (
            <p className="rail-empty">
              아직 기록된 회의가 없습니다.
              <br />
              녹음하거나 음성 파일을 끌어다 놓으세요.
            </p>
          ) : (
            grouped.map((group) => (
              <div className="rail-group" key={group.label}>
                <div className="rail-group-label">{group.label}</div>
                {group.items.map((m) => (
                  <button
                    key={m.id}
                    className={`entry${current?.id === m.id ? " on" : ""}`}
                    onClick={() => openMeeting(m.id)}
                  >
                    <div className="entry-title">
                      {m.title || m.preview || "제목 없는 회의"}
                    </div>
                    <div className="entry-meta">
                      <span>{formatShortDate(m.recordedAt)}</span>
                      <span>{formatClock(m.duration)}</span>
                      {!m.hasSummary && <span>요약 없음</span>}
                    </div>
                  </button>
                ))}
              </div>
            ))
          )}
        </div>

        <div className="rail-foot">
          <span className="local-mark">
            <i />이 기기에서 처리
          </span>
          <button className="icon-btn" onClick={() => setShowConfig(true)}>
            설정
          </button>
        </div>
      </aside>

      <main className="stage">
        <div className="stage-drag" />
        <Updater />

        {error && <div className="alarm">{error}</div>}

        {/*
          화면 우선순위. 진행 중인 작업이 준비 화면보다 앞선다.

          예전엔 준비 화면이 맨 앞이었는데, 그러면 준비가 덜 끝난 상태에서
          시작한 녹음이나 드롭이 화면에 전혀 안 나타난다 — 눌러도 아무 일도
          안 일어나는 것처럼 보인다. 지금 하고 있는 일을 항상 먼저 보여주고,
          준비 화면은 아무것도 안 하고 있을 때만 띄운다.
        */}
        {phase === "booting" ? null : recording ? (
          <div className="progress">
            <div className="progress-card">
              <div className="live-bar">
                <span className="live-time">
                  {formatClock(rec?.seconds ?? 0)}
                </span>
                <div className="live-meter" aria-hidden="true">
                  {Array.from({ length: METER_BARS }, (_, i) => (
                    <i
                      key={i}
                      className={i < litBars ? "on" : ""}
                      style={{ height: i < litBars ? "100%" : "3px" }}
                    />
                  ))}
                </div>
              </div>
              <p
                style={{
                  margin: "16px 0 0",
                  fontSize: 13,
                  color: rec?.silent ? "var(--seal)" : "var(--ink-3)",
                }}
              >
                {rec?.silent
                  ? "소리가 잡히지 않습니다 — 마이크 권한과 입력 장치를 확인해 주세요"
                  : "녹음 중입니다. 정지하면 바로 회의록을 만듭니다."}
              </p>
            </div>
          </div>
        ) : phase === "briefing" && pending ? (
          <Brief
            source={pending.path}
            seconds={pending.seconds}
            initial={lastContext}
            onStart={(context) => void process(pending.path, context)}
            onCancel={() => {
              setPending(null);
              setPhase("idle");
            }}
          />
        ) : working ? (
          <div className="progress">
            <div className="progress-card">
              <div className="wave" aria-hidden="true">
                {Array.from({ length: 26 }, (_, i) => (
                  <i key={i} style={{ animationDelay: `${i * 0.045}s` }} />
                ))}
              </div>
              <div
                className={`stage-row ${
                  phase === "transcribing" ? "now" : "was"
                }`}
              >
                <span className="stage-mark">✓</span>
                음성을 글로 옮기는 중
                {current && phase !== "transcribing" && (
                  <span className="stage-time">
                    {current.segments.length}문장
                  </span>
                )}
              </div>
              <div className={`stage-row ${phase === "summarizing" ? "now" : ""}`}>
                <span className="stage-mark">✓</span>
                회의록으로 정리하는 중
              </div>
            </div>
          </div>
        ) : needsSetup && env ? (
          <Gate
            env={env}
            progress={download}
            busy={busy}
            onDownload={onDownloadModel}
            selectedSummarizer={settings.ollamaModel}
            pending={pending?.path ?? null}
            pulling={pulling}
            pullProgress={pullProgress}
            onSelectSummarizer={(id) =>
              setSettings((s) => ({
                ...s,
                ollamaModel: id,
                summarizer: "ollama",
              }))
            }
            onPullSummarizer={onPullSummarizer}
          />
        ) : current ? (
          <article className="doc">
            <header className="doc-head">
              <div className="doc-date">
                {formatFullDate(current.recorded_at)}
              </div>
              <input
                className="doc-title"
                value={current.title}
                placeholder="제목 없는 회의"
                onChange={(e) => onRename(e.target.value)}
              />
              <div className="doc-facts">
                <span>{formatClock(current.duration)}</span>
                <span>{current.segments.length}문장</span>
                <button
                  className="icon-btn"
                  style={{ marginLeft: "auto", padding: "2px 8px" }}
                  onClick={onDelete}
                >
                  삭제
                </button>
              </div>
            </header>

            {current.summary ? (
              <Minutes markdown={current.summary} />
            ) : (
              <p style={{ color: "var(--ink-3)", fontSize: 13 }}>
                요약이 없습니다. 전사 원문은 아래에서 볼 수 있습니다.
              </p>
            )}

            <section className="raw">
              <button
                className={`raw-toggle${showRaw ? " open" : ""}`}
                onClick={() => setShowRaw((v) => !v)}
              >
                <svg
                  width="9"
                  height="9"
                  viewBox="0 0 10 10"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.6"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M3 1l5 4-5 4" />
                </svg>
                전사 원문 {current.segments.length}문장
              </button>
              {showRaw && (
                <ol className="reel">
                  {current.segments.map((s, i) => (
                    <li key={i}>
                      <time>{formatClock(s.start)}</time>
                      <span>{s.text}</span>
                    </li>
                  ))}
                </ol>
              )}
            </section>
          </article>
        ) : (
          <div className="blank">
            <div className="blank-art" />
            <h2>회의록을 만들어 보세요</h2>
            <p>
              {noMic
                ? "마이크가 연결돼 있지 않습니다. 음성 파일을 열거나 창 위로 끌어다 놓으면 회의록이 만들어집니다."
                : "녹음을 시작하거나, 이미 있는 음성 파일을 열어 보세요. 창 위로 끌어다 놓아도 됩니다."}
            </p>
            <button className="btn" onClick={pickFile}>
              음성 파일 열기
            </button>
          </div>
        )}
      </main>

      {showConfig && (
        <Config
          settings={settings}
          env={env}
          devices={devices}
          onChange={setSettings}
          onClose={() => setShowConfig(false)}
        />
      )}
    </div>
  );
}
