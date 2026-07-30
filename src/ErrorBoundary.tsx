import { Component, type ErrorInfo, type ReactNode } from "react";

/**
 * 렌더링 중 예외가 나도 최소한 무슨 일이 났는지는 보이게 한다.
 *
 * 이게 없으면 어떤 오류든 빈 창으로 끝난다 — 사용자는 앱이 고장 났다는 것만
 * 알 뿐 이유도, 알릴 방법도 없다.
 */
export class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("렌더링 오류:", error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <main className="shell" style={{ paddingTop: 48 }}>
        <header className="masthead">
          <h1 className="wordmark">
            회의록 <span className="rev">ERROR</span>
          </h1>
        </header>

        <section className="panel setup">
          <div className="setup-head">
            <span className="eyebrow">Unexpected</span>
            <h2>화면을 그리는 중 문제가 생겼습니다</h2>
            <p>
              녹음 파일은 그대로 남아 있습니다. 앱을 다시 시작해도 같은 문제가
              반복되면 아래 내용을 그대로 알려주세요.
            </p>
          </div>

          <pre
            className="req-note"
            style={{
              margin: 0,
              padding: 14,
              borderRadius: 8,
              background: "var(--inset)",
              whiteSpace: "pre-wrap",
              overflowWrap: "anywhere",
              userSelect: "text",
              cursor: "text",
            }}
          >
            {error.message}
          </pre>

          <button
            className="btn btn--key"
            style={{ alignSelf: "flex-start" }}
            onClick={() => this.setState({ error: null })}
          >
            다시 시도
          </button>
        </section>
      </main>
    );
  }
}
