import { invoke } from "@tauri-apps/api/core";
import * as Dialog from "@radix-ui/react-dialog";
import { Library, Search, Download, Settings, UserRound, Plus, ArrowLeft, ChevronRight, X, RefreshCw, Check, Heart, Ellipsis, Wifi, Globe, Megaphone } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import NetworkSettings from "./DnsSettings";
import PdfReader from "./PdfReader";
import ReaderView from "./ReaderView";
import ZLibraryAccount, { type ZLibraryAccountStatus } from "./AccountPage";
import BookCover from "./BookCover";
import BookCard, { FavoriteButton, SkeletonGrid } from "./BookCard";
import MirrorPreferences from "./MirrorPreferences";
import BookSourceHome from "./BookSourceHome";
import BookDetailsDialog from "./BookDetailsDialog";
import DownloadedBookActions from "./DownloadedBookActions";
import AnnouncementsPage from "./AnnouncementsPage";
import DownloadQueue, { activeDownload, useDownloadQueue } from "./DownloadQueue";
import { openExternalFile } from "./external-files";
import type { BookSummary, BookFormat, LibraryItem, ReadingProgress, ReaderBookMetadata, FavoriteBook } from "./types";

type Page = "library" | "search" | "downloads" | "settings" | "account" | "network" | "mirrors" | "announcements";
type Provider = { id: string; name: string; capabilities: { authenticated: boolean; searchable: boolean; downloadable: boolean } };
type SettingsData = { defaultProvider: string; searchPageSize: number };
type SearchResult = { items: BookSummary[]; page: number; hasNext: boolean };
const navigation = [
  { id: "library", label: "书架", icon: Library },
  { id: "search", label: "找书", icon: Search },
  { id: "downloads", label: "下载", icon: Download },
  { id: "settings", label: "设置", icon: Settings },
] as const;
const settingsPages: Page[] = ["account", "network", "mirrors", "announcements"];
const pageTitle: Record<Page, string> = { library: "我的书架", search: "找一本书", downloads: "下载", settings: "设置", account: "书源账户", network: "DNS 与网络", mirrors: "书源镜像", announcements: "公告与更新" };
const defaults: SettingsData = { defaultProvider: "", searchPageSize: 24 };
function loadSettings(): SettingsData {
  try { return { ...defaults, ...JSON.parse(localStorage.getItem("luminashelf.settings") || "{}") }; } catch { return defaults; }
}
function readableFormat(book: BookSummary): BookFormat | null {
  return (["epub", "pdf", "txt"] as BookFormat[]).find((format) => book.format === format || book.availableFormats.includes(format)) || null;
}
function messageOf(reason: unknown) { return reason instanceof Error ? reason.message : String(reason); }
function favoriteSummary(item: FavoriteBook): BookSummary { const format = item.format as BookFormat | null; return { id: item.bookId, title: item.title, authors: item.authors, coverUrl: item.coverUrl, format, availableFormats: format ? [format] : [] }; }
function providerName(id: string) { return id === "gutendex" ? "古登堡公版书" : "Z-Library"; }
function byline(authors: string[], fallback: string) { return authors.join(" · ") || fallback; }

