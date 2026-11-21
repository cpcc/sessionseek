# SessionSeek

跨平台(macOS / Windows / Linux)桌面应用,统一浏览本地四种 AI Coding Harness 的会话记录:

- **Claude Code** — `~/.claude/projects/**/*.jsonl`
- **Codex** — `~/.codex/sessions/**/rollout-*.jsonl(.zst)`
- **ZCode** — `~/.zcode/cli/db/db.sqlite`
- **DeepSeek Harness** — `~/.dsh/sessions/**/session.vN.jsonl.zstd`

只读访问,不写入任何 harness 数据;所有索引存放在应用自己的数据目录
(`~/Library/Application Support/com.sessionseek.app/index.db` / Windows `%APPDATA%` 等)。

## 功能

- 四来源统一会话列表(虚拟滚动、来源/项目过滤、计数徽标)
- 会话回放:markdown 渲染、思考过程折叠、工具调用输入输出配对展示
- **全文搜索**(FTS5 trigram,中英文子串,≥3 字符),命中高亮 + 点击跳转到消息
- **统计页**:token 用量(输入/输出/缓存)、按来源/模型聚合、近 30 天会话分布
- **实时监听**(notify):harness 目录变更后自动增量索引;源文件删除会同步清理索引

## 开发

依赖:Node ≥ 20、Rust stable、以及 Tauri 系统依赖(见 [Tauri 文档](https://tauri.app/start/prerequisites/))。

```bash
npm install
npm run tauri dev     # 开发运行
cargo test --lib      # 解析器/索引库测试(src-tauri 下)
npm run tauri build   # 打包(DMG / AppImage+deb / NSIS)
```

## 发布

推送 `v*` 标签触发三平台构建(GitHub Actions),产出草稿 Release:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

初期为未签名包:macOS 首次打开需右键 → 打开;签名/公证的 secrets 接入见
`.github/workflows/release.yml` 注释。

## 架构

```
src/            React 18 + TS + Tailwind v4(界面)
src-tauri/      Rust 核心
  parsers/      四家格式解析器 → 归一化 SessionMeta/Message/Part
  store.rs      自有索引库(WAL;messages_fts trigram / session_stats)
  scan.rs       增量扫描(mtime/size 水位 + 孤儿清理)
  watcher.rs    notify 目录监听(3s 防抖)
```

里程碑与实测记录见 [PLAN.md](PLAN.md)。
