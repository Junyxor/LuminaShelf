import { invoke } from "@tauri-apps/api/core";
import * as Dialog from "@radix-ui/react-dialog";
import { Heart, Download, X, ExternalLink } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import BookCover from "./BookCover";
import type { BookSummary, BookDetails, BookFormat } from "./types";
type Options = { variants: BookSummary[]; websiteUrl?: string | null; siteFeatures: { email?: boolean; kindle?: boolean; onlineReading?: boolean; nativeSendSupported?: boolean } };

function size(bytes?: number | null) { return bytes ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : null; }

export default function BookDetailsDialog({ book, providerId, favorite, signedIn, onClose, onFavorite, onDownload, onLogin }: { book: BookSummary | null; providerId: string; favorite: boolean; signedIn: boolean; onClose: () => void; onFavorite: (book: BookSummary) => void; onDownload: (book: BookSummary, format: BookFormat) => void; onLogin: () => void }) {
  const [details, setDetails] = useState<BookDetails | null>(null);
  const [options, setOptions] = useState<Options | null>(null);
  const [selection, setSelection] = useState(0);
  const [busy, setBusy] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [clamped, setClamped] = useState(false);
  const descriptionRef = useRef<HTMLParagraphElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!book) return;
    let active = true; setBusy(true); setError(null); setDetails(null); setOptions(null); setSelection(0); setExpanded(false);
    void invoke<BookDetails>("book_details", { providerId, bookId: book.id }).then(async (result) => {
      if (!active) return; setDetails(result);
      const next = await invoke<Options>("book_download_options", { providerId, bookId: book.id }); if (active) setOptions(next);
    }).catch((reason) => { if (active) setError(String(reason)); }).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [book?.id, providerId]);
  const variants = options?.variants.length ? options.variants : details ? details.availableFormats.map((format) => ({ ...details, format })) : [];
  const selected = variants[selection];
  const needsLogin = providerId === "zlibrary" && !signedIn;
  const shown = details || book;
  const description = busy && !details ? "正在读取书籍详情…" : details?.description || "这本书暂时没有简介。";
  // Offer "展开" only when the clamp actually hides text.
  useLayoutEffect(() => {
    const element = descriptionRef.current;
    if (element && !expanded) setClamped(element.scrollHeight > element.clientHeight + 1);
  }, [description, expanded]);
  const meta = shown ? [shown.format && shown.format !== "other" ? shown.format.toUpperCase() : null, size(shown.sizeBytes), shown.language, shown.year ? `${shown.year}` : null, providerId === "gutendex" ? "公版书" : "Z-Library"].filter(Boolean) : [];
  return <Dialog.Root open={Boolean(book)} onOpenChange={(open) => { if (!open) onClose(); }}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="book-dialog" aria-describedby="book-description"><Dialog.Close className="icon-button dialog-close" aria-label="关闭书籍详情"><X size={20} /></Dialog.Close>
    {book && shown ? <>
      <div className="detail-head">
        <div className="detail-cover"><BookCover title={book.title} authors={book.authors} url={details?.coverUrl || book.coverUrl} format={shown.format || ""} /></div>
        <div>
          <Dialog.Title>{shown.title}</Dialog.Title>
          <p className="detail-author">{shown.authors.join(" · ") || "作者未提供"}</p>
          <p className="detail-meta">{meta.map((value, index) => <span key={index}>{value}</span>)}</p>
        </div>
      </div>
      <Dialog.Description ref={descriptionRef} id="book-description" className={`detail-description ${expanded ? "open" : ""}`}>{description}</Dialog.Description>
      {clamped || expanded ? <button className="text-button detail-more" onClick={() => setExpanded((value) => !value)}>{expanded ? "收起简介" : "展开简介"}</button> : null}
      <div className="detail-actions">
        <div className="format-row">
          {variants.length ? <select aria-label="可用文件格式" value={selection} onChange={(event) => setSelection(Number(event.target.value))}>{variants.map((variant, index) => <option key={`${variant.id}:${variant.format}:${index}`} value={index}>{variant.format?.toUpperCase() || "格式待确认"}{variant.sizeBytes ? ` · ${size(variant.sizeBytes)}` : ""}{index ? " · 其他版本" : ""}</option>)}</select> : <select aria-label="可用文件格式" disabled><option>{busy ? "正在读取可用格式…" : "暂无可下载格式"}</option></select>}
          <button className={`icon-button ${favorite ? "is-on" : ""}`} aria-label={favorite ? "已收藏 · 点击取消" : "收藏到书架"} aria-pressed={favorite} onClick={() => onFavorite(details || book)}><Heart size={20} strokeWidth={1.8} fill={favorite ? "currentColor" : "none"} /></button>
        </div>
        {error ? <p className="network-feedback error" role="alert">{error}</p> : null}
        {needsLogin ? <button className="primary-button full-width" onClick={onLogin}>登录 Z-Library 后下载</button> : <button className="primary-button full-width" disabled={busy || !selected?.format || selected.format === "other"} onClick={() => { if (selected?.format) onDownload(selected, selected.format); }}><Download size={19} />{busy ? "正在读取可用格式…" : selected?.format ? `下载 ${selected.format.toUpperCase()}` : "暂无可下载格式"}</button>}
        <p className="detail-note">收藏不占用空间。下载后可以离线阅读、分享文件，或在 TXT 与 EPUB 之间转换。</p>
      </div>
      {providerId === "zlibrary" ? <div className="official-services"><h3>官网服务</h3><p>{options?.siteFeatures.email ? "该书在官网标记支持发送到邮箱。" : "发送到邮箱、Kindle 和网盘取决于站点与账户权限。"}转码与发送请在官网书籍页操作。</p><button className="text-button" onClick={() => { void invoke("open_book_website", { providerId, bookId: book.id }).catch((reason) => setError(String(reason))); }}><ExternalLink size={16} />打开官网书籍页</button></div> : null}
    </> : <Dialog.Title>书籍详情</Dialog.Title>}
  </Dialog.Content></Dialog.Portal></Dialog.Root>;
}
