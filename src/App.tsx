import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState, type FormEvent } from "react";
import NetworkSettings from "./NetworkSettings";
import ZLibraryAccount, { type ZLibraryAccountStatus } from "./ZLibraryAccount";

type CoreStatus = {
  name: string;
  version: string;
  rustCore: boolean;
  networkStack: string;
};

type ProviderDescriptor = {
  id: string;
  name: string;
  capabilities: {
    authenticated: boolean;
    downloadable: boolean;
    searchable: boolean;
    paginated: boolean;
  };
};

type BookFormat = "epub" | "pdf" | "mobi" | "azw3" | "txt" | "cbz" | "other";

type BookSummary = {
  id: string;
  title: string;
  authors: string[];
  year?: number | null;
  language?: string | null;
  format?: BookFormat | null;
  availableFormats: BookFormat[];
  sizeBytes?: number | null;
  coverUrl?: string | null;
};

type BookDetails = BookSummary & {
  description?: string | null;
  identifiers: Array<[string, string]>;
};

type SearchResult = {
  items: BookSummary[];
  page: number;
  hasNext: boolean;
};

type LibraryItem = {
  id: string;
  title: string;
  authors: string[];
  path: string;
  format: string;
  sizeBytes: number;
  sourceProvider?: string | null;
  sourceBookId?: string | null;
};

type DownloadState =
  | "queued"
  | "connecting"
  | "downloading"
  | "paused"
  | "verifying"
  | "completed"
  | "failed"
  | "cancelled";

type DownloadProgressEvent = {
  taskId: string;
  providerId: string;
  bookId: string;
  downloadedBytes: number;
  totalBytes?: number | null;
};

type DownloadReceipt = {
  path: string;
  item: LibraryItem;
};

type PersistedDownloadTask = {
  id: string;
  providerId: string;
  bookId: string;
  title: string;
  format: BookFormat;
  destination: string;
  state: DownloadState;
  downloadedBytes: number;
  totalBytes?: number | null;
  error?: string | null;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
};

type DownloadTask = {
  key: string;
  providerId: string;
  bookId: string;
  title: string;
  format: BookFormat;
  state: DownloadState;
  downloadedBytes: number;
  totalBytes?: number | null;
  path?: string;
  error?: string;
};

type AppSettings = {
  defaultProvider: string;
  searchPageSize: number;
  downloadDirectory: string;
  libraryDirectory: string;
  recursiveLibraryScan: boolean;
};

type Page = "home" | "search" | "downloads" | "library" | "account" | "settings";
type MainPage = Extract<Page, "home" | "search" | "downloads" | "library">;

const nav: Array<{ id: MainPage; label: string; glyph: string }> = [
  { id: "home", label: "总览", glyph: "◈" },
  { id: "search", label: "搜索", glyph: "⌕" },
  { id: "downloads", label: "下载", glyph: "⇣" },
  { id: "library", label: "书库", glyph: "▤" },
];

const pageMeta: Record<Page, { label: string; eyebrow: string }> = {
  home: { label: "总览", eyebrow: "OVERVIEW" },
  search: { label: "搜索", eyebrow: "DISCOVER" },
  downloads: { label: "下载", eyebrow: "TRANSFERS" },
  library: { label: "书库", eyebrow: "LOCAL LIBRARY" },
  account: { label: "账户", eyebrow: "ACCOUNT" },
  settings: { label: "设置", eyebrow: "PREFERENCES" },
};

const defaultSettings: AppSettings = {
  defaultProvider: "",
  searchPageSize: 24,
  downloadDirectory: "",
  libraryDirectory: "",
  recursiveLibraryScan: true,
};

function loadSettings(): AppSettings {
  try {
    const raw = localStorage.getItem("luminashelf.settings");
    return raw ? { ...defaultSettings, ...JSON.parse(raw) } : defaultSettings;
  } catch {
    return defaultSettings;
  }
}

function formatBytes(value?: number | null) {
  if (!value || value <= 0) return "—";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 10 || unit === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
}

function taskKey(providerId: string, bookId: string, format: BookFormat) {
  return `${providerId}:${bookId}:${format}`;
}

