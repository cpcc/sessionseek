// 镜像 Rust 归一化模型(src-tauri/src/model.rs)
export type Source = "claude" | "codex" | "zcode" | "dsh";

export interface SessionMeta {
  id: string;
  source: Source;
  project?: string;
  title?: string;
  startedAt?: number;
  updatedAt?: number;
  messageCount: number;
  sourcePath: string;
  fileSize: number;
  fileMtime: number;
  isSidechain: boolean;
}

export type Role = "user" | "assistant" | "system";

export type Part =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string }
  | { type: "tool_call"; name: string; inputJson: string; output?: string; status: string }
  | { type: "meta"; raw: string };

export interface Message {
  seq: number;
  role: Role;
  timestamp?: number;
  parts: Part[];
}

export interface SessionPage {
  items: SessionMeta[];
  total: number;
}

export interface MessagePage {
  meta: SessionMeta;
  items: Message[];
  total: number;
}

export interface SourceCount {
  source: Source;
  label: string;
  count: number;
}

export interface ProjectCount {
  project: string;
  count: number;
}

export interface Stats {
  sources: SourceCount[];
  projects: ProjectCount[];
}

export interface ScanProgress {
  source: string;
  done: number;
  total: number;
  updated: number;
}

export interface ScanSummary {
  claude: number;
  codex: number;
  zcode: number;
  dsh: number;
  skipped: number;
  durationMs: number;
}

// ---- M2:搜索 + 统计 ----

export interface SearchHit {
  sessionId: string;
  seq: number;
  snippet: string;
  role: Role;
  timestamp?: number;
  title?: string;
  source: Source;
  project?: string;
}

export interface SourceStat {
  source: Source;
  sessions: number;
  messages: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  toolCalls: number;
  apiCalls: number;
}

export interface ModelStat {
  model: string;
  apiCalls: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
}

export interface DayCount {
  day: string;
  count: number;
}

export interface OverviewStats {
  sessions: number;
  messages: number;
  inputTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  cacheReadTokens: number;
  toolCalls: number;
  apiCalls: number;
  bySource: SourceStat[];
  byModel: ModelStat[];
  byDay: DayCount[];
}

export const SOURCE_ORDER: Source[] = ["claude", "codex", "zcode", "dsh"];

export const SOURCE_LABEL: Record<Source, string> = {
  claude: "Claude",
  codex: "Codex",
  zcode: "ZCode",
  dsh: "DSH",
};

export const SOURCE_COLOR: Record<Source, string> = {
  claude: "text-orange-400 bg-orange-400/10 border-orange-400/30",
  codex: "text-sky-400 bg-sky-400/10 border-sky-400/30",
  zcode: "text-emerald-400 bg-emerald-400/10 border-emerald-400/30",
  dsh: "text-violet-400 bg-violet-400/10 border-violet-400/30",
};
