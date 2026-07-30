import { useState, type ReactNode } from "react";

/**
 * 회의록 본문 렌더러.
 *
 * 요약 모델이 내놓는 마크다운의 부분집합만 다루되, **액션 아이템은 특별히
 * 취급한다.** 회의록에서 나중에 다시 열어보는 이유가 대개 "내가 뭘 하기로
 * 했더라"이기 때문에, 문단이 아니라 체크리스트로 세운다.
 *
 * 라이브러리를 쓰지 않고 React 엘리먼트를 직접 만든다. `dangerouslySetInnerHTML`을
 * 쓰지 않으므로 모델 출력에 HTML이 섞여도 실행될 여지가 없다.
 */

/** `**굵게**`와 `` `코드` ``만 처리한다. */
function inline(text: string, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  const pattern = /(\*\*([^*]+)\*\*)|(`([^`]+)`)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let i = 0;

  while ((m = pattern.exec(text)) !== null) {
    if (m.index > last) out.push(text.slice(last, m.index));
    if (m[2] !== undefined) out.push(<strong key={`${key}-b${i}`}>{m[2]}</strong>);
    else if (m[4] !== undefined) out.push(<code key={`${key}-c${i}`}>{m[4]}</code>);
    last = m.index + m[0].length;
    i++;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** 제목이 액션 아이템 절인지. 모델이 표현을 조금씩 바꿔서 넓게 잡는다. */
function isActionHeading(text: string): boolean {
  return /액션|실행\s*항목|할\s*일|to-?do|action/i.test(text);
}

type Todo = { text: string; who: string | null; when: string | null };

/**
 * "- [박대리] 결제사에 확답 요청 (기한: 오늘)" 같은 줄을 쪼갠다.
 *
 * 담당자는 `[이름]`, 기한은 문장 끝 괄호에서 찾는다. 형식이 안 맞으면 전부
 * 본문으로 두고 태그만 비운다 — 파싱 실패로 내용을 잃지 않는 게 우선이다.
 */
function parseTodo(raw: string): Todo {
  let text = raw.trim();
  let who: string | null = null;
  let when: string | null = null;

  const owner = /^\[([^\]]+)\]\s*/.exec(text);
  if (owner) {
    who = owner[1].trim();
    text = text.slice(owner[0].length);
  }

  // 끝에 붙은 괄호를 기한으로 본다. "기한:" 접두어는 떼어낸다.
  const tail = /\s*[（(]([^)）]*)[)）]\s*$/.exec(text);
  if (tail) {
    const inner = tail[1].trim();
    if (inner) {
      when = inner.replace(/^기한\s*[:：]\s*/, "").trim() || null;
      text = text.slice(0, tail.index).trim();
    }
  }

  // 담당자가 굵게 표시된 형태도 흔하다: "**박대리**: 할 일"
  if (!who) {
    const bold = /^\*\*([^*]+)\*\*\s*[:：]\s*/.exec(text);
    if (bold) {
      who = bold[1].trim();
      text = text.slice(bold[0].length);
    }
  }

  return { text: text.trim(), who, when };
}

function TodoList({ items }: { items: string[] }) {
  const [done, setDone] = useState<Set<number>>(new Set());

  const toggle = (i: number) =>
    setDone((prev) => {
      const next = new Set(prev);
      next.has(i) ? next.delete(i) : next.add(i);
      return next;
    });

  return (
    <ul className="todos">
      {items.map((raw, i) => {
        const todo = parseTodo(raw);
        const checked = done.has(i);
        return (
          <li key={i} className={`todo${checked ? " done" : ""}`}>
            <button
              className="todo-box"
              onClick={() => toggle(i)}
              role="checkbox"
              aria-checked={checked}
              aria-label={todo.text}
            >
              ✓
            </button>
            <div className="todo-body">
              <div className="todo-text">{inline(todo.text, `t${i}`)}</div>
              {(todo.who || todo.when) && (
                <div className="todo-tags">
                  {todo.who && <span className="tag">{todo.who}</span>}
                  {todo.when && (
                    <span className="tag tag--when">{todo.when}</span>
                  )}
                </div>
              )}
            </div>
          </li>
        );
      })}
    </ul>
  );
}

type Block =
  | { type: "heading"; text: string; action: boolean }
  | { type: "list"; ordered: boolean; items: string[]; action: boolean }
  | { type: "para"; text: string };

function parse(markdown: string): Block[] {
  const blocks: Block[] = [];
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");

  let para: string[] = [];
  let list: { ordered: boolean; items: string[] } | null = null;
  // 직전 제목이 액션 절이면 그 아래 목록은 체크리스트로 만든다.
  let underAction = false;

  const flushPara = () => {
    if (para.length) {
      blocks.push({ type: "para", text: para.join(" ") });
      para = [];
    }
  };
  const flushList = () => {
    if (list) {
      blocks.push({ type: "list", ...list, action: underAction });
      list = null;
    }
  };

  for (const rawLine of lines) {
    const line = rawLine.trim();

    if (!line) {
      flushPara();
      flushList();
      continue;
    }

    const heading = /^#{1,4}\s+(.*)$/.exec(line);
    if (heading) {
      flushPara();
      flushList();
      const text = heading[1].replace(/[*_`]/g, "").trim();
      underAction = isActionHeading(text);
      blocks.push({ type: "heading", text, action: underAction });
      continue;
    }

    const bullet = /^[-*]\s+(.*)$/.exec(line);
    const numbered = /^\d+[.)]\s+(.*)$/.exec(line);
    if (bullet || numbered) {
      flushPara();
      const ordered = !!numbered;
      // 체크박스 형태로 이미 왔으면 표시만 떼어낸다.
      const item = (bullet ?? numbered)![1].replace(/^\[[ xX]\]\s*/, "");
      if (!list || list.ordered !== ordered) {
        flushList();
        list = { ordered, items: [] };
      }
      list.items.push(item);
      continue;
    }

    flushList();
    para.push(line);
  }

  flushPara();
  flushList();
  return blocks;
}

export function Minutes({ markdown }: { markdown: string }) {
  const blocks = parse(markdown);

  return (
    <div className="body">
      {blocks.map((block, i) => {
        switch (block.type) {
          case "heading":
            return <h2 key={i}>{block.text}</h2>;
          case "list": {
            if (block.action) {
              return <TodoList key={i} items={block.items} />;
            }
            const items = block.items.map((item, j) => (
              <li key={j}>{inline(item, `l${i}-${j}`)}</li>
            ));
            return block.ordered ? (
              <ol key={i}>{items}</ol>
            ) : (
              <ul key={i}>{items}</ul>
            );
          }
          case "para":
            return <p key={i}>{inline(block.text, `p${i}`)}</p>;
        }
      })}
    </div>
  );
}
