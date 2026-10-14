import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as Dialog from "@radix-ui/react-dialog";
import { BookOpen, Library, Search, Download, Settings, UserRound, Plus, ArrowLeft, ChevronRight, X, RefreshCw, Check, Trash2, FileText } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import NetworkSettings from "./NetworkPreferences";
import PdfReader from "./PdfReader";
import ReaderView from "./ReaderView";
import ZLibraryAccount, { type ZLibraryAccountStatus } from "./AccountPage";
import BookCover from "./BookCover";
import type { BookSummary, BookDetails, BookFormat, LibraryItem, ReadingProgress, ReaderBookMetadata } from "./types";

type Page = "library" | "search" | "downloads" | "settings" | "account";
type Provider = { id: string; name: string; capabilities: { authenticated: boolean; searchable: boolean; downloadable: boolean } };
type DownloadTask = { key: string; providerId: string; bookId: string; title: string; format: BookFormat; state: "queued" | "downloading" | "completed" | "failed"; downloadedBytes: number; totalBytes?: number | null; error?: string };
type SettingsData = { defaultProvider: string; searchPageSize: number };
type SearchResult = { items: BookSummary[]; page: number; hasNext: boolean };
const navigation = [
  { id: "library", label: "书架", icon: Library },
  { id: "search", label: "找书", icon: Search },
  { id: "downloads", label: "下载", icon: Download },
  { id: "settings", label: "设置", icon: Settings },
] as const;
const pageTitle: Record<Page, string> = { library: "我的书架", search: "找一本书", downloads: "下载", settings: "设置", account: "书源账户" };
const defaults: SettingsData = { defaultProvider: "", searchPageSize: 24 };
function loadSettings(): SettingsData {
  try { return { ...defaults, ...JSON.parse(localStorage.getItem("luminashelf.settings") || "{}") }; } catch { return defaults; }
}
function formatBytes(value?: number | null) {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB"]; let size = value; let unit = 0;
  while (size >= 1024 && unit < 3) { size /= 1024; unit++; }
  return `${size.toFixed(unit && size < 10 ? 1 : 0)} ${units[unit]}`;
}
function readableFormat(book: BookSummary): BookFormat | null {
  return (["epub", "pdf", "txt"] as BookFormat[]).find((format) => book.format === format || book.availableFormats.includes(format)) || null;
}
function messageOf(reason: unknown) { return reason instanceof Error ? reason.message : String(reason); }

