import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  MessagePage,
  ScanProgress,
  ScanSummary,
  SessionPage,
  Source,
  Stats,
} from "../types";

export async function listSessions(opts: {
  source?: Source | null;
  project?: string | null;
  offset?: number;
  limit?: number;
}): Promise<SessionPage> {
  return invoke("list_sessions", {
    source: opts.source ?? null,
    project: opts.project ?? null,
    offset: opts.offset ?? 0,
    limit: opts.limit ?? 200,
  });
}

export async function getSession(id: string, offset = 0, limit = 100): Promise<MessagePage> {
  return invoke("get_session", { id, offset, limit });
}

export async function getStats(source?: Source | null): Promise<Stats> {
  return invoke("get_stats", { source: source ?? null });
}

export async function scanNow(): Promise<boolean> {
  return invoke("scan_now");
}

export function onScanProgress(cb: (p: ScanProgress) => void): Promise<UnlistenFn> {
  return listen<ScanProgress>("scan-progress", (e) => cb(e.payload));
}

export function onScanFinished(cb: (s: ScanSummary) => void): Promise<UnlistenFn> {
  return listen<ScanSummary>("scan-finished", (e) => cb(e.payload));
}
