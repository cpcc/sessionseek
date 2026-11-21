import { SOURCE_COLOR, SOURCE_LABEL, type Source } from "../types";

export function SourceBadge({ source, className = "" }: { source: Source; className?: string }) {
  return (
    <span
      className={`inline-block shrink-0 rounded border px-1.5 py-px text-[10px] font-medium ${SOURCE_COLOR[source]} ${className}`}
    >
      {SOURCE_LABEL[source]}
    </span>
  );
}

export function formatTime(ms?: number): string {
  if (!ms) return "—";
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "—";
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  if (sameDay) {
    return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
  }
  const sameYear = d.getFullYear() === now.getFullYear();
  return d.toLocaleDateString("zh-CN", {
    ...(sameYear ? {} : { year: "numeric" }),
    month: "2-digit",
    day: "2-digit",
  });
}

export function shortProject(project?: string): string {
  if (!project) return "未知项目";
  const parts = project.split("/").filter(Boolean);
  return parts[parts.length - 1] || project;
}
