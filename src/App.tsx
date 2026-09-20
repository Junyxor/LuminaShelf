import { invoke } from "@tauri-apps/api/core";
import { onBackButtonPress } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import NetworkSettings from "./NetworkSettings";
import PdfReader from "./PdfReader";
import ReaderView from "./ReaderView";
import { parseReaderPosition } from "./readerPosition";
import LibraryActions from "./LibraryActions";
import ZLibraryAccount, { type ZLibraryAccountStatus } from "./ZLibraryAccount";

type CoreStatus = {
  name: string;
  version: string;
  rustCore: boolean;
  platform: string;
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

type ReaderChapterMetadata = {
  id: string;
  title: string;
};

type ReaderBookMetadata = {
  title: string;
  chapters: ReaderChapterMetadata[];
};

type ReadingProgress = {
  libraryId: string;
  locator?: string | null;
  fraction: number;
  updatedAtUnixMs: number;
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

type ZLibraryRestoreResult = {
  status: ZLibraryAccountStatus;
  restored: boolean;
  warning?: string | null;
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
  const [recentSearches, setRecentSearches] = useState<string[]>(() => {
    try { return JSON.parse(localStorage.getItem("luminashelf.recentSearches") || "[]").filter((value: unknown) => typeof value === "string").slice(0, 12); } catch { return []; }
  });
  const [downloadFormat, setDownloadFormat] = useState<BookFormat | "">("");
  const searchVersion = useRef(0);
  const detailsVersion = useRef(0);
  const pendingDownloads = useRef(new Set<string>());
  const [taskActions, setTaskActions] = useState<Set<string>>(new Set());
  const [libraryQuery, setLibraryQuery] = useState("");
  const [librarySort, setLibrarySort] = useState("recent");
  const [manageItem, setManageItem] = useState<LibraryItem | null>(null);
  const [importMessage, setImportMessage] = useState<string | null>(null);
  const [selectedProvider, setSelectedProvider] = useState("");
  const [searchResult, setSearchResult] = useState<SearchResult | null>(null);
  const [searching, setSearching] = useState(false);
  const [selectedBook, setSelectedBook] = useState<BookSummary | null>(null);
  const [bookDetails, setBookDetails] = useState<BookDetails | null>(null);
  const [detailsLoading, setDetailsLoading] = useState(false);

  const [downloads, setDownloads] = useState<Record<string, DownloadTask>>({});
  const [libraryItems, setLibraryItems] = useState<LibraryItem[]>([]);
  const [libraryLoading, setLibraryLoading] = useState(false);
  const [readerItem, setReaderItem] = useState<LibraryItem | null>(null);
  const [readerBook, setReaderBook] = useState<ReaderBookMetadata | null>(null);
  const [readerChapterIndex, setReaderChapterIndex] = useState(0);
  const [readerInitialScroll, setReaderInitialScroll] = useState(0);
  const [readerLoading, setReaderLoading] = useState(false);
  const [pdfItem, setPdfItem] = useState<LibraryItem | null>(null);

  useEffect(() => {
    if (status?.platform !== "android" || (page === "home" && !readerItem && !pdfItem && !selectedBook && !manageItem)) return;
    let disposed = false;
    let stop: (() => Promise<void>) | undefined;
    void onBackButtonPress(() => {
      if (manageItem) setManageItem(null);
      else if (pdfItem) setPdfItem(null);
      else if (readerItem) closeReader();
      else if (selectedBook) setSelectedBook(null);
      else setPage("home");
    }).then((listener) => {
      if (disposed) void listener.unregister();
      else stop = () => listener.unregister();
    }).catch((reason) => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; void stop?.(); };
  }, [status?.platform, page, readerItem, readerBook, readerChapterIndex, pdfItem, selectedBook, manageItem]);

  useEffect(() => {
    try { localStorage.setItem("luminashelf.settings", JSON.stringify(settings)); } catch { /* Storage may be unavailable. */ }
  }, [settings]);

  useEffect(() => {
    Promise.all([
      invoke<CoreStatus>("core_status"),
      invoke<ProviderDescriptor[]>("provider_descriptors"),
      invoke<ZLibraryRestoreResult>("zlibrary_restore_session"),
      invoke<PersistedDownloadTask[]>("download_tasks"),
      invoke<LibraryItem[]>("library_items"),
    ])
      .then(([nextStatus, nextProviders, zlibraryRestore, persistedDownloads, nextLibraryItems]) => {
        setStatus(nextStatus);
        setLibraryItems(nextLibraryItems);
        setProviders(nextProviders);
        setZlibraryStatus(zlibraryRestore.status);
        setDownloads(restoreDownloads(persistedDownloads));
        if (zlibraryRestore.warning) {
          setError(`安全 Session 恢复失败：${zlibraryRestore.warning}`);
        }
        const preferred = nextProviders.some((item) => item.id === settings.defaultProvider)
          ? settings.defaultProvider
          : nextProviders.find((item) => !item.capabilities.authenticated)?.id ?? nextProviders[0]?.id ?? "";
        setSelectedProvider(preferred);
      })
      .catch((reason) => setError(String(reason)));
  }, []);

  useEffect(() => {
    let disposed = false;
    const stops: Array<() => void> = [];
    const subscriptions = [
      listen<PersistedDownloadTask>("download-task-updated", ({ payload }) => {
        setDownloads((current) => ({ ...current, ...restoreDownloads([payload]) }));
      }),
      listen("library-updated", () => {
        void invoke<LibraryItem[]>("library_items").then(setLibraryItems).catch((reason) => setError(String(reason)));
      }),
    ];
    for (const subscription of subscriptions) {
      void subscription.then((stop) => {
        if (disposed) stop();
        else stops.push(stop);
      }).catch((reason) => { if (!disposed) setError(String(reason)); });
    }
    return () => { disposed = true; stops.forEach((stop) => stop()); };
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
  const readerFraction = readerBook?.chapters.length
    ? (readerChapterIndex + 1) / readerBook.chapters.length
    : 0;

  function changeProvider(providerId: string) {
    searchVersion.current += 1;
    detailsVersion.current += 1;
    setSearching(false);
    setDetailsLoading(false);
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

  async function refreshDownloads() {
    const persisted = await invoke<PersistedDownloadTask[]>("download_tasks");
    setDownloads(restoreDownloads(persisted));
  }

  async function runSearch(targetPage = 1) {
    if (!query.trim() || !selectedProvider || providerNeedsLogin) return;
    const version = ++searchVersion.current;
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
      if (version === searchVersion.current) {
        setSearchResult(result);
        const recent = [query.trim(), ...recentSearches.filter((value) => value !== query.trim())].slice(0, 12);
        setRecentSearches(recent);
        try { localStorage.setItem("luminashelf.recentSearches", JSON.stringify(recent)); } catch {}
      }
    } catch (reason) {
      if (version === searchVersion.current) setError(`搜索失败，请检查网络或在设置中调整 DNS，然后重试。详情：${String(reason)}`);
    } finally {
      if (version === searchVersion.current) setSearching(false);
    }
  }

  function submitSearch(event: FormEvent) {
    event.preventDefault();
    void runSearch(1);
  }

  async function openDetails(book: BookSummary) {
    const version = ++detailsVersion.current;
    setSelectedBook(book);
    setDownloadFormat(preferredFormat(book) ?? "");
    setBookDetails(null);
    setDetailsLoading(true);
    setError(null);
    try {
      const details = await invoke<BookDetails>("book_details", {
        providerId: selectedProvider,
        bookId: book.id,
      });
      if (version === detailsVersion.current) setBookDetails(details);
    } catch (reason) {
      if (version === detailsVersion.current) setError(String(reason));
    } finally {
      if (version === detailsVersion.current) setDetailsLoading(false);
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
    if (pendingDownloads.current.has(key)) return;
    pendingDownloads.current.add(key);
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
      const task = await invoke<PersistedDownloadTask>("enqueue_download", {
        providerId, bookId, title, format,
        downloadDir: settings.downloadDirectory.trim() || null,
      });
      setDownloads((current) => current[key]?.state !== "queued"
        ? current : { ...current, ...restoreDownloads([task]) });
    } catch (reason) {
      const message = String(reason);
      try {
        await refreshDownloads();
      } catch (refreshReason) {
        setError(`${message} · 队列刷新失败：${String(refreshReason)}`);
        return;
      }
      if (message.includes("__LUMINA_PAUSED__") || message.includes("__LUMINA_CANCELLED__")) {
        return;
      }
      setError(message);
    } finally {
      pendingDownloads.current.delete(key);
    }
  }

  async function startDownload(book: BookSummary, chosen?: BookFormat) {
    const format = chosen ?? preferredFormat(book);
    if (!format || !selectedProvider) {
      setError("这本书没有可用的下载格式。");
      return;
    }
    await queueDownload(selectedProvider, book.id, book.title, format);
  }

  async function retryDownload(task: DownloadTask) {
    await queueDownload(task.providerId, task.bookId, task.title, task.format);
  }

  async function controlDownload(task: DownloadTask, command: "pause_download" | "cancel_download") {
    setTaskActions((current) => new Set(current).add(task.key));
    try {
      await invoke<boolean>(command, { taskId: task.key });
      await refreshDownloads();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setTaskActions((current) => { const next = new Set(current); next.delete(task.key); return next; });
    }
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

  async function backupLibrary(restore: boolean) {
    setLibraryLoading(true);
    setError(null);
    setImportMessage(restore ? "请选择书库备份文件…" : "正在打包书籍和阅读进度…");
    try {
      if (restore) {
        const result = await invoke<{ imported: number; items: LibraryItem[] } | null>("restore_library_backup");
        if (result) {
          setLibraryItems(result.items);
          setImportMessage(`已恢复 ${result.imported} 本书及阅读位置、书签。`);
        } else setImportMessage(null);
      } else {
        const count = await invoke<number | null>("export_library_backup");
        setImportMessage(count === null ? null : `已导出 ${count} 本书，备份不包含账户凭据。`);
      }
    } catch (reason) { setError(String(reason)); setImportMessage(null); }
    finally { setLibraryLoading(false); }
  }

  async function importLibrary() {
    setLibraryLoading(true);
    setError(null);
    setImportMessage(null);
    try {
      const report = await invoke<{ items: LibraryItem[]; imported: number; errors: string[] }>("import_library_files");
      setLibraryItems(report.items);
      if (report.imported) setImportMessage(`已导入 ${report.imported} 本书，文件已保存在应用书库。`);
      if (report.errors.length) setError(report.errors.join("；"));
    } catch (reason) { setError(String(reason)); }
    finally { setLibraryLoading(false); }
  }

  async function chooseFolder() {
    try {
      const path = await invoke<string | null>("choose_library_folder");
      if (path) setSettings((current) => ({ ...current, libraryDirectory: path }));
    } catch (reason) { setError(String(reason)); }
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

  async function openLibraryItem(item: LibraryItem) {
    if (item.format === "pdf") {
      setError(null);
      setPdfItem(item);
      return;
    }
    if (item.format !== "epub" && item.format !== "txt") {
      setError(`内置阅读器暂时支持 EPUB / TXT / PDF，${item.format.toUpperCase()} 会在后续接入。`);
      return;
    }
    setReaderLoading(true);
    setError(null);
    try {
      const [book, progress] = await Promise.all([
        invoke<ReaderBookMetadata>("open_local_book_metadata", { path: item.path }),
        invoke<ReadingProgress | null>("reading_progress", { libraryId: item.id }),
      ]);
      let chapterIndex = 0;
      const savedPosition = parseReaderPosition(progress?.locator);
      if (savedPosition) {
        const located = book.chapters.findIndex((chapter) => chapter.id === savedPosition.chapterId);
        if (located >= 0) chapterIndex = located;
      } else if (progress && book.chapters.length > 1) {
        chapterIndex = Math.min(
          book.chapters.length - 1,
          Math.max(0, Math.floor(progress.fraction * book.chapters.length)),
        );
      }
      setReaderItem(item);
      setReaderBook({ ...book, title: item.title });
      setReaderInitialScroll(savedPosition?.scroll ?? 0);
      setReaderChapterIndex(chapterIndex);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setReaderLoading(false);
    }
  }

  function changeReaderChapter(nextIndex: number) {
    if (!readerBook) return;
    const bounded = Math.min(readerBook.chapters.length - 1, Math.max(0, nextIndex));
    setReaderChapterIndex(bounded);
  }

  function closeReader() {
    setReaderItem(null);
    setReaderBook(null);
    setReaderChapterIndex(0);
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
            <span className="eyebrow">LUMINASHELF · 星书</span>
            <h2>把想读的书，<br />留在自己的书架。</h2>
            <p>搜索书目，导入手机里的电子书，随时接着上次的位置阅读。未完成的下载会保留，重新打开后可以继续。</p>
            <button className="primary-button hero-action" onClick={() => setPage("search")}>开始搜索</button>
          </div>
          <div className="orb"><span>Z</span></div>
        </section>

        <section className="cards">
          <article className="card glass"><span>CORE</span><strong>{status?.networkStack ?? "Rust + Tokio"}</strong><small>{status?.rustCore ? "网络服务已连接" : "正在连接核心服务"}</small></article>
          <article className="card glass"><span>LIBRARY</span><strong>{libraryItems.length}</strong><small>已持久化本地书籍</small></article>
          <article className="card glass"><span>DOWNLOADS</span><strong>{completedCount}/{downloadTasks.length}</strong><small>已完成 / 持久化任务</small></article>
        </section>

        <section className="workspace glass">
          <div className="workspace-title"><div><span className="eyebrow">PROVIDERS</span><h3>数据源状态</h3></div><span className="status-dot">{status?.rustCore ? "● READY" : "CONNECTING"}</span></div>
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
            <input aria-label="搜索电子书" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="书名、作者、关键词…" autoFocus />
            <select aria-label="数据源" value={selectedProvider} onChange={(event) => changeProvider(event.target.value)}>
              {providers.filter((provider) => provider.capabilities.searchable).map((provider) => (
                <option value={provider.id} key={provider.id}>{provider.name}</option>
              ))}
            </select>
            <button className="primary-button" disabled={searching || !query.trim() || providerNeedsLogin}>{searching ? "搜索中…" : "搜索"}</button>
          </form>
          <div className="search-hint">每页 {settings.searchPageSize} 项 · 支持书名、作者和关键词</div>
          {recentSearches.length > 0 ? <div className="recent-searches" aria-label="最近搜索">
            {recentSearches.map((text) => <button className="ghost-button small" key={text} onClick={() => setQuery(text)}>{text}</button>)}
            <button className="ghost-button small" onClick={() => { setRecentSearches([]); localStorage.removeItem("luminashelf.recentSearches"); }}>清空历史</button>
          </div> : null}
        </section>

        {providerNeedsLogin ? (
          <section className="auth-gate glass">
            <span className="auth-gate-mark">○</span>
            <div><span className="eyebrow">ACCOUNT</span><h3>先连接 Z-Library 账户</h3><p>登录后可以搜索和下载书籍，也可以切换到无需登录的公开书源。</p></div>
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
          <section className="empty-state glass"><span>⌕</span><h3>从一本书开始</h3><p>输入书名或作者，找到下一本想读的书。</p></section>
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
                    {["queued", "connecting", "downloading"].includes(task.state) ? <button className="ghost-button" disabled={taskActions.has(task.key)} onClick={() => void controlDownload(task, "pause_download")}>暂停</button> : null}
                    {["queued", "connecting", "downloading"].includes(task.state) ? <button className="ghost-button danger" disabled={taskActions.has(task.key)} onClick={() => void controlDownload(task, "cancel_download")}>取消</button> : null}
                    {canResume ? <button className="primary-button small" disabled={taskActions.has(task.key)} onClick={() => void retryDownload(task)}>继续</button> : null}
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
            <button className="primary-button" disabled={libraryLoading} onClick={() => void importLibrary()}>{libraryLoading ? "处理中…" : "导入电子书"}</button>
            <input aria-label="筛选书库" value={libraryQuery} onChange={(event) => setLibraryQuery(event.target.value)} placeholder="筛选书名、作者或格式…" />
          </div>
          {status?.platform !== "android" ? <div className="library-toolbar library-scan">
            <input aria-label="书库目录" value={settings.libraryDirectory} onChange={(event) => setSettings((current) => ({ ...current, libraryDirectory: event.target.value }))} placeholder="也可选择已有书库目录" />
            <button className="ghost-button" disabled={libraryLoading} onClick={() => void chooseFolder()}>选择目录</button>
            <button className="ghost-button" disabled={libraryLoading || !settings.libraryDirectory.trim()} onClick={() => void scanLibrary()}>扫描书库</button>
          </div> : null}
          <div className="library-toolbar library-scan">
            <select aria-label="书库排序" value={librarySort} onChange={(event) => setLibrarySort(event.target.value)}>
              <option value="recent">最近添加</option><option value="title">按书名</option><option value="author">按作者</option>
            </select>
            <button className="ghost-button" disabled={libraryLoading} onClick={() => void backupLibrary(false)}>导出备份</button>
            <button className="ghost-button" disabled={libraryLoading} onClick={() => void backupLibrary(true)}>恢复备份</button>
          </div>
          <div className="search-hint">导入会保留原文件 · 内置阅读器支持 EPUB / TXT / PDF · 书库和阅读位置自动保存</div>
          {importMessage ? <div className="search-hint" role="status">{importMessage}</div> : null}
        </section>
        {libraryItems.length > 0 ? (
          <section className="library-grid">
            {libraryItems.filter((item) => `${item.title} ${item.authors.join(" ")} ${item.format}`.toLowerCase().includes(libraryQuery.trim().toLowerCase())).sort((a,b) => librarySort === "title" ? a.title.localeCompare(b.title) : librarySort === "author" ? a.authors.join("").localeCompare(b.authors.join("")) : 0).map((item) => {
              const readable = item.format === "epub" || item.format === "txt" || item.format === "pdf";
              return (
                <article className="library-card glass" key={`${item.id}:${item.path}`}>
                  <div className="library-icon">{item.format.toUpperCase()}</div>
                  <div className="library-card-copy">
                    <h4>{item.title}</h4>
                    <p>{item.authors.join(" · ") || "本地文件"}</p>
                    <small>{formatBytes(item.sizeBytes)} · {item.path}</small>
                    <div className="library-card-actions">
                      <button className="ghost-button small" aria-label={`管理 ${item.title}`} onClick={() => setManageItem(item)}>管理</button>
                      <button className="primary-button small" disabled={!readable || readerLoading} onClick={() => void openLibraryItem(item)}>
                        {readerLoading ? "打开中…" : readable ? "阅读 / 继续阅读" : "暂不支持阅读"}
                      </button>
                    </div>
                  </div>
                </article>
              );
            })}
          </section>
        ) : (
          <section className="empty-state glass"><span>▤</span><h3>本地书架还是空的</h3><p>点击「导入电子书」，或从搜索页下载一本书。重启后仍可继续阅读。</p></section>
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
            {status?.platform === "android" ? (
              <p className="search-hint">下载和导入的书籍保存在应用书库，可离线阅读。在「书库」点「导入电子书」选择手机文件。卸载应用会移除应用内保存的书籍与进度。</p>
            ) : (<>
              <label className="stacked"><span>下载目录<small>留空使用系统 Downloads/LuminaShelf</small></span><input value={settings.downloadDirectory} onChange={(event) => setSettings((current) => ({ ...current, downloadDirectory: event.target.value }))} placeholder="默认系统下载目录" /></label>
              <label className="stacked"><span>书库目录<small>用于本地扫描</small></span><input value={settings.libraryDirectory} onChange={(event) => setSettings((current) => ({ ...current, libraryDirectory: event.target.value }))} placeholder="选择或填写本地书库目录" /></label>
              <label className="toggle-row"><span>递归扫描<small>同时扫描所有子目录</small></span><button className={`toggle ${settings.recursiveLibraryScan ? "on" : ""}`} onClick={() => setSettings((current) => ({ ...current, recursiveLibraryScan: !current.recursiveLibraryScan }))}><i /></button></label>
            </>)}
          </article>

          <NetworkSettings networkStack={status?.networkStack ?? "Tokio · Reqwest · Hickory"} />

          <article className="setting-card glass">
            <span className="eyebrow">CORE</span><h3>关于 LuminaShelf</h3>
            <div className="readout"><span>版本</span><strong>v{status?.version ?? "…"}</strong></div>
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
            <div className="brand-mark">L</div>
            <div className="brand-copy"><strong>LuminaShelf</strong><span>星书 · 随时继续阅读</span></div>
          </button>
          <div className="utility-buttons">
            <button className={page === "account" ? "active" : ""} onClick={() => setPage("account")} aria-label="账户" title="账户"><span className={zlibraryStatus?.signedIn ? "utility-online" : ""}>○</span></button>
            <button className={page === "settings" ? "active" : ""} onClick={() => setPage("settings")} aria-label="设置" title="设置">⚙</button>
          </div>
        </div>
        <nav className="primary-nav">
          {nav.map((item) => (
            <button key={item.id} aria-label={item.label} className={page === item.id ? "active" : ""} onClick={() => setPage(item.id)}>
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
          {page === "settings" || page === "account" ? <button className="ghost-button header-back" onClick={() => setPage("home")}>← 返回总览</button> : <div className="version-chip">v{status?.version ?? "…"}</div>}
        </header>

        {error ? <section className="notice error" role="alert"><span>Core bridge</span><strong>{error}</strong><button onClick={() => setError(null)}>×</button></section> : null}

        {page === "home" ? renderHome() : null}
        {page === "search" ? renderSearch() : null}
        {page === "downloads" ? renderDownloads() : null}
        {page === "library" ? renderLibrary() : null}
        {page === "account" ? <ZLibraryAccount initialStatus={zlibraryStatus} onStatusChange={handleZlibraryStatus} /> : null}
        {page === "settings" ? renderSettings() : null}
      </main>

      <nav className="mobile-nav glass">
        {nav.map((item) => <button key={item.id} aria-label={item.label} className={page === item.id ? "active" : ""} onClick={() => setPage(item.id)}><span>{item.glyph}</span><small>{item.label}</small></button>)}
      </nav>

      {manageItem ? <LibraryActions item={manageItem} onClose={() => setManageItem(null)} onChanged={(item) => { setLibraryItems((current) => item ? current.map((value) => value.id === item.id ? item : value) : current.filter((value) => value.id !== manageItem.id)); setManageItem(null); }} /> : null}

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
            <label className="download-format-choice">下载格式
              <select aria-label="下载格式" value={downloadFormat} onChange={(event) => setDownloadFormat(event.target.value as BookFormat)}>
                {Array.from(new Set([selectedBook.format, ...selectedBook.availableFormats].filter((format): format is BookFormat => Boolean(format) && format !== "other"))).map((format) => <option key={format} value={format}>{format.toUpperCase()}</option>)}
              </select>
            </label>
            <button className="primary-button wide" disabled={!downloadFormat || ["queued", "connecting", "downloading", "verifying"].includes(downloads[taskKey(selectedProvider, selectedBook.id, downloadFormat || "other")]?.state ?? "")}
              onClick={() => downloadFormat && void startDownload(selectedBook, downloadFormat)}>加入下载队列</button>
          </aside>
        </div>
      ) : null}

      {readerItem && readerBook ? (
        <ReaderView
          libraryId={readerItem.id}
          initialScroll={readerInitialScroll}
          book={readerBook}
          bookPath={readerItem.path}
          fallbackTitle={readerItem.title}
          format={readerItem.format}
          chapterIndex={readerChapterIndex}
          progressFraction={readerFraction}
          onChapterChange={changeReaderChapter}
          onClose={closeReader}
        />
      ) : null}

      {pdfItem ? (
        <PdfReader
          libraryId={pdfItem.id}
          path={pdfItem.path}
          title={pdfItem.title}
          onClose={() => setPdfItem(null)}
        />
      ) : null}
    </div>
  );
}
