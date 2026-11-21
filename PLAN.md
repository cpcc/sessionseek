# SessionSeek · Tauri 骨架搭建计划(M1)

> 日期:2026-11-21
> 目标:跨平台(macOS / Windows / Linux)Tauri 桌面应用,统一浏览本地四种 AI harness 的会话。
> 覆盖:Claude Code、Codex、ZCode、DeepSeek Harness(本机实测约 1074 个会话)。

## 背景(调研结论)

- 四家存储格式均已在本机实测采样确认,竞品 agent-sessions(macOS-only,MIT)验证了 Codex/DSH 格式的可解析性;**ZCode SQLite 是无人支持过的空白点**。
- 没有任何现有工具支持 ZCode(SQLite)和 DSH(zstd 事件日志),也没有 Windows 上覆盖这四家的统一查看器。

## 环境前置(需安装)

1. **安装 Rust**(本机缺失):`winget install Rustlang.Rustup -e`,然后 `rustup default stable-msvc`。
   - MSVC BuildTools 14.40 + Win SDK 已就位(`C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools`),无需再装。
   - 若 winget 不可用则下载 rustup-init.exe 静默安装。
2. Node v24.19 / npm 已就绪,WebView2 系统自带。

## 项目结构

```
sessionseek/
├── package.json  vite.config.ts  index.html  tsconfig.json
├── src/                          # React 18 + TS + Tailwind v4
│   ├── main.tsx  App.tsx
│   ├── types.ts                  # 镜像 Rust 归一化模型
│   ├── lib/ipc.ts                # invoke 封装(listSessions/getSession/rescan)
│   └── components/
│       ├── Sidebar.tsx           # 来源过滤(带计数)+ 项目分组
│       ├── SessionList.tsx       # TanStack Virtual 虚拟列表,分页加载
│       └── Transcript.tsx        # 回放:用户/助手/工具/思考块渲染
└── src-tauri/
    ├── Cargo.toml  tauri.conf.json  capabilities/default.json
    └── src/
        ├── lib.rs  commands.rs   # #[tauri::command]: list_sessions, get_session, get_stats, scan_now
        ├── model.rs              # 归一化模型
        ├── store.rs              # 索引 SQLite(rusqlite bundled,放 app_data_dir/index.db)
        ├── scan.rs               # 扫描调度:mtime/size 增量
        └── parsers/{mod,claude,codex,zcode,dsh}.rs
```

## 归一化模型(核心抽象)

```rust
struct SessionMeta {
    id, source(Claude|Codex|ZCode|Dsh), project, title,
    started_at, updated_at, message_count, source_path,
}
struct Message { seq, role(User|Assistant|System), timestamp, parts: Vec<Part> }
enum Part {
    Text(String),
    Thinking(String),
    ToolCall { name, input_json, output, status },
    Meta(String), // 其余原样 JSON
}
```

- 消息以 `payload_json` 行存入索引库 `messages(session_id, seq, role, time, payload_json)`。
- FTS / 统计表留给 M2。

## 解析器要点(全部基于实测样本)

### claude.rs
- 遍历 `~/.claude/projects/<munged>/*.jsonl`
- 取 `type ∈ {user, assistant}` 记录;`message.content`(字符串或块数组:text / tool_use / thinking / tool_result)
- `parentUuid` 忽略(按行序线性化);`isSidechain=true` 标记为子会话
- `cwd` 反解为项目名;`ai-title` 记录作标题备选

### codex.rs
- `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl(.zst)`;zstd `stream::Decoder` 流式解压
- `session_meta.payload.cwd` 定项目;`response_item` 里的 message / function_call / function_call_output 转 Part;`turn_context` / `event_msg` 归 Meta

### zcode.rs
- 直接只读打开 `~/.zcode/cli/db/db.sqlite`(`SQLITE_OPEN_READ_ONLY` + `busy_timeout(5s)`;若撞锁则把 db+wal 拷临时目录以 immutable 打开)
- `session` 表取标题/目录/时间;`message` + `part` 按 session_id / sequence 拼装
- `part.data.type ∈ {text, reasoning, tool}` 映射 Part,其余归 Meta
- 按 `time_updated` 水位增量

