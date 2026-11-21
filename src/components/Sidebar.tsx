import { SOURCE_LABEL, SOURCE_ORDER, type Source, type Stats } from "../types";
import { shortProject } from "./bits";

interface Props {
  stats?: Stats;
  activeSource: Source | null;
  activeProject: string | null;
  onSourceChange: (s: Source | null) => void;
  onProjectChange: (p: string | null) => void;
  scanning: boolean;
  scanInfo: string;
  onRescan: () => void;
  searchValue: string;
  onSearchValueChange: (v: string) => void;
  onSearchSubmit: () => void;
  statsViewOpen: boolean;
  onToggleStatsView: () => void;
}

export default function Sidebar({
  stats,
  activeSource,
  activeProject,
  onSourceChange,
  onProjectChange,
  scanning,
  scanInfo,
  onRescan,
  searchValue,
  onSearchValueChange,
  onSearchSubmit,
  statsViewOpen,
  onToggleStatsView,
}: Props) {
  const countOf = (s: Source) => stats?.sources.find((x) => x.source === s)?.count ?? 0;
  const total = stats?.sources.reduce((acc, x) => acc + x.count, 0) ?? 0;

  return (
    <aside className="flex w-60 shrink-0 flex-col border-r border-[#21262d] bg-[#0d1117]">
      <div className="px-4 pt-4 pb-2">
        <div className="text-[15px] font-semibold tracking-wide">SessionSeek</div>
        <div className="mt-0.5 text-[11px] text-[#7d8590]">本地 AI 会话查看器</div>
      </div>

      <div className="px-3 pb-2">
        <input
          type="text"
          value={searchValue}
          onChange={(e) => onSearchValueChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onSearchSubmit();
          }}
          placeholder="全文搜索(≥3 字符,回车)"
          className="w-full rounded-md border border-[#30363d] bg-[#161b22] px-2.5 py-1.5 text-[12px] outline-none placeholder:text-[#7d8590] focus:border-[#58a6ff]"
        />
      </div>

      <div className="px-3 pb-2">
        <button
          className={`flex w-full items-center justify-between rounded-md px-2.5 py-1.5 text-left hover:bg-[#161b22] ${
            activeSource === null ? "bg-[#161b22]" : ""
          }`}
          onClick={() => {
            onSourceChange(null);
            onProjectChange(null);
          }}
        >
          <span className="font-medium">全部会话</span>
          <span className="text-[11px] text-[#7d8590]">{total}</span>
        </button>
        {SOURCE_ORDER.map((s) => (
          <button
            key={s}
            className={`flex w-full items-center justify-between rounded-md px-2.5 py-1.5 text-left hover:bg-[#161b22] ${
              activeSource === s ? "bg-[#161b22]" : ""
            }`}
            onClick={() => onSourceChange(s)}
          >
            <span className="font-medium">{SOURCE_LABEL[s]}</span>
            <span className="text-[11px] text-[#7d8590]">{countOf(s)}</span>
          </button>
        ))}
      </div>

      <div className="mx-3 mb-2 border-t border-[#21262d]" />

      <div className="px-4 pb-1 text-[11px] font-medium text-[#7d8590]">
        项目 {activeSource ? `· ${SOURCE_LABEL[activeSource]}` : ""}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-2">
        {(stats?.projects ?? []).map((p) => (
          <button
            key={p.project}
            title={p.project}
            className={`flex w-full items-center justify-between rounded-md px-2.5 py-1.5 text-left hover:bg-[#161b22] ${
              activeProject === p.project ? "bg-[#161b22]" : ""
            } ${p.project === "" ? "hidden" : ""}`}
            onClick={() =>
              onProjectChange(activeProject === p.project ? null : p.project)
            }
          >
            <span className="truncate">{shortProject(p.project)}</span>
            <span className="ml-2 shrink-0 text-[11px] text-[#7d8590]">{p.count}</span>
          </button>
        ))}
        {activeProject && (
          <button
            className="mt-1 w-full rounded-md border border-[#30363d] px-2.5 py-1.5 text-left text-[#8b949e] hover:bg-[#161b22]"
            onClick={() => onProjectChange(null)}
          >
            ✕ 清除项目过滤
          </button>
        )}
      </div>

      <div className="border-t border-[#21262d] px-3 py-2.5">
        <div className="mb-1.5 text-[11px] text-[#7d8590]">
          {scanning ? `⏳ 索引中… ${scanInfo}` : `✓ 索引就绪 ${scanInfo}`}
        </div>
        <div className="flex gap-2">
          <button
            className="flex-1 rounded-md border border-[#30363d] py-1 text-[12px] hover:bg-[#161b22] disabled:opacity-50"
            onClick={onRescan}
            disabled={scanning}
          >
            重新扫描
          </button>
          <button
            className={`rounded-md border border-[#30363d] px-2.5 py-1 text-[12px] hover:bg-[#161b22] ${
              statsViewOpen ? "border-[#58a6ff] text-[#58a6ff]" : ""
            }`}
            onClick={onToggleStatsView}
            title="统计总览"
          >
            📊
          </button>
        </div>
      </div>
    </aside>
  );
}