function taskProgress(task: DownloadTask) {
  if (!task.totalBytes || task.totalBytes <= 0) return task.state === "completed" ? 100 : 0;
  return Math.min(100, Math.round((task.downloadedBytes / task.totalBytes) * 100));
}

function taskLabel(task: DownloadTask) {
  switch (task.state) {
    case "completed": return "已完成";
    case "failed": return "失败";
    case "paused": return "已暂停";
    case "cancelled": return "已取消";
    case "queued": return "排队";
    case "connecting": return "连接中";
    case "verifying": return "校验中";
    case "downloading": return `${taskProgress(task)}%`;
  }
}

function preferredFormat(book: BookSummary): BookFormat | null {
  const candidates = [book.format, ...book.availableFormats].filter(Boolean) as BookFormat[];
  return candidates.find((format) => format !== "other") ?? candidates[0] ?? null;
}

function restoreDownloads(tasks: PersistedDownloadTask[]) {
  return Object.fromEntries(tasks.map((task) => [task.id, {
    key: task.id,
    providerId: task.providerId,
    bookId: task.bookId,
    title: task.title,
    format: task.format,
    state: task.state,
    downloadedBytes: task.downloadedBytes,
    totalBytes: task.totalBytes,
    path: task.destination,
    error: task.error ?? undefined,
  } satisfies DownloadTask]));
}

