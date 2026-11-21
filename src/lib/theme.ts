export type Theme = "dark" | "light";

const KEY = "sessionseek-theme";

/** 读取持久化主题;无记录时跟随系统。 */
export function initialTheme(): Theme {
  try {
    const stored = localStorage.getItem(KEY);
    if (stored === "light" || stored === "dark") return stored;
    if (window.matchMedia("(prefers-color-scheme: light)").matches) return "light";
  } catch {
    /* ignore */
  }
  return "dark";
}

/** 应用主题:html data-theme + 持久化 + 原生窗口主题同步。 */
export async function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    /* ignore */
  }
  // 同步原生窗口(标题栏)主题;能力缺失时静默忽略
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setTheme(theme);
  } catch {
    /* ignore */
  }
}
