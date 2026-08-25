import { useMemo, useState } from "react";

import { EMPTY_PERSON, personLabel, type Person } from "./api";

/**
 * 사내 명단 — 사람을 등록하고 조직도로 본다.
 *
 * 목록이 아니라 트리로 그리는 이유는, 명단이 스무 명을 넘으면 이름만 늘어선
 * 목록에서 원하는 사람을 찾지 못하기 때문이다. 상위자를 따라 내려가면 "어느
 * 팀의 누구"로 찾을 수 있다.
 *
 * 상위자를 반드시 지정해야 하는 것은 아니다. 조직도를 완성하는 게 목적이 아니라
 * 회의에서 화자를 고를 때 사람을 빨리 찾는 게 목적이다 — 이름만 넣어도 제
 * 역할을 한다.
 */

type Node = { person: Person; children: Node[] };

/**
 * 상위자 사슬을 따라 올라가다 같은 사람을 다시 만나는지.
 *
 * Rust 쪽에서 순환을 막으므로 정상적인 경로로는 생기지 않는다. 사용자가
 * `roster.json`을 직접 고친 경우를 위한 방어다 — 순환이 트리에 들어오면 아래
 * `Row`가 자기 자신을 무한히 그린다.
 */
function tangled(byId: Map<string, Person>, id: string): boolean {
  const seen = new Set<string>();
  let cursor: string | null | undefined = id;
  while (cursor) {
    if (seen.has(cursor)) return true;
    seen.add(cursor);
    cursor = byId.get(cursor)?.manager;
  }
  return false;
}

/**
 * 상위자 관계로 트리를 만든다.
 *
 * 상위자가 없거나, 그 상위자가 명단에 없거나, 사슬이 꼬인 사람이 뿌리가 된다.
 * "상위자가 명단에 없는 사람"을 뿌리로 올리는 게 특히 중요하다 — 팀장을
 * 명단에서 지웠을 때 그 밑의 팀원이 화면에서 통째로 사라지면 안 된다.
 */
function buildTree(people: Person[]): Node[] {
  const byId = new Map(people.map((p) => [p.id, p]));
  const nodes = new Map(people.map((p) => [p.id, { person: p, children: [] } as Node]));
  const roots: Node[] = [];

  for (const person of people) {
    const node = nodes.get(person.id)!;
    const parent = person.manager ? nodes.get(person.manager) : undefined;
    if (parent && parent !== node && !tangled(byId, person.id)) {
      parent.children.push(node);
    } else {
      roots.push(node);
    }
  }
  return roots;
}

/** 자기 자신과 그 아래 사람들. 상위자 후보에서 빼려고 쓴다. */
function descendantsOf(people: Person[], id: string): Set<string> {
  const out = new Set([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const person of people) {
      if (person.manager && out.has(person.manager) && !out.has(person.id)) {
        out.add(person.id);
        grew = true;
      }
    }
  }
  return out;
}

function Row({
  node,
  depth,
  onEdit,
}: {
  node: Node;
  depth: number;
  onEdit: (p: Person) => void;
}) {
  const { person, children } = node;
  return (
    <>
      <button
        className="org-row"
        style={{ paddingLeft: 10 + depth * 18 }}
        onClick={() => onEdit(person)}
      >
        <span className="org-name">{person.name}</span>
        {person.title && <span className="org-title">{person.title}</span>}
        {person.team && <span className="org-team">{person.team}</span>}
      </button>
      {children.map((child) => (
        <Row key={child.person.id} node={child} depth={depth + 1} onEdit={onEdit} />
      ))}
    </>
  );
}

