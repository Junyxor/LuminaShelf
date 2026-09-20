import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

export type Bookmark = { id: string; label: string; locator: string; fraction: number };
type Props = { libraryId: string; label: string; locator: () => string; fraction: () => number; onSelect: (bookmark: Bookmark) => void };

export default function ReaderBookmarks({ libraryId, label, locator, fraction, onSelect }: Props) {
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<Bookmark[]>([]);
  const [error, setError] = useState("");
  const [name, setName] = useState("");
  useEffect(() => {
    void invoke<Bookmark[]>("bookmarks", { libraryId }).then(setItems).catch((reason) => setError(String(reason)));
  }, [libraryId]);
  async function add() {
    try {
      const item = await invoke<Bookmark>("add_bookmark", { libraryId, label: name.trim() || label, locator: locator(), fraction: fraction() });
      setItems((current) => [...current, item].sort((a, b) => a.fraction - b.fraction));
      setName(""); setError("");
    } catch (reason) { setError(String(reason)); }
  }
  async function remove(id: string) {
    try { await invoke("remove_bookmark", { id }); setItems((current) => current.filter((item) => item.id !== id)); }
    catch (reason) { setError(String(reason)); }
  }
  return <div className="bookmark-widget">
    <button className="ghost-button" aria-label="书签" aria-expanded={open} onClick={() => setOpen(!open)}>书签 {items.length || ""}</button>
    {open ? <section className="bookmark-panel" aria-label="阅读书签">
      <div className="bookmark-add"><input aria-label="书签备注" placeholder={label} value={name} maxLength={160} onChange={(event) => setName(event.target.value)} /><button className="primary-button" onClick={() => void add()}>添加书签</button></div>
      {error ? <p role="alert">{error}</p> : null}
      {items.length === 0 ? <p>在喜欢的位置留下书签。</p> : items.map((item) => <div className="bookmark-row" key={item.id}>
        <button onClick={() => { onSelect(item); setOpen(false); }}><span>{item.label}</span><small>{Math.round(item.fraction * 100)}%</small></button>
        <button aria-label={`删除书签 ${item.label}`} onClick={() => void remove(item.id)}>×</button>
      </div>)}
    </section> : null}
  </div>;
}
