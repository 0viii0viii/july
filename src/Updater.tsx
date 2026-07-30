import { useEffect, useState } from "react";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

type State =
  | { kind: "idle" }
  | { kind: "found"; update: Update }
  | { kind: "installing" }
  | { kind: "failed"; message: string };

/**
 * 새 버전 알림.
 *
 * 앱을 켤 때 한 번만 확인한다. 회의 중에 업데이트 배너가 튀어나오면 방해가
 * 되므로 주기적으로 다시 묻지 않는다.
 *
 * 확인 자체가 실패하는 건(오프라인, 릴리즈 없음) 정상 상황이라 조용히 넘긴다.
 * 사용자가 설치를 누른 뒤에 실패했을 때만 알린다.
 */
export function Updater() {
  const [state, setState] = useState<State>({ kind: "idle" });

  useEffect(() => {
    let alive = true;
    check()
      .then((update) => {
        if (alive && update) setState({ kind: "found", update });
      })
      .catch(() => {
        /* 오프라인이거나 아직 릴리즈가 없다 — 알릴 일이 아니다 */
      });
    return () => {
      alive = false;
    };
  }, []);

  if (state.kind === "idle") return null;

  if (state.kind === "failed") {
    return (
      <div className="update-bar update-bar--bad">
        <span>업데이트에 실패했습니다: {state.message}</span>
        <button className="icon-btn" onClick={() => setState({ kind: "idle" })}>
          닫기
        </button>
      </div>
    );
  }

  if (state.kind === "installing") {
    return (
      <div className="update-bar">
        <span>내려받는 중… 끝나면 자동으로 다시 시작합니다.</span>
      </div>
    );
  }

  const { update } = state;
  return (
    <div className="update-bar">
      <span>
        새 버전 <b>{update.version}</b>이 있습니다.
      </span>
      <div className="update-actions">
        <button className="icon-btn" onClick={() => setState({ kind: "idle" })}>
          나중에
        </button>
        <button
          className="btn btn--seal"
          onClick={async () => {
            setState({ kind: "installing" });
            try {
              await update.downloadAndInstall();
              await relaunch();
            } catch (e) {
              setState({ kind: "failed", message: String(e) });
            }
          }}
        >
          지금 설치
        </button>
      </div>
    </div>
  );
}
