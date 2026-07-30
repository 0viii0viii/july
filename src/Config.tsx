import type { Environment, InputDevice } from "./api";

export type Settings = {
  summarizer: "ollama" | "anthropic";
  ollamaModel: string;
  anthropicModel: string;
  /** 이 기기에만 저장된다. Anthropic 외 어디로도 나가지 않는다. */
  apiKey: string;
  /** null이면 시스템 기본 입력 장치. */
  device: string | null;
};

// 참석자·주제·용어는 여기 없다. 회의마다 다르므로 처리 직전에 따로 받는다.
export const DEFAULT_SETTINGS: Settings = {
  summarizer: "ollama",
  ollamaModel: "",
  anthropicModel: "claude-opus-5",
  apiKey: "",
  device: null,
};

const ANTHROPIC_MODELS = [
  { id: "claude-opus-5", label: "Opus 5 — 최고 품질" },
  { id: "claude-sonnet-5", label: "Sonnet 5 — 균형" },
  { id: "claude-haiku-4-5", label: "Haiku 4.5 — 가장 저렴" },
];

export function Config({
  settings,
  env,
  devices,
  onChange,
  onClose,
}: {
  settings: Settings;
  env: Environment | null;
  devices: InputDevice[];
  onChange: (next: Settings) => void;
  onClose: () => void;
}) {
  const set = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    onChange({ ...settings, [key]: value });

  return (
    <div
      className="sheet-backdrop"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="sheet" role="dialog" aria-label="설정">
        <div className="sheet-head">
          <h2>설정</h2>
          <button className="icon-btn" onClick={onClose} aria-label="닫기">
            닫기
          </button>
        </div>

        <div className="knob">
          <label htmlFor="cfg-device">마이크</label>
          {devices.length > 0 ? (
            <select
              id="cfg-device"
              value={settings.device ?? ""}
              onChange={(e) => set("device", e.target.value || null)}
            >
              <option value="">시스템 기본</option>
              {devices.map((d) => (
                <option key={d.name} value={d.name}>
                  {d.name}
                  {d.is_default ? " (기본)" : ""}
                </option>
              ))}
            </select>
          ) : (
            <span className="aside warn">
              연결된 마이크가 없습니다. Mac mini·Mac Studio에는 내장 마이크가
              없어 USB 마이크나 헤드셋이 필요합니다. 파일을 끌어다 놓는 방식은
              그대로 쓸 수 있습니다.
            </span>
          )}
        </div>

        <div className="knob">
          <label>요약 방식</label>
          <div className="forks">
            <button
              className={`fork${settings.summarizer === "ollama" ? " on" : ""}`}
              onClick={() => set("summarizer", "ollama")}
            >
              <div className="fork-name">이 기기에서</div>
              <div className="fork-desc">무료 · 오프라인 · 밖으로 안 나감</div>
            </button>
            <button
              className={`fork${
                settings.summarizer === "anthropic" ? " on" : ""
              }`}
              onClick={() => set("summarizer", "anthropic")}
            >
              <div className="fork-name">Anthropic API</div>
              <div className="fork-desc">더 정확함 · 본인 키로 직접 결제</div>
            </button>
          </div>
        </div>

        {settings.summarizer === "ollama" ? (
          <div className="knob">
            <label htmlFor="cfg-ollama">로컬 모델</label>
            {env?.ollamaRunning && env.ollamaModels.length > 0 ? (
              <select
                id="cfg-ollama"
                value={settings.ollamaModel}
                onChange={(e) => set("ollamaModel", e.target.value)}
              >
                {!settings.ollamaModel && <option value="">선택하세요</option>}
                {env.ollamaModels.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            ) : (
              <span className="aside warn">
                Ollama가 실행 중이 아닙니다. 설치하고 모델을 하나 받아두면 요약도
                전부 오프라인으로 돌아갑니다.
              </span>
            )}
          </div>
        ) : (
          <>
            <div className="knob">
              <label htmlFor="cfg-model">모델</label>
              <select
                id="cfg-model"
                value={settings.anthropicModel}
                onChange={(e) => set("anthropicModel", e.target.value)}
              >
                {ANTHROPIC_MODELS.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.label}
                  </option>
                ))}
              </select>
            </div>
            <div className="knob">
              <label htmlFor="cfg-key">API 키</label>
              <input
                id="cfg-key"
                type="password"
                placeholder="sk-ant-..."
                value={settings.apiKey}
                onChange={(e) => set("apiKey", e.target.value)}
              />
              <span className="aside">
                이 기기에만 저장되며 Anthropic 외에는 전송되지 않습니다. 1시간
                회의 요약이 대략 60원에서 300원 수준입니다.
              </span>
            </div>
          </>
        )}

        <div className="knob" style={{ marginBottom: 0 }}>
          <label>저장 위치</label>
          <span className="aside" style={{ wordBreak: "break-all" }}>
            {env?.modelsPath ?? "-"}
          </span>
        </div>
      </div>
    </div>
  );
}
