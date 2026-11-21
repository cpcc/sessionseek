import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { formatTime, shortProject, SourceBadge } from "./bits";
import type { Message, Part, SessionMeta } from "../types";

function ToolCallBlock({ part }: { part: Extract<Part, { type: "tool_call" }> }) {
  const [open, setOpen] = useState(false);
  const statusColor =
    part.status === "error"
      ? "text-red-400"
      : part.status === "completed"
        ? "text-emerald-400"
        : "text-yellow-400";
  let inputPretty = part.inputJson;
  try {
    inputPretty = JSON.stringify(JSON.parse(part.inputJson), null, 2);
  } catch {
    /* 保留原样 */
  }
  return (
    <div className="my-1 overflow-hidden rounded-md border border-[#30363d] bg-[#161b22]">
      <button
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-[#1c2128]"
        onClick={() => setOpen(!open)}
      >
        <span className="text-[12px] text-[#7d8590]">{open ? "▾" : "▸"}</span>
        <span className="text-[12px] font-medium text-[#c9d1d9]">🔧 {part.name}</span>
        <span className={`ml-auto text-[11px] ${statusColor}`}>{part.status}</span>
      </button>
      {open && (
        <div className="border-t border-[#21262d] px-3 py-2">
          <div className="mb-1 text-[11px] text-[#7d8590]">输入</div>
          <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-all rounded bg-[#0d1117] p-2 font-mono text-[11.5px] text-[#a5b4c3]">
            {inputPretty || "(空)"}
          </pre>
          {part.output && (
            <>
              <div className="mt-2 mb-1 text-[11px] text-[#7d8590]">输出</div>
              <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded bg-[#0d1117] p-2 font-mono text-[11.5px] text-[#a5b4c3]">
                {part.output}
              </pre>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function ThinkingBlock({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="my-1 rounded-md border border-dashed border-[#30363d]">
      <button
        className="flex w-full items-center gap-2 px-3 py-1 text-left text-[12px] text-[#7d8590] hover:bg-[#161b22]"
        onClick={() => setOpen(!open)}
      >
        {open ? "▾" : "▸"} 💭 思考过程
      </button>
      {open && (
        <pre className="whitespace-pre-wrap px-3 pb-2 text-[12px] leading-relaxed text-[#8b949e]">
          {text}
        </pre>
      )}
    </div>
  );
}

function Parts({ parts, role }: { parts: Part[]; role: Message["role"] }) {
  return (
    <>
      {parts.map((p, i) => {
        if (p.type === "tool_call") return <ToolCallBlock key={i} part={p} />;
        if (p.type === "thinking") return <ThinkingBlock key={i} text={p.text} />;
        if (p.type === "meta")
          return (
            <pre key={i} className="my-1 overflow-auto rounded bg-[#161b22] p-2 text-[11px] text-[#7d8590]">
              {p.raw}
            </pre>
          );
        // text
        if (!p.text.trim()) return null;
        if (role === "assistant") {
          return (
            <div key={i} className="markdown">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{p.text}</ReactMarkdown>
            </div>
          );
        }
        return (
          <p key={i} className="whitespace-pre-wrap break-words">
            {p.text}
          </p>
        );
      })}
    </>
  );
}

function MessageItem({ msg }: { msg: Message }) {
  if (msg.role === "system") {
    return (
      <div className="mx-auto max-w-[90%] rounded border border-dashed border-[#30363d] px-3 py-1.5 text-[12px] text-[#7d8590]">
        <Parts parts={msg.parts} role={msg.role} />
      </div>
    );
  }
  const isUser = msg.role === "user";
  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
      <div
        className={`max-w-[85%] rounded-lg px-3.5 py-2 ${
          isUser
            ? "bg-[#1d4ed8]/25 border border-[#1d4ed8]/40"
            : "bg-[#161b22] border border-[#30363d]"
        }`}
      >
        <Parts parts={msg.parts} role={msg.role} />
        {msg.timestamp && (
          <div className="mt-1 text-right text-[10px] text-[#7d8590]">
            {formatTime(msg.timestamp)}
          </div>
        )}
      </div>
    </div>
  );
}

interface Props {
  meta?: SessionMeta;
  messages: Message[];
  total: number;
  loading: boolean;
  hasMore: boolean;
  onLoadMore: () => void;
  /** 跳转目标消息序号(搜索结果点击时),加载后滚动定位 */
  targetSeq?: number | null;
}

export default function Transcript({ meta, messages, total, loading, hasMore, onLoadMore, targetSeq }: Props) {
  useEffect(() => {
    if (targetSeq == null || messages.length === 0) return;
    const el = document.querySelector(`[data-seq="${targetSeq}"]`);
    el?.scrollIntoView({ block: "center" });
  }, [targetSeq, messages.length, meta?.id]);

  if (!meta) {
    return (
      <div className="flex flex-1 items-center justify-center text-[#7d8590]">
        {loading ? "加载中…" : "← 选择一个会话查看内容"}
      </div>
    );
  }
  return (
    <div className="flex min-w-0 flex-1 flex-col bg-[#0d1117]">
      <div className="border-b border-[#21262d] px-4 py-2.5">
        <div className="flex items-center gap-2">
          <SourceBadge source={meta.source} />
          <span className="truncate font-medium">{meta.title || "(无标题)"}</span>
          <span className="ml-auto shrink-0 text-[11px] text-[#7d8590]">
            {shortProject(meta.project)} · {meta.messageCount} 条消息
          </span>
        </div>
        <div className="mt-0.5 truncate text-[11px] text-[#7d8590]" title={meta.sourcePath}>
          {meta.sourcePath}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        <div className="mx-auto flex max-w-[860px] flex-col gap-2.5">
          {messages.map((m) => (
            <div key={m.seq} data-seq={m.seq}>
              <MessageItem msg={m} />
            </div>
          ))}
          {hasMore && (
            <button
              className="mx-auto my-2 rounded-md border border-[#30363d] px-3 py-1 text-[12px] text-[#8b949e] hover:bg-[#161b22]"
              onClick={onLoadMore}
            >
              加载更多消息({messages.length}/{total})
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
