import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";

type Item = { id: string; title: string; authors: string[]; path: string; format: string; sizeBytes: number };
export default function LibraryActions({ item, onClose, onChanged }: { item: Item; onClose: () => void; onChanged: (item: Item | null) => void }) {
  const [title, setTitle] = useState(item.title);
  const [authors, setAuthors] = useState(item.authors.join("、"));
  const [confirm, setConfirm] = useState(false);
  const [deleteFile, setDeleteFile] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  async function run(action: "save" | "export" | "remove" | "share") {
    setBusy(true); setMessage("");
    try {
      if (action === "save") {
        const saved = await invoke<Item>("edit_library_item", { id: item.id, title, authors: authors.split(/[、,，]/) });
        onChanged(saved);
      } else if (action === "export") {
        const saved = await invoke<boolean>("export_library_book", { id: item.id });
        setMessage(saved ? "已导出副本。" : "已取消导出。");
      } else if (action === "share") {
        await invoke("share_library_book", { id: item.id });
      } else {
        await invoke("remove_library_item", { id: item.id, deleteFile });
        onChanged(null);
      }
    } catch (reason) { setMessage(String(reason)); }
    finally { setBusy(false); }
  }
  return <div className="detail-backdrop" onClick={busy ? undefined : onClose}>
    <section className="detail-drawer glass library-manage" role="dialog" aria-modal="true" aria-label="管理书籍" onClick={(event) => event.stopPropagation()}>
      <button className="drawer-close" disabled={busy} onClick={onClose} aria-label="关闭书籍管理">×</button>
      <h2>管理书籍</h2>
      <label>书名<input aria-label="编辑书名" value={title} onChange={(event) => setTitle(event.target.value)} maxLength={300} /></label>
      <label>作者<input aria-label="编辑作者" value={authors} onChange={(event) => setAuthors(event.target.value)} placeholder="多位作者用顿号分隔" /></label>
      <p className="search-hint">{item.format.toUpperCase()} · {item.path}</p>
      {message ? <p role="status">{message}</p> : null}
      <div className="library-card-actions">
        <button className="primary-button" disabled={busy || !title.trim()} onClick={() => void run("save")}>保存信息</button>
        <button className="ghost-button" disabled={busy} onClick={() => void run("export")}>导出电子书</button>
        <button className="ghost-button" disabled={busy} onClick={() => void run("share")}>分享</button>
      </div>
      {confirm ? <div className="remove-confirm">
        <p>移除《{item.title}》及其阅读进度、书签？</p>
        <label><input type="checkbox" checked={deleteFile} onChange={(event) => setDeleteFile(event.target.checked)} />同时删除应用内的文件副本</label>
        <p className="search-hint">应用之外的原文件不会被删除。需要保留进度时，请先导出书库备份。</p>
        <button className="ghost-button danger" disabled={busy} onClick={() => void run("remove")}>确认移除</button>
        <button className="ghost-button" disabled={busy} onClick={() => setConfirm(false)}>保留书籍</button>
      </div> : <button className="ghost-button danger" disabled={busy} onClick={() => setConfirm(true)}>从书库移除</button>}
    </section>
  </div>;
}
