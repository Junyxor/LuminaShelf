import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { RefreshCw, Heart, ChevronRight } from "lucide-react";
import BookCover from "./BookCover";
import type { BookSummary } from "./types";
type Discovery = { title: string; sourceUrl: string; items: BookSummary[]; fetchedAtUnixMs: number; stale: boolean; error?: string | null };
export default function BookSourceHome({ providerId, favorites, onSelect, onFavorite }: { providerId: string; favorites: Set<string>; onSelect: (book: BookSummary) => void; onFavorite: (book: BookSummary) => void }) {
  const [data, setData] = useState<Discovery | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let active = true; setBusy(true); setError(null);
    void invoke<Discovery>("discover_books", { providerId, refresh: false }).then((value) => { if (active) setData(value); }).catch((reason) => { if (active) setError(String(reason)); }).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [providerId]);
  async function refresh() { setBusy(true); setError(null); try { setData(await invoke<Discovery>("discover_books", { providerId, refresh: true })); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  return <section className="source-home" aria-label="书源首页">
    <div className="discovery-heading"><div><h2>{data?.title || "书源首页"}</h2><p>{providerId === "gutendex" ? "按古登堡实际下载热度排列" : "来自当前书源的真实书单"}</p></div><button className="icon-button" aria-label="刷新书源首页" disabled={busy} onClick={() => void refresh()}><RefreshCw size={21} /></button></div>
    {data ? <p className="discovery-source">{new URL(data.sourceUrl).hostname} · {new Date(data.fetchedAtUnixMs).toLocaleString()}{data.stale ? " · 上次缓存" : ""}</p> : null}
    {error || data?.error ? <p className="network-feedback error" role="alert">{error || data?.error}<button className="text-button" disabled={busy} onClick={() => void refresh()}>重试</button></p> : null}
    {busy && !data ? <p className="loading-state" role="status">正在读取书源首页…</p> : null}
    {data?.items.length ? <div className="shelf-grid">{data.items.map((book) => <article className="shelf-book" key={book.id}><button className="cover-button" onClick={() => onSelect(book)} aria-label={`查看 ${book.title}`}><BookCover title={book.title} authors={book.authors} url={book.coverUrl} format={book.format === "other" ? "" : book.format || ""} /></button><h3>{book.title}</h3><p>{book.authors.join(" · ") || "作者未提供"}</p><div className="shelf-actions"><button className="text-button" onClick={() => onFavorite(book)} aria-label={`${favorites.has(`${providerId}:${book.id}`) ? "取消收藏" : "收藏"} ${book.title}`}><Heart size={18} fill={favorites.has(`${providerId}:${book.id}`) ? "currentColor" : "none"} />{favorites.has(`${providerId}:${book.id}`) ? "已收藏" : "收藏"}</button><button className="icon-button" aria-label={`书籍详情 ${book.title}`} onClick={() => onSelect(book)}><ChevronRight size={19} /></button></div></article>)}</div> : !busy && !error ? <p className="settings-hint">书源当前没有提供首页书籍，可以继续使用搜索。</p> : null}
  </section>;
}