function Editor({
  draft,
  people,
  busy,
  onChange,
  onSubmit,
  onDelete,
  onCancel,
}: {
  draft: Person;
  people: Person[];
  busy: boolean;
  onChange: (next: Person) => void;
  onSubmit: () => void;
  onDelete: () => void;
  onCancel: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const set = <K extends keyof Person>(key: K, value: Person[K]) =>
    onChange({ ...draft, [key]: value });

  // 자기 자신과 자기 아래 사람은 상위자가 될 수 없다 — 조직도가 순환한다.
  // Rust 쪽에서도 막지만, 고를 수 없게 두는 편이 오류 메시지보다 낫다.
  const blocked = draft.id ? descendantsOf(people, draft.id) : new Set<string>();

  return (
    <form
      className="org-editor"
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit();
      }}
    >
      <div className="org-fields">
        <div className="knob">
          <label htmlFor="org-name">이름</label>
          <input
            id="org-name"
            type="text"
            value={draft.name}
            placeholder="김서연"
            autoFocus
            onChange={(e) => set("name", e.target.value)}
          />
        </div>
        <div className="knob">
          <label htmlFor="org-title">직책</label>
          <input
            id="org-title"
            type="text"
            value={draft.title}
            placeholder="팀장"
            onChange={(e) => set("title", e.target.value)}
          />
        </div>
        <div className="knob">
          <label htmlFor="org-team">팀</label>
          <input
            id="org-team"
            type="text"
            value={draft.team}
            placeholder="플랫폼실"
            onChange={(e) => set("team", e.target.value)}
          />
        </div>
        <div className="knob">
          <label htmlFor="org-manager">상위자</label>
          <select
            id="org-manager"
            value={draft.manager ?? ""}
            onChange={(e) => set("manager", e.target.value || null)}
          >
            <option value="">없음</option>
            {people
              .filter((p) => !blocked.has(p.id))
              .map((p) => (
                <option key={p.id} value={p.id}>
                  {personLabel(p)}
                </option>
              ))}
          </select>
        </div>
      </div>

      <div className="org-actions">
        {draft.id &&
          (confirming ? (
            <button
              type="button"
              className="icon-btn danger"
              onClick={onDelete}
              disabled={busy}
            >
              정말 지울까요?
            </button>
          ) : (
            <button
              type="button"
              className="icon-btn"
              onClick={() => setConfirming(true)}
              disabled={busy}
            >
              삭제
            </button>
          ))}
        <button
          type="button"
          className="icon-btn"
          style={{ marginLeft: "auto" }}
          onClick={onCancel}
          disabled={busy}
        >
          취소
        </button>
        <button
          type="submit"
          className="org-save"
          disabled={busy || !draft.name.trim()}
        >
          저장
        </button>
      </div>
    </form>
  );
}

export function Roster({
  people,
  busy,
  error,
  onSave,
  onDelete,
  onClose,
}: {
  people: Person[];
  busy: boolean;
  error: string | null;
  /** 저장에 성공했는지. 실패하면 편집기를 닫지 않는다 — 입력을 잃지 않게. */
  onSave: (person: Person) => Promise<boolean>;
  onDelete: (id: string) => Promise<void>;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState<Person | null>(null);
  const tree = useMemo(() => buildTree(people), [people]);

  return (
    <div
      className="sheet-backdrop"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="sheet sheet--wide" role="dialog" aria-label="조직도">
        <div className="sheet-head">
          <h2>조직도</h2>
          <button className="icon-btn" onClick={onClose} aria-label="닫기">
            닫기
          </button>
        </div>

        <p className="org-lead">
          한 번 등록해두면 회의록에서 화자를 고르기만 하면 됩니다. 직책까지 넣으면
          요약의 액션 아이템에 담당자가 제대로 붙습니다.
        </p>

        {error && <div className="alarm">{error}</div>}

        {draft ? (
          <Editor
            draft={draft}
            people={people}
            busy={busy}
            onChange={setDraft}
            onSubmit={async () => {
              if (await onSave(draft)) setDraft(null);
            }}
            onDelete={async () => {
              await onDelete(draft.id);
              setDraft(null);
            }}
            onCancel={() => setDraft(null)}
          />
        ) : (
          <>
            <div className="org">
              {tree.length === 0 ? (
                <p className="org-empty">
                  아직 등록된 사람이 없습니다. 회의에 자주 들어오는 사람부터 넣어
                  보세요.
                </p>
              ) : (
                tree.map((node) => (
                  <Row key={node.person.id} node={node} depth={0} onEdit={setDraft} />
                ))
              )}
            </div>
            <button
              className="org-add"
              onClick={() => setDraft({ ...EMPTY_PERSON })}
              disabled={busy}
            >
              + 사람 추가
            </button>
          </>
        )}
      </div>
    </div>
  );
}
