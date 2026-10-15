import { invoke } from "@tauri-apps/api/core";
import * as Dialog from "@radix-ui/react-dialog";
import { Heart, Download, X, ExternalLink } from "lucide-react";
import { useEffect, useState } from "react";
import BookCover from "./BookCover";
import type { BookSummary, BookDetails, BookFormat } from "./types";
type Options = { variants: BookSummary[]; websiteUrl?: string | null; siteFeatures: { email?: boolean; kindle?: boolean; onlineReading?: boolean; nativeSendSupported?: boolean } };
export default function BookDetailsDialog({ book, providerId, favorite, signedIn, onClose, onFavorite, onDownload, onLogin }: { book: BookSummary | null; providerId: string; favorite: boolean; signedIn: boolean; onClose: () => void; onFavorite: (book: BookSummary) => void; onDownload: (book: BookSummary, format: BookFormat) => void; onLogin: () => void }) {
  const [details, setDetails] = useState<BookDetails | null>(null);
  const [options, setOptions] = useState<Options | null>(null);
  const [selection, setSelection] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!book) return;
    let active = true; setBusy(true); setError(null); setDetails(null); setOptions(null); setSelection(0);
    void invoke<BookDetails>("book_details", { providerId, bookId: book.id }).then(async (result) => {
      if (!active) return; setDetails(result);
      const next = await invoke<Options>("book_download_options", { providerId, bookId: book.id }); if (active) setOptions(next);
    }).catch((reason) => { if (active) setError(String(reason)); }).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [book?.id, providerId]);
  const variants = options?.variants.length ? options.variants : details ? details.availableFormats.map((format) => ({ ...details, format })) : [];
  const selected = variants[selection];
  const needsLogin = providerId === "zlibrary" && !signedIn;
  return <Dialog.Root open={Boolean(book)} onOpenChange={(open) => { if (!open) onClose(); }}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="book-dialog" aria-describedby="book-description"><Dialog.Close className="icon-button dialog-close" aria-label="关闭书籍详情"><X size={22} /></Dialog.Close>
    {book ? <><div className="detail-cover"><BookCover title={book.title} authors={book.authors} url={details?.coverUrl || book.coverUrl} /></div><Dialog.Title>{details?.title || book.title}</Dialog.Title><p className="detail-author">{(details?.authors || book.authors).join(" · ") || "作者未提供"}</p><Dialog.Description id="book-description">{busy && !details ? "正在读取书籍详情…" : details?.description || "暂无简介"}</Dialog.Description>
      <button className="secondary-button full-width" onClick={() => onFavorite(details || book)}><Heart size={19} fill={favorite ? "currentColor" : "none"} />{favorite ? "已收藏 · 点击取消" : "收藏到书架"}</button>
      <h3 className="download-method-heading">下载方式</h3>
      {variants.length ? <label className="network-field"><span>可用文件格式</span><select aria-label="可用文件格式" value={selection} onChange={(event) => setSelection(Number(event.target.value))}>{variants.map((variant, index) => <option key={`${variant.id}:${variant.format}:${index}`} value={index}>{variant.format?.toUpperCase() || "格式待确认"}{variant.sizeBytes ? ` · ${(variant.sizeBytes / 1024 / 1024).toFixed(1)} MB` : ""}{index ? " · 其他版本" : " · 原文件"}</option>)}</select></label> : null}
      {error ? <p className="network-feedback error" role="alert">{error}</p> : null}
      {needsLogin ? <button className="primary-button full-width" onClick={onLogin}>登录后下载</button> : <button className="primary-button full-width" disabled={busy || !selected?.format || selected.format === "other"} onClick={() => { if (selected?.format) onDownload(selected, selected.format); }}><Download size={19} />{busy ? "正在读取可用格式…" : selected?.format ? `下载 ${selected.format.toUpperCase()}` : "暂无可下载格式"}</button>}
      <p className="settings-hint">收藏不需要下载。下载后可以离线阅读、分享文件，或将 TXT 与 EPUB 在本地互转。</p>
      {providerId === "zlibrary" ? <div className="official-services"><h3>官网服务</h3><p>{options?.siteFeatures.email ? "该书在官网标记支持发送邮箱。" : "发送邮箱、Kindle 和网盘由站点与账户权限决定。"}官网转码与发送可在书籍页面操作。</p><button className="text-button" onClick={() => { void invoke("open_book_website", { providerId, bookId: book.id }).catch((reason) => setError(String(reason))); }}><ExternalLink size={18} />打开官网书籍页面</button></div> : null}
    </> : <Dialog.Title>书籍详情</Dialog.Title>}
  </Dialog.Content></Dialog.Portal></Dialog.Root>;
}
