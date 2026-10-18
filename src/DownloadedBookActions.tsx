import { invoke } from "@tauri-apps/api/core";
import * as Dialog from "@radix-ui/react-dialog";
import { useState } from "react";
import { X, BookOpen, ExternalLink, Share2, Download, ArrowRightLeft, Trash2, RefreshCw, FolderMinus } from "lucide-react";
import type { LibraryItem } from "./types";
import { openExternalFile } from "./external-files";
type FileBridge = { shareFile: (path: string, mime: string, title: string) => string };
function mime(format: string) { return ({ epub: "application/epub+zip", pdf: "application/pdf", txt: "text/plain", mobi: "application/x-mobipocket-ebook", zip: "application/zip" } as Record<string, string>)[format] || "application/octet-stream"; }
export default function DownloadedBookActions({ item, onClose, onRead, onRemove, onDelete, onRetry, onConverted }: { item: LibraryItem | null; onClose: () => void; onRead: (item: LibraryItem) => void; onRemove: (item: LibraryItem) => void; onDelete: (item: LibraryItem) => void; onRetry: (item: LibraryItem) => void; onConverted: (item: LibraryItem) => void }) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const bridge = (window as Window & { LuminaFiles?: FileBridge }).LuminaFiles;
  async function perform(work: () => Promise<void>) { setBusy(true); setError(null); setMessage(null); try { await work(); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  return <Dialog.Root open={Boolean(item)} onOpenChange={(open) => { if (!open) { onClose(); setMessage(null); setError(null); } }}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="book-dialog file-actions-dialog" aria-describedby="file-actions-description"><Dialog.Close className="icon-button dialog-close" aria-label="关闭下载管理"><X size={20} /></Dialog.Close><Dialog.Title>{item?.title || "下载管理"}</Dialog.Title><Dialog.Description id="file-actions-description" className="settings-hint">{item?.format.toUpperCase()} · {((item?.sizeBytes || 0) / 1024 / 1024).toFixed(1)} MB{item?.authors.length ? ` · ${item.authors.join(" · ")}` : ""}</Dialog.Description>
    {item ? <div className="file-action-list">
      <button className="settings-row" disabled={busy} onClick={() => { onClose(); onRead(item); }}><BookOpen size={20} strokeWidth={1.8} /><span><strong>阅读</strong><small>{["epub", "pdf", "txt"].includes(item.format) ? "从上次的位置继续" : "此格式交给外部应用阅读"}</small></span></button>
      <button className="settings-row" disabled={busy} onClick={() => void perform(async () => { await openExternalFile(item); })}><ExternalLink size={20} strokeWidth={1.8} /><span><strong>使用其他应用打开</strong></span></button>
      {bridge ? <button className="settings-row" disabled={busy} onClick={() => { const result = bridge.shareFile(item.path, mime(item.format), item.title); if (result) setError(result); }}><Share2 size={20} strokeWidth={1.8} /><span><strong>分享至邮箱或云盘</strong></span></button> : null}
      <button className="settings-row" disabled={busy} onClick={() => void perform(async () => { const path = await invoke<string | null>("export_library_book", { libraryId: item.id }); if (path) setMessage("已导出文件"); })}><Download size={20} strokeWidth={1.8} /><span><strong>导出文件</strong><small>另存到文件夹</small></span></button>
      {["epub", "txt"].includes(item.format) ? <button className="settings-row" disabled={busy} onClick={() => void perform(async () => { const converted = await invoke<LibraryItem>("convert_library_book", { libraryId: item.id, target: item.format === "txt" ? "epub" : "txt" }); onConverted(converted); setMessage("转换完成，已加入已下载书架"); })}><ArrowRightLeft size={20} strokeWidth={1.8} /><span><strong>{busy ? "正在处理…" : `本地转换为 ${item.format === "txt" ? "EPUB" : "TXT"}`}</strong><small>只保留文字，不含图片与排版</small></span></button> : null}
      <button className="settings-row" disabled={busy} onClick={() => { onClose(); onRemove(item); }}><FolderMinus size={20} strokeWidth={1.8} /><span><strong>从书架移除</strong><small>文件保留在磁盘上</small></span></button>
      {item.sourceProvider && item.sourceBookId ? <button className="settings-row" disabled={busy} onClick={() => { onClose(); onRetry(item); }}><RefreshCw size={20} strokeWidth={1.8} /><span><strong>重新获取文件</strong><small>查看书源当前提供的格式</small></span></button> : null}
      <button className="settings-row danger" disabled={busy} onClick={() => { onClose(); onDelete(item); }}><Trash2 size={20} strokeWidth={1.8} /><span><strong>删除下载文件</strong><small>删除应用管理的本地副本，收藏保留</small></span></button>
    </div> : null}
    {message ? <p className="network-feedback success" role="status">{message}</p> : null}{error ? <p className="network-feedback error" role="alert">{error}</p> : null}
  </Dialog.Content></Dialog.Portal></Dialog.Root>;
}