export default function App() {
  const [page, setPage] = useState<Page>("library");
  const [providers, setProviders] = useState<Provider[]>([]);
  const [account, setAccount] = useState<ZLibraryAccountStatus | null>(null);
  const [version, setVersion] = useState("0.8.2");
  const [settings, setSettings] = useState<SettingsData>(loadSettings);
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);
  const [library, setLibrary] = useState<LibraryItem[]>([]);
  const [favorites, setFavorites] = useState<FavoriteBook[]>([]);
  const [shelfView, setShelfView] = useState<"downloaded" | "favorites">("downloaded");
  const [managed, setManaged] = useState<LibraryItem | null>(null);
  const [libraryLoading, setLibraryLoading] = useState(true);
  const [importing, setImporting] = useState(false);
  const [filter, setFilter] = useState("");
  const [query, setQuery] = useState("");
  const [provider, setProvider] = useState("");
  const [results, setResults] = useState<SearchResult | null>(null);
  const [searching, setSearching] = useState(false);
  const [selectedBook, setSelectedBook] = useState<BookSummary | null>(null);
  const [selectedProvider, setSelectedProvider] = useState("");
  const downloadQueue = useDownloadQueue((task) => { void refreshLibrary(); setNotice({ text: `“${task.title}”下载完成，已加入书架`, error: false }); });
  const [reader, setReader] = useState<{ item: LibraryItem; book: ReaderBookMetadata; chapter: number; offset: number } | null>(null);
  const [readerLoading, setReaderLoading] = useState<string | null>(null);
  const [pdf, setPdf] = useState<LibraryItem | null>(null);
  const [removing, setRemoving] = useState<LibraryItem | null>(null);
  const [removeFile, setRemoveFile] = useState(false);
  const searchId = useRef(0);
  const pressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const suppressOpen = useRef(false);
  const pressOrigin = useRef({ x: 0, y: 0 });
  const bookCache = useRef<Record<string, string>>({});
  const progressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { localStorage.setItem("luminashelf.settings", JSON.stringify(settings)); }, [settings]);
  useEffect(() => { window.scrollTo(0, 0); }, [page]);
  // Confirmations fade on their own; errors stay until read.
  useEffect(() => {
    if (!notice || notice.error) return;
    const timer = setTimeout(() => setNotice(null), 4500);
    return () => clearTimeout(timer);
  }, [notice]);
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
      invoke<FavoriteBook[]>("favorite_books").then((items) => { if (active) setFavorites(items); }).catch((reason) => { if (active) setNotice({ text: messageOf(reason), error: true }); }),
    ]).finally(() => { if (active) setLibraryLoading(false); });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    const back = () => {
      if (removing) { setRemoving(null); return true; }
      if (managed) { setManaged(null); return true; }
      if (selectedBook) { setSelectedBook(null); return true; }
      if (reader) { setReader(null); return true; }
      if (pdf) { setPdf(null); return true; }
      if (page === "downloads" && (window as Window & { luminaDownloadBack?: () => boolean }).luminaDownloadBack?.()) return true;
      if (page !== "library") { setPage(settingsPages.includes(page) ? "settings" : "library"); return true; }
      return false;
    };
    (window as Window & { luminaBack?: () => boolean }).luminaBack = back;
    return () => { delete (window as Window & { luminaBack?: () => boolean }).luminaBack; };
  }, [page, reader, pdf, selectedBook, removing, managed]);
  const signedOut = provider === "zlibrary" && !account?.signedIn;
  const visibleLibrary = useMemo(() => library.filter((item) => `${item.title} ${item.authors.join(" ")}`.toLowerCase().includes(filter.toLowerCase())), [library, filter]);
  const visibleFavorites = useMemo(() => favorites.filter((item) => `${item.title} ${item.authors.join(" ")}`.toLowerCase().includes(filter.toLowerCase())), [favorites, filter]);
  const favoriteKeys = useMemo(() => new Set(favorites.map((item) => `${item.providerId}:${item.bookId}`)), [favorites]);
  const pending = downloadQueue.tasks.filter(activeDownload).length;
  const inSettings = page === "settings" || settingsPages.includes(page);
  function switchProvider(value: string) { searchId.current++; setProvider(value); setResults(null); setSelectedBook(null); setSearching(false); }
  async function refreshLibrary() {
    setLibraryLoading(true);
    try { const [items, saved] = await Promise.all([invoke<LibraryItem[]>("library_items"), invoke<FavoriteBook[]>("favorite_books")]); setLibrary(items); setFavorites(saved); } catch (reason) { setNotice({ text: messageOf(reason), error: true }); }
    finally { setLibraryLoading(false); }
  }
  async function importBooks() {
    setImporting(true); setNotice(null);
    try { const items = await invoke<LibraryItem[]>("import_local_books"); if (items.length) { await refreshLibrary(); setNotice({ text: `已将 ${items.length} 本书加入书架`, error: false }); } }
    catch (reason) { setNotice({ text: `导入失败：${messageOf(reason)}`, error: true }); }
    finally { setImporting(false); }
  }
  async function search(targetPage = 1) {
    if (!query.trim() || !provider) return;
    const id = ++searchId.current; setSearching(true); setNotice(null);
    try { const found = await invoke<SearchResult>("search_books", { providerId: provider, text: query.trim(), page: targetPage, pageSize: settings.searchPageSize, formats: [] }); if (id === searchId.current) setResults(found); }
    catch (reason) { if (id === searchId.current) setNotice({ text: `搜索失败：${messageOf(reason)}`, error: true }); }
    finally { if (id === searchId.current) setSearching(false); }
  }
  function submitSearch(event: FormEvent) { event.preventDefault(); void search(); }
  function showDetails(book: BookSummary, from = provider) { setSelectedBook(book); setSelectedProvider(from); }
  async function toggleFavorite(book: BookSummary, from = provider) {
    const selected = favoriteKeys.has(`${from}:${book.id}`);
    try { await invoke("set_book_favorite", { providerId: from, book, favorite: !selected }); setFavorites(await invoke<FavoriteBook[]>("favorite_books")); setNotice({ text: selected ? "已取消收藏" : "已收藏到书架", error: false }); }
    catch (reason) { setNotice({ text: messageOf(reason), error: true }); }
  }
  async function downloadBook(book: BookSummary, format = readableFormat(book), bookProvider = provider) {
    if (!format) { showDetails(book, bookProvider); return; }
    if (bookProvider === "zlibrary" && !account?.signedIn) { setPage("account"); return; }
    bookCache.current[book.title] = book.coverUrl || "";
    try {
      await downloadQueue.enqueue(bookProvider, book.id, book.title, format);
      setSelectedBook(null); setPage("downloads"); setNotice(null);
    } catch (reason) { setNotice({ text: "添加下载失败：" + messageOf(reason), error: true }); }
  }
  async function openBook(item: LibraryItem) {
    setReaderLoading(item.id); setNotice(null);
    try {
      await invoke("validate_library_file", { libraryId: item.id });
      if (item.format === "pdf") { setPdf(item); return; }
      if (!["epub", "txt"].includes(item.format)) { await openExternalFile(item); return; }
      const [book, progress] = await Promise.all([invoke<ReaderBookMetadata>("open_local_book_metadata", { path: item.path }), invoke<ReadingProgress | null>("reading_progress", { libraryId: item.id })]);
      let chapterId = progress?.locator; let offset = 0;
      if (chapterId?.startsWith("{")) {
        try { const locator = JSON.parse(chapterId); chapterId = locator.chapter; offset = Math.min(1, Math.max(0, Number(locator.offset) || 0)); } catch {}
      }
      const found = book.chapters.findIndex((chapter) => chapter.id === chapterId); setReader({ item, book, chapter: Math.max(0, found), offset });
    } catch (reason) { setNotice({ text: `打开失败：${messageOf(reason)}`, error: true }); }
    finally { setReaderLoading(null); }
  }
  function cancelPress() { if (pressTimer.current) { clearTimeout(pressTimer.current); pressTimer.current = null; } }
  function hold(item: LibraryItem, x: number, y: number) { cancelPress(); suppressOpen.current = false; pressOrigin.current = { x, y }; pressTimer.current = setTimeout(() => { suppressOpen.current = true; setManaged(item); }, 550); }
  function readAfterPress(item: LibraryItem) { cancelPress(); if (suppressOpen.current) { suppressOpen.current = false; return; } void openBook(item); }
  function changeChapter(chapter: number) {
    if (progressTimer.current) { clearTimeout(progressTimer.current); progressTimer.current = null; }
    if (!reader) return; setReader({ ...reader, chapter, offset: 0 });
    void invoke("save_reading_progress", { libraryId: reader.item.id, locator: JSON.stringify({ chapter: reader.book.chapters[chapter]?.id, offset: 0 }), fraction: chapter / reader.book.chapters.length }).catch((reason) => setNotice({ text: messageOf(reason), error: true }));
  }
  function saveOffset(offset: number) {
    // The reader tracks its own scroll position; only persist it here so scrolling doesn't re-render the whole shelf.
    if (!reader) return;
    if (progressTimer.current) clearTimeout(progressTimer.current);
    progressTimer.current = setTimeout(() => {
      void invoke("save_reading_progress", { libraryId: reader.item.id, locator: JSON.stringify({ chapter: reader.book.chapters[reader.chapter]?.id, offset }), fraction: (reader.chapter + offset) / reader.book.chapters.length }).catch((reason) => setNotice({ text: messageOf(reason), error: true }));
    }, 400);
  }
  async function removeBook() {
    if (!removing) return;
    try { await invoke(removeFile ? "delete_library_download" : "remove_library_item", { libraryId: removing.id }); setLibrary((current) => current.filter((item) => item.id !== removing.id)); setRemoving(null); }
    catch (reason) { setRemoving(null); setNotice({ text: messageOf(reason), error: true }); }
  }

  const shelf = libraryLoading ? <SkeletonGrid label="正在整理书架…" />
    : shelfView === "favorites" ? (visibleFavorites.length ? <div className="shelf-grid">{visibleFavorites.map((item) => {
      const book = favoriteSummary(item);
      return <BookCard key={`${item.providerId}:${item.bookId}`} title={item.title} subtitle={byline(item.authors, providerName(item.providerId))} openLabel={`查看收藏 ${item.title}`} onOpen={() => showDetails(book, item.providerId)}
        cover={<BookCover title={item.title} authors={item.authors} url={item.coverUrl} format={item.format || ""} />}
        action={<FavoriteButton active title={item.title} onToggle={() => void toggleFavorite(book, item.providerId)} />} />;
    })}</div> : <section className="empty-state">
      <div className="empty-icon" aria-hidden="true"><Heart size={26} strokeWidth={1.6} /></div>
      <h2>{filter ? "收藏里没有这本书" : "收藏想读的书"}</h2>
      <p>收藏只保存书籍条目，不占用空间；想离线阅读时再下载。</p>
      {!filter ? <div><button className="primary-button" onClick={() => setPage("search")}>去找书</button></div> : null}
    </section>)
    : visibleLibrary.length ? <div className="shelf-grid">{visibleLibrary.map((item) => <BookCard key={item.id} title={item.title} subtitle={byline(item.authors, item.format.toUpperCase())}
      openLabel={`阅读 ${item.title}`} busy={readerLoading === item.id} onOpen={() => readAfterPress(item)}
      onContextMenu={(event) => { event.preventDefault(); cancelPress(); setManaged(item); }}
      coverProps={{ onPointerDown: (event) => hold(item, event.clientX, event.clientY), onPointerUp: cancelPress, onPointerCancel: cancelPress, onPointerMove: (event) => { if (Math.hypot(event.clientX - pressOrigin.current.x, event.clientY - pressOrigin.current.y) > 12) cancelPress(); } }}
      cover={<BookCover title={item.title} authors={item.authors} libraryId={item.id} url={bookCache.current[item.title]} format={item.format} />}
      action={<button className="icon-button" aria-label={`管理下载 ${item.title}`} onClick={() => setManaged(item)}><Ellipsis size={19} /></button>} />)}</div>
    : <section className="empty-state">
      {filter ? <div className="empty-icon" aria-hidden="true"><Search size={24} strokeWidth={1.6} /></div> : <div className="empty-books" aria-hidden="true">{[0, 2, 1].map((tone) => <div key={tone} className={`book-cover tone-${tone}`}><div className="type-cover"><i className="type-rule" /></div></div>)}</div>}
      <h2>{filter ? "没有找到这本书" : "书架还空着"}</h2>
      <p>{filter ? "换个书名或作者的关键词试试。" : "导入手边的 EPUB、PDF、TXT，或者去找一本书。读到哪里，下次就从哪里接着读。"}</p>
      {!filter ? <div><button className="primary-button" onClick={() => void importBooks()} disabled={importing}><Plus size={18} />导入书籍</button><button className="secondary-button" onClick={() => setPage("search")}>去找书</button></div> : null}
    </section>;

  return <div className="app-shell">
    <aside className="sidebar">
      <button className="brand" onClick={() => setPage("library")} aria-label="星书书架"><span className="seal" aria-hidden="true">星</span><span className="brand-name"><strong>星书</strong><small>LuminaShelf</small></span></button>
      <nav aria-label="主导航">{navigation.map(({ id, label, icon: Icon }) => <button key={id} onClick={() => setPage(id)} className={page === id || (inSettings && id === "settings") ? "nav-active" : ""} aria-current={page === id ? "page" : undefined}><Icon size={20} strokeWidth={1.8} /><span>{label}</span>{id === "downloads" && pending > 0 ? <span className="count" aria-hidden="true">{pending}</span> : null}</button>)}</nav>
      <button className="account-entry" onClick={() => setPage("account")}><span className="account-avatar" aria-hidden="true"><UserRound size={18} strokeWidth={1.8} /></span><span className="account-text">书源账户<small><i className={`status-dot ${account?.signedIn ? "on" : ""}`} />{account?.signedIn ? "Z-Library 已登录" : "连接你的 Z-Library"}</small></span><ChevronRight size={16} /></button>
    </aside>
    <main className="main-content">
      <header className="page-header">
        <div>
          {settingsPages.includes(page) ? <button className="text-button back-button" onClick={() => setPage("settings")}><ArrowLeft size={18} />设置</button> : <span className="mobile-brand"><span className="seal" aria-hidden="true">星</span>星书</span>}
          <h1>{pageTitle[page]}</h1>
        </div>
        {page === "library" ? <button className="primary-button" onClick={() => void importBooks()} disabled={importing}><Plus size={18} />{importing ? "正在导入" : "导入书籍"}</button> : null}
      </header>
      {notice ? <div className={`notice ${notice.error ? "error" : "success"}`} role={notice.error ? "alert" : "status"}><span>{notice.text}</span>{notice.error && /搜索|下载|DNS|连接/.test(notice.text) ? <button className="text-button" onClick={() => { setNotice(null); setPage("network"); }}>检查网络</button> : null}<button aria-label="关闭提示" onClick={() => setNotice(null)}><X size={18} /></button></div> : null}

      {page === "library" ? <>
        <div className="shelf-tabs" role="tablist" aria-label="书架内容"><button role="tab" aria-selected={shelfView === "downloaded"} onClick={() => setShelfView("downloaded")}>已下载 <span>{library.length}</span></button><button role="tab" aria-selected={shelfView === "favorites"} onClick={() => setShelfView("favorites")}>收藏 <span>{favorites.length}</span></button></div>
        {library.length + favorites.length > 0 ? <div className="library-tools"><label className="search-input"><Search size={18} /><input aria-label="搜索书架" placeholder="在书架中找书" value={filter} onChange={(event) => setFilter(event.target.value)} /></label><button className="icon-button" aria-label="刷新书架" onClick={() => void refreshLibrary()}><RefreshCw size={18} /></button></div> : null}
        {shelf}
        <button className="announcement-entry" onClick={() => setPage("announcements")}><Megaphone size={16} />公告栏 · 项目动态与书源公告<ChevronRight size={16} /></button>
      </> : null}

      {page === "search" ? <>
        <form className="book-search" onSubmit={submitSearch}><label className="search-input"><Search size={19} /><input aria-label="书名或作者" placeholder="书名、作者或关键词" value={query} onChange={(event) => setQuery(event.target.value)} /></label><button className="primary-button" disabled={searching || !query.trim()}>{searching ? "正在搜索" : "搜索"}</button></form>
        <div className="source-picker" role="group" aria-label="选择书源">{providers.filter((item) => item.capabilities.searchable).map((item) => <button key={item.id} type="button" className={provider === item.id ? "selected" : ""} aria-pressed={provider === item.id} onClick={() => switchProvider(item.id)}>{provider === item.id ? <Check size={15} /> : null}{providerName(item.id)}</button>)}</div>
        {signedOut ? <div className="source-login"><p>未登录也可以浏览和搜索，下载需要登录书源账户。</p><button className="text-button" onClick={() => setPage("account")}>登录<ChevronRight size={16} /></button></div> : null}
        {searching ? <SkeletonGrid label="正在找书…" count={6} /> : results ? <>
          <div className="results-heading"><span>{results.items.length ? `第 ${results.page} 页 · ${results.items.length} 本` : "没有匹配的书籍，换个关键词试试"}</span><div><button className="secondary-button" disabled={results.page <= 1 || searching} onClick={() => void search(results.page - 1)}>上一页</button><button className="secondary-button" disabled={!results.hasNext || searching} onClick={() => void search(results.page + 1)}>下一页</button></div></div>
          <div className="shelf-grid search-results">{results.items.map((book) => <BookCard key={book.id} title={book.title} subtitle={byline(book.authors, "作者未提供")} openLabel={`查看 ${book.title}`} onOpen={() => showDetails(book)}
            cover={<BookCover title={book.title} authors={book.authors} url={book.coverUrl} format={readableFormat(book) || ""} />}
            action={<FavoriteButton active={favoriteKeys.has(`${provider}:${book.id}`)} title={book.title} onToggle={() => void toggleFavorite(book)} />} />)}</div>
        </> : provider ? <BookSourceHome key={`${provider}:${account?.signedIn}:${account?.origin}`} providerId={provider} favorites={favoriteKeys} onSelect={showDetails} onFavorite={(book) => void toggleFavorite(book)} /> : null}
      </> : null}

      {page === "downloads" ? <DownloadQueue queue={downloadQueue} onRead={(item) => void openBook(item)} onManageFile={setManaged} onFind={() => setPage("search")} /> : null}

      {page === "settings" ? <div className="settings-content">
        <section className="settings-section"><h2>账户与书源</h2><div className="group">
          <button className="settings-row" onClick={() => setPage("account")}><UserRound size={20} strokeWidth={1.8} /><span><strong>Z-Library 账户</strong><small>{account?.signedIn ? "已登录" : "登录后可以下载书籍"}</small></span><ChevronRight size={18} /></button>
          <label className="settings-field"><span>默认书源</span><select value={settings.defaultProvider} onChange={(event) => setSettings({ ...settings, defaultProvider: event.target.value })}><option value="">自动选择</option>{providers.map((item) => <option value={item.id} key={item.id}>{providerName(item.id)}</option>)}</select></label>
          <label className="settings-field"><span>每页搜索结果</span><select value={settings.searchPageSize} onChange={(event) => setSettings({ ...settings, searchPageSize: Number(event.target.value) })}>{[12, 24, 36, 50].map((count) => <option key={count} value={count}>{count} 本</option>)}</select></label>
        </div></section>
        <section className="settings-section"><h2>连接</h2><div className="group">
          <button className="settings-row" onClick={() => setPage("network")}><Wifi size={20} strokeWidth={1.8} /><span><strong>DNS 服务器与网络</strong><small>DoH、DoT、代理与应用内 Hosts</small></span><ChevronRight size={18} /></button>
          <button className="settings-row" onClick={() => setPage("mirrors")}><Globe size={20} strokeWidth={1.8} /><span><strong>书源镜像与发布源</strong><small>{account?.origin ? new URL(account.origin).hostname : "检测与切换 Z-Library 站点"}{account?.originMode === "automatic" ? " · 自动切换" : ""}</small></span><ChevronRight size={18} /></button>
          <button className="settings-row" onClick={() => setPage("announcements")}><Megaphone size={20} strokeWidth={1.8} /><span><strong>公告与检查更新</strong><small>当前版本 {version}</small></span><ChevronRight size={18} /></button>
        </div></section>
        <section className="about-section"><p><strong>星书</strong> · LuminaShelf {version}</p><p>找书、收书、读书的本地书架。</p><a href="https://github.com/Junyxor/LuminaShelf" target="_blank" rel="noreferrer">项目主页与开源说明<ChevronRight size={15} /></a></section>
      </div> : null}
      {page === "account" ? <ZLibraryAccount initialStatus={account} onStatusChange={setAccount} /> : null}
      {page === "network" ? <div className="settings-content"><NetworkSettings /></div> : null}
      {page === "mirrors" ? <div className="settings-content"><MirrorPreferences onSelected={(status) => { setAccount(status); setResults(null); }} /></div> : null}
      {page === "announcements" ? <AnnouncementsPage version={version} /> : null}
    </main>
    <nav className="mobile-nav" aria-label="手机主导航">{navigation.map(({ id, label, icon: Icon }) => <button key={id} aria-current={page === id ? "page" : undefined} className={page === id || (inSettings && id === "settings") ? "nav-active" : ""} onClick={() => setPage(id)}><Icon size={22} strokeWidth={1.7} /><span>{label}</span>{id === "downloads" && pending > 0 ? <span className="count" aria-hidden="true">{pending}</span> : null}</button>)}</nav>
    <BookDetailsDialog book={selectedBook} providerId={selectedProvider} favorite={Boolean(selectedBook && favoriteKeys.has(`${selectedProvider}:${selectedBook.id}`))} signedIn={Boolean(account?.signedIn)} onClose={() => setSelectedBook(null)} onFavorite={(book) => void toggleFavorite(book, selectedProvider)} onDownload={(book, format) => void downloadBook(book, format, selectedProvider)} onLogin={() => { setSelectedBook(null); setPage("account"); }} />
    <DownloadedBookActions item={managed} onClose={() => setManaged(null)} onRead={(item) => void openBook(item)} onRemove={(item) => { setRemoveFile(false); setRemoving(item); }} onDelete={(item) => { setRemoveFile(true); setRemoving(item); }} onRetry={(item) => showDetails({ id: item.sourceBookId!, title: item.title, authors: item.authors, format: item.format as BookFormat, availableFormats: [item.format as BookFormat] }, item.sourceProvider!)} onConverted={(item) => setLibrary((current) => [item, ...current.filter((value) => value.id !== item.id)])} />
    <Dialog.Root open={Boolean(removing)} onOpenChange={(open) => { if (!open) setRemoving(null); }}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="confirm-dialog"><Dialog.Title>{removeFile ? "删除下载文件？" : "从书架移除？"}</Dialog.Title><Dialog.Description>“{removing?.title}”将从已下载书架移除。{removeFile ? "应用管理的本地副本会删除，收藏会保留。" : "本地文件会保留。"}</Dialog.Description><div className="dialog-actions"><Dialog.Close className="secondary-button">取消</Dialog.Close><button className="danger-button" onClick={() => void removeBook()}>{removeFile ? "删除文件" : "移除"}</button></div></Dialog.Content></Dialog.Portal></Dialog.Root>
    {reader ? <ReaderView book={reader.book} bookPath={reader.item.path} fallbackTitle={reader.item.title} format={reader.item.format} chapterIndex={reader.chapter} initialOffset={reader.offset} onProgressChange={saveOffset} onChapterChange={changeChapter} onClose={() => setReader(null)} /> : null}
    {pdf ? <PdfReader libraryId={pdf.id} path={pdf.path} title={pdf.title} onClose={() => setPdf(null)} /> : null}
  </div>;
}
