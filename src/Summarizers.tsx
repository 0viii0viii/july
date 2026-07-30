import { formatBytes, type PullProgress, type Summarizer } from "./api";

const SPEED_LABEL = {
  fast: "빠름",
  balanced: "보통",
  slow: "느림",
} as const;

const FIT_NOTE = {
  comfortable: null,
  tight: "메모리가 빠듯합니다",
  too_big: "이 기기에는 큽니다",
} as const;

/**
 * 요약 모델 고르기.
 *
 * 앱을 막 설치한 사람에게는 모델이 하나도 없다. 목록·용량·기기 적합성을 다
 * 보여주고 앱 안에서 받게 한다 — 터미널을 열게 하면 거기서 이탈한다.
 */
export function Summarizers({
  models,
  selected,
  pulling,
  progress,
  onSelect,
  onPull,
}: {
  models: Summarizer[];
  selected: string;
  /** 지금 내려받는 중인 모델 id. */
  pulling: string | null;
  progress: PullProgress | null;
  onSelect: (id: string) => void;
  onPull: (id: string) => void;
}) {
  return (
    <ul className="picks">
      {models.map((m) => {
        const busy = pulling === m.id;
        const pct =
          busy && progress && progress.total > 0
            ? Math.min(100, (progress.completed / progress.total) * 100)
            : null;
        const fitNote = FIT_NOTE[m.fit];

        return (
          <li
            key={m.id}
            className={[
              "pick",
              m.installed && selected === m.id ? "on" : "",
              m.fit === "too_big" ? "over" : "",
            ]
              .filter(Boolean)
              .join(" ")}
            onClick={() => m.installed && onSelect(m.id)}
          >
            <div className="pick-main">
              <div className="pick-title">
                <span className="pick-name">{m.name}</span>
                {m.recommended && <span className="chip chip--rec">추천</span>}
                {m.verified && <span className="chip">검증됨</span>}
                {m.installed && <span className="chip chip--on">설치됨</span>}
              </div>
              <div className="pick-note">{m.note}</div>
              <div className="pick-meta">
                <span>{m.maker}</span>
                <span>{formatBytes(m.bytes)}</span>
                <span>{SPEED_LABEL[m.speed]}</span>
                {fitNote && <span className="pick-warn">{fitNote}</span>}
              </div>
              {busy && (
                <>
                  <div className="bar">
                    <i style={{ width: pct !== null ? `${pct}%` : "30%" }} />
                  </div>
                  <div className="pick-status">
                    {progress?.status ?? "준비 중"}
                    {pct !== null && ` · ${pct.toFixed(0)}%`}
                  </div>
                </>
              )}
            </div>

            {!m.installed && !busy && (
              <button
                className="btn"
                onClick={(e) => {
                  e.stopPropagation();
                  onPull(m.id);
                }}
                disabled={pulling !== null}
              >
                받기
              </button>
            )}
          </li>
        );
      })}
    </ul>
  );
}
