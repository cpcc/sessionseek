import { useQuery } from "@tanstack/react-query";
import { getOverview } from "../lib/ipc";
import { SOURCE_LABEL } from "../types";

function fmt(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
  return String(n);
}

function Card({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="flex-1 rounded-lg border border-strong bg-panel px-4 py-3">
      <div className="text-[11px] text-faint">{label}</div>
      <div className="mt-1 text-[20px] font-semibold">{value}</div>
      {sub && <div className="mt-0.5 text-[11px] text-faint">{sub}</div>}
    </div>
  );
}

export default function StatsView() {
  const { data, isLoading } = useQuery({ queryKey: ["overview"], queryFn: getOverview });

  if (isLoading) {
    return <div className="flex flex-1 items-center justify-center text-faint">加载统计中…</div>;
  }
  if (!data) {
    return <div className="flex flex-1 items-center justify-center text-faint">暂无统计数据</div>;
  }

  const maxDay = Math.max(...data.byDay.map((d) => d.count), 1);
  const maxModel = Math.max(
    ...data.byModel.map((m) => m.inputTokens + m.outputTokens),
    1,
  );

  return (
    <div className="min-h-0 flex-1 overflow-y-auto bg-bg">
      <div className="mx-auto max-w-[860px] px-6 py-5">
        <h2 className="mb-4 text-[15px] font-semibold">统计总览</h2>

        <div className="flex gap-3">
          <Card label="会话" value={fmt(data.sessions)} />
          <Card label="消息" value={fmt(data.messages)} />
          <Card label="输入 tokens" value={fmt(data.inputTokens)} sub={`缓存命中 ${fmt(data.cacheReadTokens)}`} />
          <Card label="输出 tokens" value={fmt(data.outputTokens)} sub={`思考 ${fmt(data.reasoningTokens)}`} />
          <Card label="工具调用" value={fmt(data.toolCalls)} sub={`API 请求 ${fmt(data.apiCalls)}`} />
        </div>

        <h3 className="mb-2 mt-6 text-[13px] font-medium text-fg-soft">近 30 天会话数</h3>
        <div className="flex h-24 items-end gap-[3px] rounded-lg border border-strong bg-panel px-3 pb-2 pt-3">
          {data.byDay.length === 0 && (
            <div className="w-full text-center text-[12px] text-faint">近 30 天没有会话</div>
          )}
          {data.byDay.map((d) => (
            <div key={d.day} className="group relative flex-1" title={`${d.day}: ${d.count} 会话`}>
              <div
                className="w-full rounded-sm bg-[#1d4ed8]/60 group-hover:bg-[#1d4ed8]"
                style={{ height: `${Math.max((d.count / maxDay) * 72, 2)}px` }}
              />
            </div>
          ))}
        </div>

        <h3 className="mb-2 mt-6 text-[13px] font-medium text-fg-soft">按来源</h3>
        <table className="w-full text-[12.5px]">
          <thead>
            <tr className="text-left text-[11px] text-faint">
              <th className="py-1.5 pr-2 font-normal">来源</th>
              <th className="py-1.5 pr-2 text-right font-normal">会话</th>
              <th className="py-1.5 pr-2 text-right font-normal">消息</th>
              <th className="py-1.5 pr-2 text-right font-normal">输入</th>
              <th className="py-1.5 pr-2 text-right font-normal">输出</th>
              <th className="py-1.5 text-right font-normal">工具调用</th>
            </tr>
          </thead>
          <tbody>
            {data.bySource.map((s) => (
              <tr key={s.source} className="border-t border-line">
                <td className="py-1.5 pr-2">{SOURCE_LABEL[s.source]}</td>
                <td className="py-1.5 pr-2 text-right">{fmt(s.sessions)}</td>
                <td className="py-1.5 pr-2 text-right">{fmt(s.messages)}</td>
                <td className="py-1.5 pr-2 text-right">{fmt(s.inputTokens)}</td>
                <td className="py-1.5 pr-2 text-right">{fmt(s.outputTokens)}</td>
                <td className="py-1.5 text-right">{fmt(s.toolCalls)}</td>
              </tr>
            ))}
          </tbody>
        </table>

        <h3 className="mb-2 mt-6 text-[13px] font-medium text-fg-soft">按模型(Top 20)</h3>
        <div className="flex flex-col gap-1.5">
          {data.byModel.length === 0 && (
            <div className="text-[12px] text-faint">暂无模型用量数据(M1 前的旧索引需重新扫描)</div>
          )}
          {data.byModel.map((m) => {
            const total = m.inputTokens + m.outputTokens;
            return (
              <div key={m.model} className="flex items-center gap-2">
                <span className="w-56 shrink-0 truncate text-[12px]" title={m.model}>
                  {m.model}
                </span>
                <div className="h-3.5 flex-1 overflow-hidden rounded-sm bg-panel">
                  <div
                    className="h-full rounded-sm bg-gradient-to-r from-[#1d4ed8]/70 to-[#58a6ff]/70"
                    style={{ width: `${Math.max((total / maxModel) * 100, 1)}%` }}
                  />
                </div>
                <span className="w-40 shrink-0 text-right text-[11px] text-faint">
                  ↓{fmt(m.inputTokens)} ↑{fmt(m.outputTokens)} · {fmt(m.apiCalls)} 次
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
