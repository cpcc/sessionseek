import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import Sidebar from "./components/Sidebar";
import SessionList from "./components/SessionList";
import Transcript from "./components/Transcript";
import SearchResults from "./components/SearchResults";
import StatsView from "./components/StatsView";
import {
  getStats,
  listSessions,
  getSession,
  searchMessages,
  onScanFinished,
  onScanProgress,
  scanNow,
} from "./lib/ipc";
import { SOURCE_LABEL, type ScanProgress, type SearchHit, type Source } from "./types";

const MSG_PAGE = 100;

export default function App() {
  const queryClient = useQueryClient();
  const [activeSource, setActiveSource] = useState<Source | null>(null);
  const [activeProject, setActiveProject] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [startOffset, setStartOffset] = useState(0);
  const [targetSeq, setTargetSeq] = useState<number | null>(null);
  const [searchValue, setSearchValue] = useState("");
  const [submittedQuery, setSubmittedQuery] = useState<string | null>(null);
  const [view, setView] = useState<"session" | "stats">("session");
  const [scanning, setScanning] = useState(false);
  const [scanInfo, setScanInfo] = useState("");
  const initialized = useRef(false);

  // ---- 扫描事件 ----
  useEffect(() => {
    const unlistens: (() => void)[] = [];
    onScanProgress((p: ScanProgress) => {
      setScanning(true);
      const label = SOURCE_LABEL[p.source as Source] ?? p.source;
      setScanInfo(p.total > 0 ? `${label} ${p.done}/${p.total}` : `${label} …`);
    }).then((un) => unlistens.push(un));
    onScanFinished((s) => {
      setScanning(false);
      const removed = s.removed > 0 ? `,清理 ${s.removed}` : "";
      setScanInfo(
        `(更新 ${s.claude + s.codex + s.zcode + s.dsh}${removed},耗时 ${(s.durationMs / 1000).toFixed(1)}s)`,
      );
      queryClient.invalidateQueries({ queryKey: ["stats"] });
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["overview"] });
      queryClient.invalidateQueries({ queryKey: ["search"] });
    }).then((un) => unlistens.push(un));
    return () => unlistens.forEach((u) => u());
  }, [queryClient]);

  // 首次进入先拉一次,让界面知道是否有数据
  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;
    getStats().then((st) => {
      const total = st.sources.reduce((a, x) => a + x.count, 0);
      if (total === 0) {
        setScanning(true);
        setScanInfo("等待首次索引…");
      }
    });
  }, []);

  const handleRescan = useCallback(() => {
    setScanning(true);
    setScanInfo("");
    scanNow();
  }, []);

  const selectSession = useCallback((id: string, seq?: number) => {
    setSelectedId(id);
    setTargetSeq(seq ?? null);
    setStartOffset(seq ? Math.max(Math.floor((seq - 1) / MSG_PAGE) * MSG_PAGE, 0) : 0);
    setView("session");
  }, []);

  // ---- 数据 ----
  const statsQ = useQuery({ queryKey: ["stats", activeSource], queryFn: () => getStats(activeSource) });

  const sessionsQ = useInfiniteQuery({
    queryKey: ["sessions", activeSource, activeProject],
    queryFn: ({ pageParam }) =>
      listSessions({ source: activeSource, project: activeProject, offset: pageParam as number, limit: 200 }),
    initialPageParam: 0,
    getNextPageParam: (last, all) => {
      const loaded = all.reduce((a, p) => a + p.items.length, 0);
      return loaded < last.total ? loaded : undefined;
    },
  });

  const msgsQ = useInfiniteQuery({
    queryKey: ["messages", selectedId, startOffset],
    queryFn: ({ pageParam }) => getSession(selectedId!, startOffset + (pageParam as number), MSG_PAGE),
    initialPageParam: 0,
    getNextPageParam: (last, all) => {
      const loaded = startOffset + all.reduce((a, p) => a + p.items.length, 0);
      return loaded < last.total ? loaded - startOffset : undefined;
    },
    enabled: !!selectedId,
  });

  const searchQ = useQuery({
    queryKey: ["search", submittedQuery, activeSource, activeProject],
    queryFn: () =>
      searchMessages({ query: submittedQuery!, source: activeSource, project: activeProject }),
    enabled: !!submittedQuery,
    staleTime: 10_000,
  });

  const sessions = useMemo(
    () => sessionsQ.data?.pages.flatMap((p) => p.items) ?? [],
    [sessionsQ.data],
  );
  const messages = useMemo(
    () => msgsQ.data?.pages.flatMap((p) => p.items) ?? [],
    [msgsQ.data],
  );

  const showSearch = submittedQuery !== null;
  const searching = searchQ.isFetching;

  const openHit = useCallback(
    (hit: SearchHit) => {
      selectSession(hit.sessionId, hit.seq);
    },
    [selectSession],
  );

  const handleSearchSubmit = useCallback(() => {
    const q = searchValue.trim();
    if (q.length === 0) {
      setSubmittedQuery(null);
      return;
    }
    if ([...q].length < 3) return; // trigram 最少 3 字符
    setSubmittedQuery(q);
  }, [searchValue]);

  // 选中会话被过滤/消失时清理选中态
  useEffect(() => {
    if (selectedId && sessionsQ.isFetched && sessions.length > 0 && !sessions.some((s) => s.id === selectedId)) {
      setSelectedId(null);
    }
  }, [selectedId, sessions, sessionsQ.isFetched]);

  return (
    <div className="flex h-full">
      <Sidebar
        stats={statsQ.data}
        activeSource={activeSource}
        activeProject={activeProject}
        onSourceChange={(s) => {
          setActiveSource(s);
          setActiveProject(null);
          setSelectedId(null);
        }}
        onProjectChange={(p) => {
          setActiveProject(p);
          setSelectedId(null);
        }}
        scanning={scanning}
        scanInfo={scanInfo}
        onRescan={handleRescan}
        searchValue={searchValue}
        onSearchValueChange={(v) => {
          setSearchValue(v);
          if (v === "") setSubmittedQuery(null);
        }}
        onSearchSubmit={handleSearchSubmit}
        statsViewOpen={view === "stats"}
        onToggleStatsView={() => setView(view === "stats" ? "session" : "stats")}
      />

      {showSearch ? (
        <SearchResults
          hits={searchQ.data ?? []}
          loading={searching}
          error={searchQ.error ? String(searchQ.error) : null}
          query={submittedQuery ?? ""}
          onOpenHit={openHit}
        />
      ) : (
        <SessionList
          sessions={sessions}
          selectedId={selectedId}
          onSelect={(id) => selectSession(id)}
          hasMore={sessionsQ.hasNextPage}
          total={sessionsQ.data?.pages[0]?.total ?? 0}
          onLoadMore={() => sessionsQ.fetchNextPage()}
          loading={sessionsQ.isLoading}
        />
      )}

      {view === "stats" ? (
        <StatsView />
      ) : (
        <Transcript
          meta={msgsQ.data?.pages[0]?.meta}
          messages={messages}
          total={msgsQ.data?.pages[0]?.total ?? 0}
          loading={msgsQ.isLoading}
          hasMore={msgsQ.hasNextPage}
          onLoadMore={() => msgsQ.fetchNextPage()}
          targetSeq={targetSeq}
        />
      )}
    </div>
  );
}
