import {
  formatBytes,
  type DownloadProgress,
  type Environment,
  type ModelSize,
  type PullProgress,
} from "./api";
import { Summarizers } from "./Summarizers";

/**
 * 첫 실행 준비 화면.
 *
 * 이 앱을 쓰는 사람이 터미널을 열 일은 없어야 한다. 무엇이 필요한지, 얼마나
 * 받아야 하는지, 지금 어디까지 왔는지를 전부 여기서 보여주고 여기서 받는다.
 */
export function Gate({
  env,
  progress,
  busy,
  onDownload,
  selectedSummarizer,
  pending,
  pulling,
  pullProgress,
  onSelectSummarizer,
  onPullSummarizer,
}: {
  env: Environment;
  progress: DownloadProgress | null;
  busy: boolean;
  onDownload: (model: ModelSize) => void;
  selectedSummarizer: string;
  /** 준비가 끝나길 기다리는 음성 파일 경로. 준비 중에 끌어다 놓은 것. */
  pending: string | null;
  pulling: string | null;
  pullProgress: PullProgress | null;
  onSelectSummarizer: (id: string) => void;
  onPullSummarizer: (id: string) => void;
}) {
  const primary = env.models.find((m) => m.id === "large_v3_turbo");
  const downloading = progress !== null;
  const pct =
    progress && progress.total
      ? Math.min(100, (progress.received / progress.total) * 100)
      : null;

  const hasSummarizer = env.summarizers.some((m) => m.installed);
  const memGb = Math.round(env.hardware.totalMemoryBytes / 1e9);

  return (
    <div className="gate">
      <h2>시작하기 전에</h2>
      <p>
        음성 인식도 요약도 이 기기 안에서 돌아갑니다. 그래서 모델을 한 번
        내려받아야 하고, 그다음부터는 인터넷 없이 회의록이 만들어집니다.
      </p>

      {/* 준비 중에 파일을 끌어다 놓은 경우. 아무 반응이 없으면 드롭이 씹힌
          줄 알게 되므로 무엇을 들고 있는지 반드시 알려준다. */}
      {pending && (
        <div className="gate-pending">
          <b>{pending.split("/").pop()}</b>
          <span>준비가 끝나면 이 파일로 바로 회의록을 만듭니다.</span>
        </div>
      )}

      {/* 1. 음성 인식 모델 */}
      <div className="need">
        <span className={`need-mark${primary?.installed ? " on" : ""}`}>
          {primary?.installed ? "✓" : "1"}
        </span>
        <div className="need-body">
          <div className="need-title">음성 인식 모델</div>
          <div className="need-note">
            {primary?.installed
              ? `설치됨 · ${formatBytes(primary.bytes_on_disk)}`
              : downloading
                ? `${formatBytes(progress.received)}${
                    progress.total ? ` / ${formatBytes(progress.total)}` : ""
                  }`
                : `약 ${formatBytes(primary?.approx_bytes ?? 0)} · 최초 1회만`}
          </div>
          {downloading && (
            <div className="bar">
              <i style={{ width: pct !== null ? `${pct}%` : "35%" }} />
            </div>
          )}
        </div>
        {!primary?.installed && !downloading && (
          <button
            className="btn btn--seal"
            disabled={busy}
            onClick={() => onDownload("large_v3_turbo")}
          >
            내려받기
          </button>
        )}
      </div>

      {/* 2. 요약 모델 */}
      <div className="need">
        <span className={`need-mark${hasSummarizer ? " on" : ""}`}>
          {hasSummarizer ? "✓" : "2"}
        </span>
        <div className="need-body">
          <div className="need-title">요약 모델</div>
          <div className="need-note">
            {env.hardware.cpu} · 메모리 {memGb}GB
          </div>
        </div>
      </div>

      {env.ollamaRunning ? (
        <>
          <p className="gate-hint">
            기기 사양에 맞는 모델을 <b>추천</b>으로 표시했습니다. 크면 요약이
            조금 나아지지만 그만큼 느려집니다.
          </p>
          <Summarizers
            models={env.summarizers}
            selected={selectedSummarizer}
            pulling={pulling}
            progress={pullProgress}
            onSelect={onSelectSummarizer}
            onPull={onPullSummarizer}
          />
        </>
      ) : (
        <div className="gate-missing">
          <b>Ollama가 필요합니다.</b>
          <p>
            요약을 이 기기에서 돌리려면 Ollama를 설치하고 실행해 주세요. 설치
            후 이 화면으로 돌아오면 모델 목록이 나타납니다.
          </p>
          <p className="gate-alt">
            설치하지 않고 쓰려면 설정에서 Anthropic API 키를 넣어도 됩니다. 이
            경우 녹취 내용이 Anthropic으로 전송됩니다.
          </p>
        </div>
      )}
    </div>
  );
}