export default function App() {
  const [page, setPage] = useState<Page>("library");
  const [providers, setProviders] = useState<Provider[]>([]);
  const [account, setAccount] = useState<ZLibraryAccountStatus | null>(null);
  const [version, setVersion] = useState("0.6.0");
  const [settings, setSettings] = useState<SettingsData>(loadSettings);
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);
  const [library, setLibrary] = useState<LibraryItem[]>([]);
  const [libraryLoading, setLibraryLoading] = useState(true);
  const [importing, setImporting] = useState(false);
  const [filter, setFilter] = useState("");
  const [query, setQuery] = useState("");
  const [provider, setProvider] = useState("");
  const [results, setResults] = useState<SearchResult | null>(null);
  const [searching, setSearching] = useState(false);
  const [selectedBook, setSelectedBook] = useState<BookSummary | null>(null);
  const [details, setDetails] = useState<BookDetails | null>(null);
  const [detailsLoading, setDetailsLoading] = useState(false);
  const [selectedFormat, setSelectedFormat] = useState<BookFormat>("epub");
  const [downloads, setDownloads] = useState<Record<string, DownloadTask>>({});
  const [reader, setReader] = useState<{ item: LibraryItem; book: ReaderBookMetadata; chapter: number; offset: number } | null>(null);
  const [readerLoading, setReaderLoading] = useState<string | null>(null);
  const [pdf, setPdf] = useState<LibraryItem | null>(null);
  const [removing, setRemoving] = useState<LibraryItem | null>(null);
  const searchId = useRef(0);
  const detailsId = useRef(0);
  const bookCache = useRef<Record<string, string>>({});
  const progressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { localStorage.setItem("luminashelf.settings", JSON.stringify(settings)); }, [settings]);
  useEffect(() => {
    let active = true;
    void Promise.allSettled([
      invoke<{ version: string }>("core_status").then((value) => { if (active) setVersion(value.version); }),
      invoke<Provider[]>("provider_descriptors").then((items) => {
        if (!active) return; setProviders(items);
        setProvider(items.find((item) => item.id === settings.defaultProvider)?.id || items.find((item) => !item.capabilities.authenticated)?.id || items[0]?.id || "");
      }),
      invoke<ZLibraryAccountStatus>("zlibrary_status").then((value) => { if (active) setAccount(value); }),
      invoke<LibraryItem[]>("library_items").then((items) => { if (active) setLibrary(items); }).catch((reason) => { if (active) setNotice({ text: messageOf(reason), error: true }); }),
    ]).finally(() => { if (active) setLibraryLoading(false); });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    let disposed = false; let stop: (() => void) | undefined;
    void listen<{ providerId: string; bookId: string; downloadedBytes: number; totalBytes?: number }>("download-progress", ({ payload }) => {
      const key = `${payload.providerId}:${payload.bookId}`;
      setDownloads((current) => current[key] && current[key].state !== "completed" ? { ...current, [key]: { ...current[key], ...payload, state: "downloading" } } : current);
    }).then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; }).catch(() => {});
    return () => { disposed = true; stop?.(); };
  }, []);
  useEffect(() => {
    const back = () => {
      if (removing) { setRemoving(null); return true; }
      if (selectedBook) { setSelectedBook(null); return true; }
      if (reader) { setReader(null); return true; }
      if (pdf) { setPdf(null); return true; }
      if (page !== "library") { setPage(page === "account" ? "settings" : "library"); return true; }
      return false;
    };
    (window as Window & { luminaBack?: () => boolean }).luminaBack = back;
    return () => { delete (window as Window & { luminaBack?: () => boolean }).luminaBack; };
  }, [page, reader, pdf, selectedBook, removing]);
  const needsLogin = provider === "zlibrary" && !account?.signedIn;
  const visibleLibrary = useMemo(() => library.filter((item) => `${item.title} ${item.authors.join(" ")}`.toLowerCase().includes(filter.toLowerCase())), [library, filter]);
  const pending = Object.values(downloads).filter((task) => task.state === "queued" || task.state === "downloading").length;
  function switchProvider(value: string) { searchId.current++; detailsId.current++; setProvider(value); setResults(null); setSelectedBook(null); setSearching(false); }
  async function refreshLibrary() {
    setLibraryLoading(true);
    try { setLibrary(await invoke<LibraryItem[]>("library_items")); } catch (reason) { setNotice({ text: messageOf(reason), error: true }); }
    finally { setLibraryLoading(false); }
  }
  async function importBooks() {
    setImporting(true); setNotice(null);
    try { const items = await invoke<LibraryItem[]>("import_local_books"); if (items.length) { await refreshLibrary(); setNotice({ text: `已将 ${items.length} 本书加入书架`, error: false }); } }
    catch (reason) { setNotice({ text: `导入失败：${messageOf(reason)}`, error: true }); }
    finally { setImporting(false); }
  }
  async function search(targetPage = 1) {
    if (!query.trim() || !provider || needsLogin) return;
    const id = ++searchId.current; setSearching(true); setNotice(null);
    try { const found = await invoke<SearchResult>("search_books", { providerId: provider, text: query.trim(), page: targetPage, pageSize: settings.searchPageSize, formats: ["epub", "pdf", "txt"] }); if (id === searchId.current) setResults(found); }
    catch (reason) { if (id === searchId.current) setNotice({ text: `搜索失败：${messageOf(reason)}`, error: true }); }
    finally { if (id === searchId.current) setSearching(false); }
  }
  function submitSearch(event: FormEvent) { event.preventDefault(); void search(); }
  async function showDetails(book: BookSummary) {
    const id = ++detailsId.current; setSelectedBook(book); setDetails(null); setDetailsLoading(true); setSelectedFormat(readableFormat(book) || "epub");
    try { const found = await invoke<BookDetails>("book_details", { providerId: provider, bookId: book.id }); if (id === detailsId.current) setDetails(found); }
    catch (reason) { if (id === detailsId.current) setNotice({ text: messageOf(reason), error: true }); }
    finally { if (id === detailsId.current) setDetailsLoading(false); }
  }
  async function downloadBook(book: BookSummary, format = readableFormat(book)) {
    if (!format || needsLogin) return;
    const key = `${provider}:${book.id}`; const bookProvider = provider;
    bookCache.current[book.title] = book.coverUrl || "";
    setDownloads((current) => ({ ...current, [key]: { key, providerId: bookProvider, bookId: book.id, title: book.title, format, state: "queued", downloadedBytes: 0 } }));
    setSelectedBook(null); setNotice({ text: `“${book.title}”正在下载，完成后会加入书架`, error: false });
    try {
      const receipt = await invoke<{ item: LibraryItem }>("download_book", { providerId: bookProvider, bookId: book.id, title: book.title, format, downloadDir: null });
      setDownloads((current) => ({ ...current, [key]: { ...current[key], state: "completed", downloadedBytes: receipt.item.sizeBytes, totalBytes: receipt.item.sizeBytes } }));
      setLibrary((current) => [receipt.item, ...current.filter((item) => item.id !== receipt.item.id)]);
      setNotice({ text: `“${book.title}”已加入书架`, error: false });
    } catch (reason) { setDownloads((current) => ({ ...current, [key]: { ...current[key], state: "failed", error: messageOf(reason) } })); setNotice({ text: `下载失败：${messageOf(reason)}`, error: true }); }
  }
  async function openBook(item: LibraryItem) {
    if (item.format === "pdf") { setPdf(item); return; }
    setReaderLoading(item.id); setNotice(null);
    try {
      const [book, progress] = await Promise.all([invoke<ReaderBookMetadata>("open_local_book_metadata", { path: item.path }), invoke<ReadingProgress | null>("reading_progress", { libraryId: item.id })]);
      let chapterId = progress?.locator; let offset = 0;
      if (chapterId?.startsWith("{")) {
        try { const locator = JSON.parse(chapterId); chapterId = locator.chapter; offset = Math.min(1, Math.max(0, Number(locator.offset) || 0)); } catch {}
      }
      const found = book.chapters.findIndex((chapter) => chapter.id === chapterId); setReader({ item, book, chapter: Math.max(0, found), offset });
    } catch (reason) { setNotice({ text: `打开失败：${messageOf(reason)}`, error: true }); }
    finally { setReaderLoading(null); }
  }
  function changeChapter(chapter: number) {
    if (progressTimer.current) { clearTimeout(progressTimer.current); progressTimer.current = null; }
    if (!reader) return; setReader({ ...reader, chapter, offset: 0 });
    void invoke("save_reading_progress", { libraryId: reader.item.id, locator: JSON.stringify({ chapter: reader.book.chapters[chapter]?.id, offset: 0 }), fraction: chapter / reader.book.chapters.length }).catch((reason) => setNotice({ text: messageOf(reason), error: true }));
  }
  function saveOffset(offset: number) {
    if (!reader) return; setReader({ ...reader, offset });
    if (progressTimer.current) clearTimeout(progressTimer.current);
    progressTimer.current = setTimeout(() => {
      void invoke("save_reading_progress", { libraryId: reader.item.id, locator: JSON.stringify({ chapter: reader.book.chapters[reader.chapter]?.id, offset }), fraction: (reader.chapter + offset) / reader.book.chapters.length }).catch((reason) => setNotice({ text: messageOf(reason), error: true }));
    }, 400);
  }
  async function removeBook() {
    if (!removing) return;
    try { await invoke("remove_library_item", { libraryId: removing.id }); setLibrary((current) => current.filter((item) => item.id !== removing.id)); setRemoving(null); }
    catch (reason) { setNotice({ text: messageOf(reason), error: true }); }
  }
  return <div className="app-shell">
    <aside className="sidebar">
      <button className="brand" onClick={() => setPage("library")} aria-label="星书书架"><BookOpen size={28} strokeWidth={1.6} /><span>星书<small>LuminaShelf</small></span></button>
      <nav aria-label="主导航">{navigation.map(({ id, label, icon: Icon }) => <button key={id} onClick={() => setPage(id)} className={page === id || page === "account" && id === "settings" ? "nav-active" : ""} aria-current={page === id ? "page" : undefined}><Icon size={22} /><span>{label}</span>{id === "downloads" && pending > 0 ? <span className="count">{pending}</span> : null}</button>)}</nav>
      <button className="account-entry" onClick={() => setPage("account")}><UserRound size={22} /><span>书源账户<small>{account?.signedIn ? "Z-Library 已登录" : "连接你的 Z-Library"}</small></span><ChevronRight size={18} /></button>
    </aside>
    <main className="main-content">
      <header className="page-header"><div>{page === "account" ? <button className="text-button back-button" onClick={() => setPage("settings")}><ArrowLeft size={20} />设置</button> : <span className="mobile-brand">星书</span>}<h1>{pageTitle[page]}</h1></div>{page === "library" ? <button className="primary-button" onClick={() => void importBooks()} disabled={importing}><Plus size={20} />{importing ? "正在导入" : "导入书籍"}</button> : null}</header>
      {notice ? <div className={`notice ${notice.error ? "error" : "success"}`} role={notice.error ? "alert" : "status"}><span>{notice.text}</span><button aria-label="关闭提示" onClick={() => setNotice(null)}><X size={20} /></button>{notice.error && /搜索|下载|DNS|连接/.test(notice.text) ? <button className="text-button" onClick={() => setPage("settings")}>检查网络</button> : null}</div> : null}
      {page === "library" ? <>
        {library.length > 0 ? <div className="library-tools"><label className="search-input"><Search size={20} /><input aria-label="搜索书架" placeholder="在书架中找书" value={filter} onChange={(event) => setFilter(event.target.value)} /></label><button className="icon-button" aria-label="刷新书架" onClick={() => void refreshLibrary()}><RefreshCw size={20} /></button><span>{library.length} 本书</span></div> : null}
        {libraryLoading ? <div className="loading-state" role="status">正在整理书架…</div> : visibleLibrary.length ? <div className="shelf-grid">{visibleLibrary.map((item) => <article className="shelf-book" key={item.id}><button className="cover-button" onClick={() => void openBook(item)} aria-label={`阅读 ${item.title}`} disabled={readerLoading === item.id}><BookCover title={item.title} authors={item.authors} url={bookCache.current[item.title]} format={item.format} /></button><h3>{item.title}</h3><p>{item.authors.join(" · ") || item.format.toUpperCase()}</p><div className="shelf-actions"><button className="text-button" onClick={() => void openBook(item)} disabled={readerLoading === item.id}>{readerLoading === item.id ? "打开中" : "阅读"}<ChevronRight size={16} /></button><button className="icon-button muted" aria-label={`从书架移除 ${item.title}`} onClick={() => setRemoving(item)}><Trash2 size={18} /></button></div></article>)}</div> : <section className="empty-state"><BookOpen size={36} strokeWidth={1.4} /><h2>{filter ? "没有找到这本书" : "把第一本书放进书架"}</h2><p>{filter ? "试试书名或作者的其他关键词。" : "导入 EPUB、PDF 或 TXT，随时接着读。"}</p>{!filter ? <div><button className="primary-button" onClick={() => void importBooks()} disabled={importing}><Plus size={18} />导入书籍</button><button className="secondary-button" onClick={() => setPage("search")}>去找书<ChevronRight size={18} /></button></div> : null}</section>}
      </> : null}
      {page === "search" ? <>
        <form className="book-search" onSubmit={submitSearch}><label className="search-input"><Search size={20} /><input aria-label="书名或作者" placeholder="书名、作者或关键词" value={query} onChange={(event) => setQuery(event.target.value)} /></label><button className="primary-button" disabled={searching || !query.trim() || needsLogin}>{searching ? "正在搜索" : "搜索"}</button></form>
        <div className="source-picker" role="group" aria-label="选择书源">{providers.filter((item) => item.capabilities.searchable).map((item) => <button key={item.id} className={provider === item.id ? "selected" : ""} onClick={() => switchProvider(item.id)}>{item.id === "gutendex" ? "古登堡公版书" : "Z-Library"}{provider === item.id ? <Check size={16} /> : null}</button>)}</div>
        {needsLogin ? <section className="login-prompt"><UserRound size={28} /><h2>登录后搜索 Z-Library</h2><p>使用你的账户检索和下载书籍。</p><button className="primary-button" onClick={() => setPage("account")}>登录账户</button></section> : searching ? <div className="loading-state" role="status">正在找书…</div> : results ? <><div className="results-heading"><span>{results.items.length ? `第 ${results.page} 页 · ${results.items.length} 本书` : "没有匹配的书籍"}</span><div><button className="secondary-button" disabled={results.page <= 1 || searching} onClick={() => void search(results.page - 1)}>上一页</button><button className="secondary-button" disabled={!results.hasNext || searching} onClick={() => void search(results.page + 1)}>下一页</button></div></div><div className="shelf-grid search-results">{results.items.map((book) => {
          const task = downloads[`${provider}:${book.id}`];
          return <article className="shelf-book" key={book.id}><button className="cover-button" onClick={() => void showDetails(book)} aria-label={`查看 ${book.title}`}><BookCover title={book.title} authors={book.authors} url={book.coverUrl} format={readableFormat(book) || ""} /></button><h3>{book.title}</h3><p>{book.authors.join(" · ") || "作者未提供"}</p><button className="secondary-button book-add" disabled={!readableFormat(book) || task?.state === "completed" || task?.state === "downloading" || task?.state === "queued"} onClick={() => void downloadBook(book)}>{task?.state === "completed" ? "已加入书架" : task?.state === "downloading" || task?.state === "queued" ? "下载中" : "加入书架"}{task?.state === "completed" ? <Check size={17} /> : <Plus size={17} />}</button></article>;
        })}</div></> : <section className="search-start"><Search size={28} strokeWidth={1.5} /><h2>下一本想读什么？</h2><p>{provider === "gutendex" ? "古登堡收录公版书籍，可尝试英文书名或作者。" : "输入书名或作者开始搜索。"}</p></section>}
      </> : null}
      {page === "downloads" ? <section className="download-list">{Object.values(downloads).length ? Object.values(downloads).map((task) => { const percentage = task.totalBytes ? Math.min(100, Math.round(task.downloadedBytes / task.totalBytes * 100)) : task.state === "completed" ? 100 : 0; return <article className="download-item" key={task.key}><FileText size={26} strokeWidth={1.6} /><div className="download-copy"><h3>{task.title}</h3><p>{task.format.toUpperCase()} · {formatBytes(task.downloadedBytes)}{task.totalBytes ? ` / ${formatBytes(task.totalBytes)}` : ""}</p><div className="progress-track" role="progressbar" aria-label={task.title} aria-valuenow={percentage} aria-valuemin={0} aria-valuemax={100}><span style={{ width: `${percentage}%` }} /></div>{task.error ? <p className="error-text">{task.error}</p> : null}</div><span className={`download-state ${task.state}`}>{task.state === "completed" ? "已完成" : task.state === "failed" ? "下载失败" : task.state === "queued" ? "准备中" : `${percentage}%`}</span></article>; }) : <div className="empty-state"><Download size={32} strokeWidth={1.5} /><h2>还没有下载任务</h2><p>找到想读的书，加入书架后可在这里查看进度。</p><button className="primary-button" onClick={() => setPage("search")}>去找书</button></div>}</section> : null}
      {page === "settings" ? <div className="settings-content"><section className="settings-section"><h2>账户与书源</h2><button className="settings-row" onClick={() => setPage("account")}><UserRound size={23} /><span><strong>Z-Library 账户</strong><small>{account?.signedIn ? "已登录，点击管理账户" : "登录以搜索和下载书籍"}</small></span><ChevronRight size={20} /></button><label className="settings-field"><span>默认书源</span><select value={settings.defaultProvider} onChange={(event) => setSettings({ ...settings, defaultProvider: event.target.value })}><option value="">自动选择</option>{providers.map((item) => <option value={item.id} key={item.id}>{item.id === "gutendex" ? "古登堡公版书" : "Z-Library"}</option>)}</select></label><label className="settings-field"><span>每页搜索结果</span><select value={settings.searchPageSize} onChange={(event) => setSettings({ ...settings, searchPageSize: Number(event.target.value) })}>{[12, 24, 36, 50].map((count) => <option key={count} value={count}>{count} 本</option>)}</select></label></section><NetworkSettings /><section className="settings-section about-section"><h2>关于星书</h2><p>LuminaShelf {version}</p><p>一个用来找书、收书和阅读的本地书架。</p><a href="https://github.com/Junyxor/LuminaShelf" target="_blank" rel="noreferrer">项目与开源信息<ChevronRight size={16} /></a></section></div> : null}
      {page === "account" ? <ZLibraryAccount initialStatus={account} onStatusChange={setAccount} /> : null}
    </main>
    <nav className="mobile-nav" aria-label="手机主导航">{navigation.map(({ id, label, icon: Icon }) => <button key={id} aria-current={page === id ? "page" : undefined} className={page === id || page === "account" && id === "settings" ? "nav-active" : ""} onClick={() => setPage(id)}><Icon size={23} strokeWidth={1.7} /><span>{label}</span></button>)}</nav>
    <Dialog.Root open={Boolean(selectedBook)} onOpenChange={(open) => { if (!open) { detailsId.current++; setSelectedBook(null); } }}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="book-dialog" aria-describedby="book-description"><Dialog.Close className="icon-button dialog-close" aria-label="关闭书籍详情"><X size={22} /></Dialog.Close>{selectedBook ? <><div className="detail-cover"><BookCover title={selectedBook.title} authors={selectedBook.authors} url={selectedBook.coverUrl} /></div><Dialog.Title>{selectedBook.title}</Dialog.Title><p className="detail-author">{selectedBook.authors.join(" · ") || "作者未提供"}</p><Dialog.Description id="book-description">{detailsLoading ? "正在读取简介…" : details?.description || "暂无简介"}</Dialog.Description><label className="settings-field"><span>阅读格式</span><select value={selectedFormat} onChange={(event) => setSelectedFormat(event.target.value as BookFormat)}>{(["epub", "pdf", "txt"] as BookFormat[]).filter((format) => selectedBook.availableFormats.includes(format) || selectedBook.format === format).map((format) => <option key={format} value={format}>{format.toUpperCase()}</option>)}</select></label><button className="primary-button full-width" disabled={!readableFormat(selectedBook)} onClick={() => void downloadBook(selectedBook, selectedFormat)}><Plus size={20} />加入书架</button></> : <Dialog.Title>书籍详情</Dialog.Title>}</Dialog.Content></Dialog.Portal></Dialog.Root>
    <Dialog.Root open={Boolean(removing)} onOpenChange={(open) => { if (!open) setRemoving(null); }}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="confirm-dialog"><Dialog.Title>从书架移除？</Dialog.Title><Dialog.Description>“{removing?.title}”将从书架列表中移除。</Dialog.Description><div className="dialog-actions"><Dialog.Close className="secondary-button">取消</Dialog.Close><button className="danger-button" onClick={() => void removeBook()}>移除</button></div></Dialog.Content></Dialog.Portal></Dialog.Root>
    {reader ? <ReaderView book={reader.book} bookPath={reader.item.path} fallbackTitle={reader.item.title} format={reader.item.format} chapterIndex={reader.chapter} progressFraction={(reader.chapter + reader.offset) / reader.book.chapters.length} initialOffset={reader.offset} onProgressChange={saveOffset} onChapterChange={changeChapter} onClose={() => setReader(null)} /> : null}
    {pdf ? <PdfReader libraryId={pdf.id} path={pdf.path} title={pdf.title} onClose={() => setPdf(null)} /> : null}
  </div>;
}
