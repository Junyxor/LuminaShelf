import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import BookCover from "./BookCover";
import BookCard, { FavoriteButton, SkeletonGrid } from "./BookCard";
import type { BookSummary } from "./types";
type Discovery = { title: string; sourceUrl: string; items: BookSummary[]; fetchedAtUnixMs: number; stale: boolean; error?: string | null };

function fetchedLabel(at: number) {
  const minutes = Math.round((Date.now() - at) / 60000);
  if (minutes < 1) return "刚刚更新";
  if (minutes < 60) return `${minutes} 分钟前更新`;
  return new Date(at).toLocaleString();
}

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
    <div className="discovery-heading">
      <div><h2>{data?.title || (providerId === "gutendex" ? "古登堡热门" : "书源首页")}</h2><p>{providerId === "gutendex" ? "按古登堡实际下载次数排列的公版书" : "来自当前书源的书单"}</p></div>
      <button className="icon-button" aria-label="刷新书源首页" disabled={busy} onClick={() => void refresh()}><RefreshCw size={18} /></button>
    </div>
    <p className="discovery-source">{data ? `${new URL(data.sourceUrl).hostname} · ${fetchedLabel(data.fetchedAtUnixMs)}${data.stale ? " · 显示上次缓存" : ""}` : " "}</p>
    {error || data?.error ? <p className="network-feedback error" role="alert">{error || data?.error}<button className="text-button" disabled={busy} onClick={() => void refresh()}>重试</button></p> : null}
    {busy && !data ? <SkeletonGrid label="正在读取书源首页…" /> : null}
    {data?.items.length ? <div className="shelf-grid">{data.items.map((book) => <BookCard key={book.id} title={book.title} subtitle={book.authors.join(" · ") || "作者未提供"} openLabel={`查看 ${book.title}`} onOpen={() => onSelect(book)}
      cover={<BookCover title={book.title} authors={book.authors} url={book.coverUrl} format={book.format === "other" ? "" : book.format || ""} />}
      action={<FavoriteButton active={favorites.has(`${providerId}:${book.id}`)} title={book.title} onToggle={() => onFavorite(book)} />} />)}</div>
      : !busy && !error ? <p className="settings-hint">书源当前没有提供首页书籍，可以直接搜索。</p> : null}
  </section>;
}