### dsh.rs
- `~/.dsh/sessions/<munged>/<id>/session.vN.jsonl.zstd`(N=1/3/4 并存)
- zstd 解压后按首行 `session.version` 分派解析
- `user/message` → User;`assistant/message` → Assistant;`tool/call` + `tool/result` 按 call id 配对成 ToolCall
- `turn/step/compaction/...` 归 Meta;`session/title` 事件作标题

- 每个解析器配 1 个用真实截取样本的 `#[cfg(test)]` 测试(样本放 `src-tauri/tests/fixtures/`)

## 前端

- Tailwind v4 + TanStack Query / Virtual;深色主题,界面中文
- 布局:左侧栏(来源过滤 + 项目树 + 统计徽标)| 中间会话列表(虚拟化,`list_sessions(offset, limit, source, project)`)| 右侧/全屏会话回放(`get_session(id)` 分页 100 条)
- 回放块渲染:
  - user:蓝色气泡
  - assistant:react-markdown + remark-gfm
  - thinking:灰色可折叠
  - tool call:折叠面板(名称 + JSON 高亮 + 输出截断展开)
- 应用启动时触发首次索引(1074 会话预计 < 1 分钟,进度事件推给前端)

## Tauri 配置

- identifier `com.sessionseek.app`,窗口 1280×800,深色默认
- 权限只用 core default(文件读取全在 Rust 侧,无需 fs 插件 / scope)
- 模板默认图标
- Cargo 依赖:tauri 2、serde / serde_json、rusqlite(bundled)、zstd、chrono

## 实施步骤

1. 装 rustup(Rust 缺失)→ `cargo --version` 验证
2. `npm create tauri-app`(react-ts 模板,非交互参数)合并到工作区根目录;`npm install` + 前端依赖
3. 写 Rust 侧:model → store → parsers×4(含 fixture 测试)→ scan → commands
4. 写前端:types → ipc → Sidebar / SessionList / Transcript → App 组装
5. `cargo check` → `npm run tauri dev` 联调

## 验证

- `cargo test`:四个解析器 fixture 测试通过
- `npm run tauri dev` 启动后:
  1. 会话列表出现四个来源的真实数据(本机 203 + 88 + 505 + 278)
  2. 打开一个 Claude 会话(含中文 prompt)检查消息/工具块渲染
  3. 打开 ZCode 会话验证 SQLite 解析
  4. 列表按时间排序、来源过滤计数正确
- 已知风险:首次 Rust 编译 5-10 分钟;ZCode 1GB 库首扫耗时单独记录,如超 2 分钟改为后台增量

## 里程碑划分

| 里程碑 | 内容 | 状态 |
|---|---|---|
| **M1(本计划)** | 四家解析器 + 归一化索引 + 会话列表/回放,Windows 本机跑通 | ✅ 已完成(2026-11-21 于 macOS 实测:307 会话 / 37710 消息入库;claude 36 + codex 90 + zcode 171 + dsh 10) |
| M2 | FTS5 trigram 中文全文搜索 + 统计页(ZCode `turn_usage`/`model_usage` 现成) | 规划中 |
| M3 | notify 文件监听增量 + tauri-action 三平台 CI + 签名分发 | 规划中 |

## M1 实测补充说明(2026-11-21,macOS arm64)

- 本机 Rust 1.95 已就位(`~/.cargo/bin` 不在默认 PATH,需 `export PATH="$HOME/.cargo/bin:$PATH"`)。
- 索引位置:`~/Library/Application Support/com.sessionseek.app/index.db`(WAL 模式;扫描线程写,命令各开只读连接)。
- 与计划的三处有意偏差:
  1. codex 的 `event_msg`/`turn_context` 与 dsh 的 `turn/*`、`step/*`、`llm/*` 等纯运行时事件**未入库**(原计划归 Meta;避免索引膨胀)。
  2. zcode part 的 `step-start/step-finish/timeline/file/compaction` 跳过(file 为图片 artifact,M2 可考虑)。
  3. `scan_status` 命令未实现,进度走 `scan-progress` / `scan-finished` 事件。
- 图标:用临时脚本生成基底 PNG 后经 `npx tauri icon` 产出全套;`app-icon.png` 在仓库根。
- 已知待观察:zcode 源库为活库(WAL),若撞锁会自动回退快照拷贝;增量水位按 `session.time_updated`。