export default function App() {
  const [page, setPage] = useState<Page>("home");
  const [status, setStatus] = useState<CoreStatus | null>(null);
  const [providers, setProviders] = useState<ProviderDescriptor[]>([]);
  const [zlibraryStatus, setZlibraryStatus] = useState<ZLibraryAccountStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [settings, setSettings] = useState<AppSettings>(() => loadSettings());

  const [query, setQuery] = useState("");
  const [selectedProvider, setSelectedProvider] = useState("");
  const [searchResult, setSearchResult] = useState<SearchResult | null>(null);
  const [searching, setSearching] = useState(false);
  const [selectedBook, setSelectedBook] = useState<BookSummary | null>(null);
  const [bookDetails, setBookDetails] = useState<BookDetails | null>(null);
  const [detailsLoading, setDetailsLoading] = useState(false);

  const [downloads, setDownloads] = useState<Record<string, DownloadTask>>({});
  const [libraryItems, setLibraryItems] = useState<LibraryItem[]>([]);
  const [libraryLoading, setLibraryLoading] = useState(false);

  useEffect(() => {
    localStorage.setItem("luminashelf.settings", JSON.stringify(settings));
  }, [settings]);

  useEffect(() => {
    Promise.all([
      invoke<CoreStatus>("core_status"),
      invoke<ProviderDescriptor[]>("provider_descriptors"),
      invoke<ZLibraryAccountStatus>("zlibrary_status"),
      invoke<PersistedDownloadTask[]>("download_tasks"),
    ])
      .then(([nextStatus, nextProviders, nextZlibraryStatus, persistedDownloads]) => {
        setStatus(nextStatus);
        setProviders(nextProviders);
        setZlibraryStatus(nextZlibraryStatus);
        setDownloads(restoreDownloads(persistedDownloads));
        const preferred = nextProviders.some((item) => item.id === settings.defaultProvider)
          ? settings.defaultProvider
          : nextProviders.find((item) => !item.capabilities.authenticated)?.id ?? nextProviders[0]?.id ?? "";
        setSelectedProvider(preferred);
      })
      .catch((reason) => setError(String(reason)));
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<DownloadProgressEvent>("download-progress", (event) => {
      const progress = event.payload;
      setDownloads((current) => {
        const task = current[progress.taskId];
        if (!task) return current;
        return {
          ...current,
          [progress.taskId]: {
            ...task,
            state: "downloading",
            downloadedBytes: progress.downloadedBytes,
            totalBytes: progress.totalBytes,
            error: undefined,
          },
        };
      });
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const active = pageMeta[page];
  const selectedProviderDescriptor = providers.find((provider) => provider.id === selectedProvider);
  const providerNeedsLogin = Boolean(
    selectedProviderDescriptor?.capabilities.authenticated
      && selectedProvider === "zlibrary"
      && !zlibraryStatus?.signedIn,
  );
  const downloadTasks = useMemo(
    () => Object.values(downloads).sort((a, b) => {
      const aDone = a.state === "completed" ? 1 : 0;
      const bDone = b.state === "completed" ? 1 : 0;
      return aDone - bDone || a.title.localeCompare(b.title);
    }),
    [downloads],
  );
  const completedCount = downloadTasks.filter((task) => task.state === "completed").length;

  function changeProvider(providerId: string) {
    setSelectedProvider(providerId);
    setSearchResult(null);
    setSelectedBook(null);
    setBookDetails(null);
  }

  function handleZlibraryStatus(next: ZLibraryAccountStatus) {
    setZlibraryStatus(next);
    if (!next.signedIn && selectedProvider === "zlibrary") {
      setSearchResult(null);
      setSelectedBook(null);
      setBookDetails(null);
    }
  }

  async function runSearch(targetPage = 1) {
    if (!query.trim() || !selectedProvider || providerNeedsLogin) return;
    setSearching(true);
    setError(null);
    try {
      const result = await invoke<SearchResult>("search_books", {
        providerId: selectedProvider,
        text: query.trim(),
        page: targetPage,
        pageSize: settings.searchPageSize,
        formats: [],
      });
      setSearchResult(result);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSearching(false);
    }
  }

  function submitSearch(event: FormEvent) {
    event.preventDefault();
    void runSearch(1);
  }

  async function openDetails(book: BookSummary) {
    setSelectedBook(book);
    setBookDetails(null);
    setDetailsLoading(true);
    setError(null);
    try {
      const details = await invoke<BookDetails>("book_details", {
        providerId: selectedProvider,
        bookId: book.id,
      });
      setBookDetails(details);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setDetailsLoading(false);
    }
  }

  async function queueDownload(
    providerId: string,
    bookId: string,
    title: string,
    format: BookFormat,
  ) {
    if (providerId === "zlibrary" && !zlibraryStatus?.signedIn) {
      setPage("account");
      setError("继续 Z-Library 下载前需要重新登录账户。");
      return;
    }

    const key = taskKey(providerId, bookId, format);
    setDownloads((current) => {
      const existing = current[key];
      return {
        ...current,
        [key]: {
          key,
          providerId,
          bookId,
          title,
          format,
          state: "queued",
          downloadedBytes: existing?.downloadedBytes ?? 0,
          totalBytes: existing?.totalBytes,
          path: existing?.path,
        },
      };
    });
    setError(null);

    try {
      const receipt = await invoke<DownloadReceipt>("download_book", {
        providerId,
        bookId,
        title,
        format,
        downloadDir: settings.downloadDirectory.trim() || null,
      });
      setDownloads((current) => ({
        ...current,
        [key]: {
          ...current[key],
          key,
          providerId,
          bookId,
          title,
          format,
          state: "completed",
          path: receipt.path,
          downloadedBytes: receipt.item.sizeBytes,
          totalBytes: receipt.item.sizeBytes,
          error: undefined,
        },
      }));
      setLibraryItems((current) => [
        receipt.item,
        ...current.filter((item) => item.path !== receipt.item.path),
      ]);
    } catch (reason) {
      setDownloads((current) => ({
        ...current,
        [key]: {
          ...current[key],
          key,
          providerId,
          bookId,
          title,
          format,
          state: "failed",
          downloadedBytes: current[key]?.downloadedBytes ?? 0,
          error: String(reason),
        },
      }));
      setError(String(reason));
    }
  }

  async function startDownload(book: BookSummary) {
    const format = preferredFormat(book);
    if (!format || !selectedProvider) {
      setError("这本书没有可用的下载格式。");
      return;
    }
    await queueDownload(selectedProvider, book.id, book.title, format);
  }

  async function retryDownload(task: DownloadTask) {
    await queueDownload(task.providerId, task.bookId, task.title, task.format);
  }

  async function removeDownload(task: DownloadTask) {
    try {
      await invoke<boolean>("remove_download_task", { taskId: task.key });
      setDownloads((current) => {
        const next = { ...current };
        delete next[task.key];
        return next;
      });
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function scanLibrary() {
    const path = settings.libraryDirectory.trim();
    if (!path) {
      setError("先在书库页或设置页填写本地书库目录。");
      return;
    }
    setLibraryLoading(true);
    setError(null);
    try {
      const items = await invoke<LibraryItem[]>("scan_library", {
        path,
        recursive: settings.recursiveLibraryScan,
        maxItems: 1000,
      });
      setLibraryItems(items);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLibraryLoading(false);
    }
  }

  function resetSettings() {
    setSettings(defaultSettings);
    changeProvider(providers.find((provider) => !provider.capabilities.authenticated)?.id ?? providers[0]?.id ?? "");
  }

  function renderHome() {
    return (
      <>
        <section className="hero glass">
          <div>
            <span className="eyebrow">HIGH PERFORMANCE E-BOOK CLIENT</span>
            <h2>搜索、下载、本地书架，<br />现在是一条完整链路。</h2>
            <p>Provider 请求、网络解析和分段下载由 Rust Core 负责。下载队列已经持久化，应用重启后未完成任务会保留并可继续。</p>
            <button className="primary-button hero-action" onClick={() => setPage("search")}>开始搜索</button>
          </div>
          <div className="orb"><span>Z</span></div>
        </section>

        <section className="cards">
          <article className="card glass"><span>CORE</span><strong>{status?.networkStack ?? "Rust + Tokio"}</strong><small>统一网络栈在线</small></article>
          <article className="card glass"><span>LIBRARY</span><strong>{libraryItems.length}</strong><small>本轮已识别本地书籍</small></article>
          <article className="card glass"><span>DOWNLOADS</span><strong>{completedCount}/{downloadTasks.length}</strong><small>已完成 / 持久化任务</small></article>
        </section>

        <section className="workspace glass">
          <div className="workspace-title"><div><span className="eyebrow">PROVIDERS</span><h3>数据源状态</h3></div><span className="status-dot">● READY</span></div>
          <div className="provider-list">
            {providers.map((provider) => (
              <div className="provider-row" key={provider.id}>
                <div><strong>{provider.name}</strong><small>{provider.id}</small></div>
                <div className="provider-flags">
                  {provider.capabilities.searchable ? <span>搜索</span> : null}
                  {provider.capabilities.downloadable ? <span>下载</span> : null}
                  {provider.id === "zlibrary" ? <span>{zlibraryStatus?.signedIn ? "已登录" : "待登录"}</span> : <span>公开</span>}
                </div>
              </div>
            ))}
          </div>
        </section>
      </>
    );
  }

  function renderSearch() {
    return (
      <>
        <section className="search-panel glass">
          <form className="search-form" onSubmit={submitSearch}>
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="书名、作者、关键词…" autoFocus />
            <select value={selectedProvider} onChange={(event) => changeProvider(event.target.value)}>
              {providers.filter((provider) => provider.capabilities.searchable).map((provider) => (
                <option value={provider.id} key={provider.id}>{provider.name}</option>
              ))}
            </select>
            <button className="primary-button" disabled={searching || !query.trim() || providerNeedsLogin}>{searching ? "搜索中…" : "搜索"}</button>
          </form>
          <div className="search-hint">每页 {settings.searchPageSize} 项 · Provider 请求由 Rust Core 发起</div>
        </section>

        {providerNeedsLogin ? (
          <section className="auth-gate glass">
            <span className="auth-gate-mark">○</span>
            <div><span className="eyebrow">AUTHENTICATION REQUIRED</span><h3>先连接 Z-Library 账户</h3><p>这个 Provider 的搜索、详情和下载需要 EAPI Session。账户入口保留在左上角工具区。</p></div>
            <button className="primary-button" onClick={() => setPage("account")}>前往账户</button>
          </section>
        ) : searchResult ? (
          <section className="results-section">
            <div className="section-heading">
              <div><span className="eyebrow">RESULTS</span><h3>第 {searchResult.page} 页 · {searchResult.items.length} 项</h3></div>
              <div className="pager">
                <button className="ghost-button" disabled={searching || searchResult.page <= 1} onClick={() => void runSearch(searchResult.page - 1)}>上一页</button>
                <button className="ghost-button" disabled={searching || !searchResult.hasNext} onClick={() => void runSearch(searchResult.page + 1)}>下一页</button>
              </div>
            </div>
            <div className="book-grid">
              {searchResult.items.map((book) => {
                const format = preferredFormat(book);
                const key = format ? taskKey(selectedProvider, book.id, format) : "";
                const task = key ? downloads[key] : undefined;
                const activeTask = task && ["queued", "connecting", "downloading", "verifying"].includes(task.state);
                return (
                  <article className="book-card glass" key={book.id}>
                    <button className="cover-button" onClick={() => void openDetails(book)} aria-label={`查看 ${book.title} 详情`}>
                      {book.coverUrl ? <img src={book.coverUrl} alt="" loading="lazy" /> : <div className="cover-fallback">Z</div>}
                    </button>
                    <div className="book-copy">
                      <div className="book-meta"><span>{book.year ?? "年份未知"}</span><span>{book.language?.toUpperCase() ?? "—"}</span></div>
                      <h4 title={book.title}>{book.title}</h4>
                      <p>{book.authors.join(" · ") || "作者未知"}</p>
                      <div className="format-row">
                        {format ? <span className="format-badge">{format.toUpperCase()}</span> : null}
                        <small>{formatBytes(book.sizeBytes)}</small>
                      </div>
                      <div className="book-actions">
                        <button className="ghost-button" onClick={() => void openDetails(book)}>详情</button>
                        <button className="primary-button small" disabled={!format || Boolean(activeTask)} onClick={() => void startDownload(book)}>
                          {task?.state === "completed" ? "已下载" : activeTask ? "下载中" : task ? "继续" : "下载"}
                        </button>
                      </div>
                    </div>
                  </article>
                );
              })}
            </div>
          </section>
        ) : (
          <section className="empty-state glass"><span>⌕</span><h3>从一本书开始</h3><p>搜索结果直接使用 Rust Provider 返回的数据。</p></section>
        )}
      </>
    );
  }

  function renderDownloads() {
    return (
      <section className="workspace glass">
        <div className="workspace-title"><div><span className="eyebrow">PERSISTENT DOWNLOAD QUEUE</span><h3>下载任务</h3></div><span className="status-dot">{downloadTasks.length} TASKS</span></div>
        {downloadTasks.length === 0 ? (
          <div className="inline-empty">还没有下载任务。从搜索页选择一本书即可开始。</div>
        ) : (
          <div className="download-list">
            {downloadTasks.map((task) => {
              const progress = taskProgress(task);
              const canResume = ["paused", "failed", "cancelled"].includes(task.state);
              const isActive = ["queued", "connecting", "downloading", "verifying"].includes(task.state);
              return (
                <article className="download-row" key={task.key}>
                  <div className="download-head">
                    <div><strong>{task.title}</strong><small>{task.format.toUpperCase()} · {task.providerId}</small></div>
                    <span className={`task-state ${task.state}`}>{taskLabel(task)}</span>
                  </div>
                  <div className="progress-track"><i style={{ width: `${progress}%` }} /></div>
                  <div className="download-foot"><span>{formatBytes(task.downloadedBytes)} / {formatBytes(task.totalBytes)}</span><span>{task.error ?? task.path ?? (task.state === "paused" ? "上次退出时未完成，可继续" : "Rust segmented downloader")}</span></div>
                  <div className="download-actions">
                    {canResume ? <button className="primary-button small" onClick={() => void retryDownload(task)}>继续</button> : null}
                    {!isActive ? <button className="ghost-button" onClick={() => void removeDownload(task)}>移除记录</button> : null}
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>
    );
  }

  function renderLibrary() {
    return (
      <>
        <section className="search-panel glass">
          <div className="library-toolbar">
            <input value={settings.libraryDirectory} onChange={(event) => setSettings((current) => ({ ...current, libraryDirectory: event.target.value }))} placeholder="本地书库目录，例如 D:\\Books 或 /Users/me/Books" />
            <button className="primary-button" disabled={libraryLoading} onClick={() => void scanLibrary()}>{libraryLoading ? "扫描中…" : "扫描书库"}</button>
          </div>
          <div className="search-hint">{settings.recursiveLibraryScan ? "递归扫描子目录" : "仅扫描当前目录"} · 支持 EPUB / PDF / MOBI / AZW3 / TXT / CBZ / DJVU</div>
        </section>
        {libraryItems.length > 0 ? (
          <section className="library-grid">
            {libraryItems.map((item) => (
              <article className="library-card glass" key={`${item.id}:${item.path}`}>
                <div className="library-icon">{item.format.toUpperCase()}</div>
                <div><h4>{item.title}</h4><p>{item.authors.join(" · ") || "本地文件"}</p><small>{formatBytes(item.sizeBytes)} · {item.path}</small></div>
              </article>
            ))}
          </section>
        ) : (
          <section className="empty-state glass"><span>▤</span><h3>本地书架还是空的</h3><p>填写目录后扫描，或者从搜索页下载一本书。</p></section>
        )}
      </>
    );
  }

  function renderSettings() {
    return (
      <div className="settings-stack">
        <section className="settings-intro glass">
          <div><span className="eyebrow">APP PREFERENCES</span><h2>偏好与底层能力，都集中在这里。</h2><p>设置不是主要工作流，因此从主导航中独立出来。搜索、下载、本地书库、网络与核心配置统一放在这个页面。</p></div>
          <button className="ghost-button" onClick={resetSettings}>恢复默认</button>
        </section>

        <section className="settings-grid">
          <article className="setting-card glass">
            <span className="eyebrow">SEARCH</span><h3>搜索</h3>
            <label><span>默认 Provider<small>启动时优先使用</small></span><select value={settings.defaultProvider} onChange={(event) => { const value = event.target.value; setSettings((current) => ({ ...current, defaultProvider: value })); changeProvider(value || providers.find((provider) => !provider.capabilities.authenticated)?.id || providers[0]?.id || ""); }}><option value="">自动选择</option>{providers.map((provider) => <option value={provider.id} key={provider.id}>{provider.name}</option>)}</select></label>
            <label><span>每页结果数<small>1–50，由 Core 限制</small></span><select value={settings.searchPageSize} onChange={(event) => setSettings((current) => ({ ...current, searchPageSize: Number(event.target.value) }))}><option value={12}>12</option><option value={24}>24</option><option value={36}>36</option><option value={50}>50</option></select></label>
          </article>

          <article className="setting-card glass">
            <span className="eyebrow">DOWNLOAD & LIBRARY</span><h3>下载与书库</h3>
            <label className="stacked"><span>下载目录<small>留空使用系统 Downloads/LuminaShelf</small></span><input value={settings.downloadDirectory} onChange={(event) => setSettings((current) => ({ ...current, downloadDirectory: event.target.value }))} placeholder="默认系统下载目录" /></label>
            <label className="stacked"><span>书库目录<small>用于本地扫描</small></span><input value={settings.libraryDirectory} onChange={(event) => setSettings((current) => ({ ...current, libraryDirectory: event.target.value }))} placeholder="选择或填写本地书库目录" /></label>
            <label className="toggle-row"><span>递归扫描<small>同时扫描所有子目录</small></span><button className={`toggle ${settings.recursiveLibraryScan ? "on" : ""}`} onClick={() => setSettings((current) => ({ ...current, recursiveLibraryScan: !current.recursiveLibraryScan }))}><i /></button></label>
          </article>

          <NetworkSettings networkStack={status?.networkStack ?? "Tokio · Reqwest · Hickory"} />

          <article className="setting-card glass">
            <span className="eyebrow">CORE</span><h3>关于 LuminaShelf</h3>
            <div className="readout"><span>版本</span><strong>v{status?.version ?? "0.5.0"}</strong></div>
            <div className="readout"><span>架构</span><strong>Tauri 2 + React + Rust</strong></div>
            <div className="readout"><span>Core 状态</span><strong>{status?.rustCore ? "Online" : "Connecting"}</strong></div>
          </article>
        </section>
      </div>
    );
  }

  return (
    <div className="app-shell">
      <aside className="sidebar glass">
        <div className="brand-bar">
          <button className="brand" onClick={() => setPage("home")} aria-label="返回总览">
            <div className="brand-mark">Z</div>
            <div className="brand-copy"><strong>LuminaShelf</strong><span>Rust library client</span></div>
          </button>
          <div className="utility-buttons">
            <button className={page === "account" ? "active" : ""} onClick={() => setPage("account")} aria-label="账户" title="账户"><span className={zlibraryStatus?.signedIn ? "utility-online" : ""}>○</span></button>
            <button className={page === "settings" ? "active" : ""} onClick={() => setPage("settings")} aria-label="设置" title="设置">⚙</button>
          </div>
        </div>
        <nav className="primary-nav">
          {nav.map((item) => (
            <button key={item.id} className={page === item.id ? "active" : ""} onClick={() => setPage(item.id)}>
              <span className="glyph">{item.glyph}</span><span>{item.label}</span>
            </button>
          ))}
        </nav>
        <div className="core-pill"><i />{status?.rustCore ? "Rust Core online" : "Connecting core…"}</div>
      </aside>

      <main>
        <header className="page-header">
          <div className="page-heading">
            <div className="mobile-utilities">
              <button className={page === "account" ? "active" : ""} onClick={() => setPage("account")} aria-label="账户"><span className={zlibraryStatus?.signedIn ? "utility-online" : ""}>○</span></button>
              <button className={page === "settings" ? "active" : ""} onClick={() => setPage("settings")} aria-label="设置">⚙</button>
            </div>
            <div><span className="eyebrow">LUMINASHELF / {active.eyebrow}</span><h1>{active.label}</h1></div>
          </div>
          {page === "settings" || page === "account" ? <button className="ghost-button header-back" onClick={() => setPage("home")}>← 返回总览</button> : <div className="version-chip">v{status?.version ?? "0.5.0"}</div>}
        </header>

        {error ? <section className="notice error"><span>Core bridge</span><strong>{error}</strong><button onClick={() => setError(null)}>×</button></section> : null}

        {page === "home" ? renderHome() : null}
        {page === "search" ? renderSearch() : null}
        {page === "downloads" ? renderDownloads() : null}
        {page === "library" ? renderLibrary() : null}
        {page === "account" ? <ZLibraryAccount initialStatus={zlibraryStatus} onStatusChange={handleZlibraryStatus} /> : null}
        {page === "settings" ? renderSettings() : null}
      </main>

      <nav className="mobile-nav glass">
        {nav.map((item) => <button key={item.id} className={page === item.id ? "active" : ""} onClick={() => setPage(item.id)}><span>{item.glyph}</span><small>{item.label}</small></button>)}
      </nav>

      {selectedBook ? (
        <div className="detail-backdrop" onClick={() => setSelectedBook(null)}>
          <aside className="detail-drawer glass" onClick={(event) => event.stopPropagation()}>
            <button className="drawer-close" onClick={() => setSelectedBook(null)}>×</button>
            {selectedBook.coverUrl ? <img className="detail-cover" src={selectedBook.coverUrl} alt="" /> : <div className="detail-cover fallback">Z</div>}
            <span className="eyebrow">BOOK DETAILS</span>
            <h2>{selectedBook.title}</h2>
            <p className="detail-author">{selectedBook.authors.join(" · ") || "作者未知"}</p>
            <div className="detail-facts"><span>{selectedBook.year ?? "年份未知"}</span><span>{selectedBook.language?.toUpperCase() ?? "语言未知"}</span><span>{preferredFormat(selectedBook)?.toUpperCase() ?? "格式未知"}</span></div>
            <div className="detail-description">{detailsLoading ? "正在从 Core 获取详情…" : bookDetails?.description || "当前 Provider 没有提供简介。"}</div>
            <button className="primary-button wide" disabled={!preferredFormat(selectedBook)} onClick={() => void startDownload(selectedBook)}>下载到 LuminaShelf</button>
          </aside>
        </div>
      ) : null}
    </div>
  );
}
