import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { formatTime, shortProject, SourceBadge } from "./bits";
import type { SessionMeta } from "../types";

interface Props {
  sessions: SessionMeta[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  hasMore: boolean;
  total: number;
  onLoadMore: () => void;
  loading: boolean;
}

export default function SessionList({
  sessions,
  selectedId,
  onSelect,
  hasMore,
  total,
  onLoadMore,
  loading,
}: Props) {
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: sessions.length + (hasMore ? 1 : 0),
    getScrollElement: () => parentRef.current,
    estimateSize: () => 64,
    overscan: 10,
  });

  return (
    <div className="flex w-[340px] shrink-0 flex-col border-r border-[#21262d] bg-[#0d1117]">
      <div className="flex items-center justify-between border-b border-[#21262d] px-4 py-2.5">
        <span className="font-medium">会话</span>
        <span className="text-[11px] text-[#7d8590]">
          {loading ? "加载中…" : `共 ${total} 条`}
        </span>
      </div>
      <div ref={parentRef} className="min-h-0 flex-1 overflow-y-auto">
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((vItem) => {
            const idx = vItem.index;
            if (idx >= sessions.length) {
              return (
                <div
                  key="load-more"
                  ref={virtualizer.measureElement}
                  data-index={idx}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${vItem.start}px)`,
                  }}
                  className="p-3 text-center"
                >
                  <button
                    className="rounded-md border border-[#30363d] px-3 py-1 text-[12px] text-[#8b949e] hover:bg-[#161b22]"
                    onClick={onLoadMore}
                  >
                    加载更多(已显示 {sessions.length}/{total})
                  </button>
                </div>
              );
            }
            const s = sessions[idx];
            const selected = s.id === selectedId;
            return (
              <div
                key={s.id}
                ref={virtualizer.measureElement}
                data-index={idx}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${vItem.start}px)`,
                }}
              >
                <button
                  className={`block w-full border-b border-[#21262d]/60 px-3.5 py-2.5 text-left transition-colors ${
                    selected ? "bg-[#1c2128]" : "hover:bg-[#161b22]"
                  }`}
                  onClick={() => onSelect(s.id)}
                >
                  <div className="flex items-center gap-2">
                    <SourceBadge source={s.source} />
                    <span className="ml-auto shrink-0 text-[11px] text-[#7d8590]">
                      {formatTime(s.updatedAt)}
                    </span>
                  </div>
                  <div
                    className={`mt-1 line-clamp-2 text-[12.5px] leading-snug ${
                      selected ? "text-[#e6edf3]" : "text-[#c9d1d9]"
                    }`}
                  >
                    {s.title || "(无标题)"}
                  </div>
                  <div className="mt-1 flex items-center gap-2 text-[11px] text-[#7d8590]">
                    <span className="truncate">{shortProject(s.project)}</span>
                    <span className="shrink-0">· {s.messageCount} 条消息</span>
                  </div>
                </button>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
