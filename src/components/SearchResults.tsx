import { formatTime, shortProject, SourceBadge } from "./bits";
import type { SearchHit } from "../types";

/** snippet 中 ⟦⟧ 包裹的命中片段渲染为高亮。 */
function Highlighted({ text }: { text: string }) {
  const nodes: React.ReactNode[] = [];
  let buf = "";
  let mark = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === "⟦") {
      if (buf) nodes.push(mark ? <mark key={nodes.length}>{buf}</mark> : <span key={nodes.length}>{buf}</span>);
      buf = "";
      mark = true;
    } else if (ch === "⟧") {
      if (buf) nodes.push(<mark key={nodes.length}>{buf}</mark>);
      buf = "";
      mark = false;
    } else {
      buf += ch;
    }
  }
  if (buf) nodes.push(mark ? <mark key={nodes.length}>{buf}</mark> : <span key={nodes.length}>{buf}</span>);
  return <>{nodes}</>;
}

interface Props {
  hits: SearchHit[];
  loading: boolean;
  error: string | null;
  query: string;
  onOpenHit: (hit: SearchHit) => void;
}

export default function SearchResults({ hits, loading, error, query, onOpenHit }: Props) {
  return (
    <div className="flex w-[340px] shrink-0 flex-col border-r border-[#21262d] bg-[#0d1117]">
      <div className="flex items-center justify-between border-b border-[#21262d] px-4 py-2.5">
        <span className="font-medium">搜索结果</span>
        <span className="text-[11px] text-[#7d8590]">
          {loading ? "搜索中…" : error ? "—" : `${hits.length} 条命中`}
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {error && <div className="px-4 py-6 text-center text-[12px] text-[#f85149]">{error}</div>}
        {!error && !loading && hits.length === 0 && (
          <div className="px-4 py-6 text-center text-[12px] text-[#7d8590]">
            「{query}」没有命中任何消息
          </div>
        )}
        {hits.map((h, i) => (
          <button
            key={`${h.sessionId}:${h.seq}:${i}`}
            className="block w-full border-b border-[#21262d]/60 px-3.5 py-2.5 text-left hover:bg-[#161b22]"
            onClick={() => onOpenHit(h)}
          >
            <div className="flex items-center gap-2">
              <SourceBadge source={h.source} />
              <span className="truncate text-[12px] text-[#c9d1d9]">{h.title || "(无标题)"}</span>
              <span className="ml-auto shrink-0 text-[11px] text-[#7d8590]">{formatTime(h.timestamp)}</span>
            </div>
            <div className="mt-1 line-clamp-3 whitespace-pre-wrap break-all text-[11.5px] leading-snug text-[#8b949e]">
              <Highlighted text={h.snippet} />
            </div>
            <div className="mt-1 text-[10.5px] text-[#7d8590]">
              {shortProject(h.project)} · {h.role} · #{h.seq}
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}
