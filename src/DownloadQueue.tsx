import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as Dialog from "@radix-ui/react-dialog";
import { useEffect, useRef, useState, type PointerEvent } from "react";
import { BookOpen, Download, Ellipsis, Pause, Play, RefreshCw, Trash2, X } from "lucide-react";
import BookCover from "./BookCover";
import type { BookFormat, LibraryItem } from "./types";

export type DownloadState = "queued" | "connecting" | "downloading" | "pausing" | "paused" | "verifying" | "completed" | "failed" | "cancelled";
export type DownloadTask = { id: string; generation: string; title: string; providerId: string; bookId: string; format: BookFormat; destination: string; state: DownloadState; totalBytes?: number | null; downloadedBytes: number; bytesPerSecond: number; connections: number; activeConnections: number; peakConnections: number; supportsResume?: boolean | null; retryCount: number; error?: string | null; errorDetails?: string | null; note?: string | null; libraryId?: string | null; revision: number; createdAtUnixMs: number; updatedAtUnixMs: number };
type Settings = { connections: number; simultaneousBooks: number };
const activeStates: DownloadState[] = ["queued", "connecting", "downloading", "pausing", "verifying"];
export function activeDownload(task: DownloadTask) { return activeStates.includes(task.state); }
function merge(current: Record<string, DownloadTask>, task: DownloadTask) { const old = current[task.id]; return old && (old.createdAtUnixMs > task.createdAtUnixMs || old.generation === task.generation && old.revision > task.revision) ? current : { ...current, [task.id]: task }; }
function bytes(value?: number | null) { if (!value) return "0 B"; const units = ["B", "KB", "MB", "GB"]; let unit = 0; while (value >= 1024 && unit < 3) { value /= 1024; unit++; } return `${value.toFixed(unit && value < 10 ? 1 : 0)} ${units[unit]}`; }
const filters = [["all", "全部"], ["active", "进行中"], ["paused", "已暂停"], ["failed", "失败"], ["completed", "已完成"]] as const;
function matches(task: DownloadTask, filter: string) { return filter === "all" || (filter === "active" && activeDownload(task)) || filter === task.state; }
const labels: Record<DownloadState, string> = { queued: "等待下载", connecting: "连接中", downloading: "下载中", pausing: "保存进度…", paused: "已暂停", verifying: "校验文件", completed: "已完成", failed: "下载中断", cancelled: "已取消" };

export function useDownloadQueue(onComplete: (task: DownloadTask) => void) {
  const [records, setRecords] = useState<Record<string, DownloadTask>>({});
  const [settings, setSettings] = useState<Settings>({ connections: 4, simultaneousBooks: 2 });
  const settingsRef = useRef(settings);
  const settingsWrite = useRef<Promise<unknown>>(Promise.resolve());
  const settingsEpoch = useRef(0);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [backgroundWarning, setBackgroundWarning] = useState<string | null>(null);
  const callback = useRef(onComplete); callback.current = onComplete;
  const tasksRef = useRef(records); tasksRef.current = records;
  const mutation = useRef(0);
  const removed = useRef(new Set<string>());
  const retired = useRef(new Set<string>());
  const completed = useRef(new Set<string>());
  function notify(task: DownloadTask) {
    const key = `${task.id}:${task.generation}:${task.revision}`;
    if (task.state === "completed" && !completed.current.has(key) && !retired.current.has(task.generation)) { completed.current.add(key); callback.current(task); }
  }
  async function reload() {
    const epoch = mutation.current;
    const tasks = await invoke<DownloadTask[]>("download_tasks");
    if (epoch !== mutation.current) return;
    setRecords((current) => {
      const ids = new Set(tasks.map((task) => task.id));
      const saved = Object.fromEntries(Object.entries(current).filter(([id]) => ids.has(id)));
      return tasks.filter((task) => !removed.current.has(task.id)).reduce(merge, saved);
    });
    tasks.filter((task) => !removed.current.has(task.id)).forEach(notify);
  }
  useEffect(() => {
    let disposed = false;
    const stops: Array<() => void> = [];
    void Promise.allSettled([
      reload().catch((reason) => { if (!disposed) setError(String(reason)); }),
      invoke<Settings>("download_settings").then((value) => { if (!disposed && !settingsEpoch.current) { settingsRef.current = value; setSettings(value); } }),
      listen<DownloadTask>("download-task", ({ payload }) => {
        if (disposed) return;
        if (!removed.current.has(payload.id) && !retired.current.has(payload.generation)) { setRecords((current) => merge(current, payload)); notify(payload); }
      }).then((stop) => { if (disposed) stop(); else stops.push(stop); }),
      listen<string>("download-background-warning", ({ payload }) => { if (!disposed) setBackgroundWarning(payload); }).then((stop) => { if (disposed) stop(); else stops.push(stop); }),
    ]).finally(() => { if (!disposed) setLoading(false); });
    const timer = setInterval(() => { if (!document.hidden && Object.values(tasksRef.current).some(activeDownload)) void reload().catch(() => {}); }, 2000);
    const refresh = () => { if (!document.hidden) void reload().catch(() => {}); };
    document.addEventListener("visibilitychange", refresh);
    return () => { disposed = true; stops.forEach((stop) => stop()); clearInterval(timer); document.removeEventListener("visibilitychange", refresh); };
  }, []);
  async function command(command: string, id: string) {
    mutation.current++;
    setError(null);
    try { const task = await invoke<DownloadTask>(command, { id }); if (!removed.current.has(task.id) && !retired.current.has(task.generation)) { setRecords((current) => merge(current, task)); notify(task); } return task; }
    catch (reason) { setError(String(reason)); throw reason; }
  }
  return {
    tasks: Object.values(records).sort((a, b) => b.createdAtUnixMs - a.createdAtUnixMs || a.id.localeCompare(b.id)), settings, error, loading, backgroundWarning, reload,
    async enqueue(providerId: string, bookId: string, title: string, format: BookFormat) {
      const task = await invoke<DownloadTask>("enqueue_download_book", { providerId, bookId, title, format, downloadDir: null });
      mutation.current++; removed.current.delete(task.id);
      setRecords((current) => merge(current, task)); notify(task); return task;
    },
    pause: (id: string) => command("pause_download", id), resume: (id: string) => command("resume_download", id),
    cancel: (id: string) => command("cancel_download", id), restart: (id: string) => command("restart_download", id),
    async pauseAll() { await invoke("pause_all_downloads"); await reload(); },
    async resumeAll() { for (const task of Object.values(tasksRef.current).filter((item) => ["paused", "failed"].includes(item.state))) await command("resume_download", task.id); },
    async remove(id: string, deleteFile: boolean) { mutation.current++; const generation = tasksRef.current[id]?.generation; await invoke("remove_download", { id, deleteFile }); if (generation) retired.current.add(generation); removed.current.add(id); mutation.current++; setRecords((current) => Object.fromEntries(Object.entries(current).filter(([key]) => key !== id))); },
    async saveSettings(patch: Partial<Settings>) {
      const value = { ...settingsRef.current, ...patch }; const epoch = ++settingsEpoch.current;
      settingsRef.current = value; setSettings(value);
      const save = settingsWrite.current.catch(() => {}).then(() => invoke<Settings>("save_download_settings", { settings: value }));
      settingsWrite.current = save;
      try { await save; } catch (reason) {
        if (epoch === settingsEpoch.current) { const saved = await invoke<Settings>("download_settings"); settingsRef.current = saved; setSettings(saved); }
        throw reason;
      }
    },
  };
}

type Queue = ReturnType<typeof useDownloadQueue>;
type Confirmation = { task: DownloadTask; operation: "restart" | "delete" };
export default function DownloadQueue({ queue, onRead, onManageFile, onFind }: { queue: Queue; onRead: (item: LibraryItem) => void; onManageFile: (item: LibraryItem) => void; onFind: () => void }) {
  const [filter, setFilter] = useState("all");
  const [query, setQuery] = useState("");
  const [managedId, setManagedId] = useState<string | null>(null);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const suppressClick = useRef(false);
  const origin = useRef({ x: 0, y: 0 });
  const managed = queue.tasks.find((task) => task.id === managedId);
  const active = queue.tasks.filter(activeDownload).length;
  const resumable = queue.tasks.filter((task) => ["paused", "failed"].includes(task.state)).length;
  function cancelHold() { if (timer.current) clearTimeout(timer.current); timer.current = null; }
  function hold(event: PointerEvent<HTMLElement>, task: DownloadTask) {
    if ((event.target as Element).closest("button, input, select, a, summary")) return;
    cancelHold(); origin.current = { x: event.clientX, y: event.clientY };
    timer.current = setTimeout(() => { suppressClick.current = true; setManagedId(task.id); timer.current = null; }, 550);
  }
  useEffect(() => () => cancelHold(), []);
  useEffect(() => {
    const target = window as Window & { luminaDownloadBack?: () => boolean };
    target.luminaDownloadBack = () => { if (confirmation) { setConfirmation(null); return true; } if (managedId) { setManagedId(null); return true; } return false; };
    return () => { delete target.luminaDownloadBack; };
  }, [managedId, confirmation]);
  async function perform(key: string, work: () => Promise<unknown>) { setBusy(key); setError(null); try { await work(); } catch (reason) { setError(String(reason)); } finally { setBusy(null); } }
  async function file(task: DownloadTask, manage: boolean) {
    let items = await invoke<LibraryItem[]>("library_items");
    let item = items.find((item) => item.id === task.libraryId || item.path === task.destination);
    if (!item) { await queue.resume(task.id); items = await invoke<LibraryItem[]>("library_items"); item = items.find((item) => item.id === task.libraryId || item.path === task.destination); }
    if (!item) throw new Error("下载文件已移动或删除，可以重新下载");
    setManagedId(null); if (manage) onManageFile(item); else onRead(item);
  }
  const visible = queue.tasks.filter((task) => matches(task, filter) && task.title.toLowerCase().includes(query.toLowerCase()));
  return <section className="download-queue" aria-label="下载管理" onPointerDownCapture={() => { suppressClick.current = false; }} onClickCapture={(event) => { if (suppressClick.current) { event.preventDefault(); event.stopPropagation(); suppressClick.current = false; } }}>
    <div className="download-toolbar">
      <div className="download-filters" role="tablist" aria-label="下载状态">{filters.map(([value, label]) => { const count = queue.tasks.filter((task) => matches(task, value)).length; return <button key={value} role="tab" aria-selected={filter === value} onClick={() => setFilter(value)}>{label}{count ? <span>{count}</span> : null}</button>; })}</div>
      <div className="download-toolbar-actions"><button className="text-button" disabled={!active || Boolean(busy)} onClick={() => void perform("all", queue.pauseAll)}><Pause size={16} />暂停全部</button><button className="text-button" disabled={!resumable || Boolean(busy)} onClick={() => void perform("all", queue.resumeAll)}><Play size={16} />继续未完成</button><button className="icon-button" aria-label="刷新下载任务" onClick={() => void perform("refresh", queue.reload)}><RefreshCw size={17} /></button></div>
    </div>
    <details className="download-preferences"><summary>下载设置 · 每本最多 {queue.settings.connections} 个连接，同时 {queue.settings.simultaneousBooks} 本</summary>
      <div className="download-preference-fields">
        <label>单文件连接数<select aria-label="单文件连接数" value={queue.settings.connections} onChange={(event) => { const connections = Number(event.target.value); void perform("settings", () => queue.saveSettings({ connections })); }}>{[1, 2, 4, 8].map((value) => <option key={value} value={value}>{value} 个连接</option>)}</select></label>
        <label>同时下载书籍<select aria-label="同时下载书籍" value={queue.settings.simultaneousBooks} onChange={(event) => { const simultaneousBooks = Number(event.target.value); void perform("settings", () => queue.saveSettings({ simultaneousBooks })); }}>{[1, 2, 3].map((value) => <option key={value} value={value}>{value} 本</option>)}</select></label>
      </div><p>新设置用于之后开始或继续的任务。能否分段由服务器决定，任务会显示实际连接数。</p>
    </details>
    {queue.tasks.length > 4 ? <input className="download-search" aria-label="搜索下载任务" placeholder="搜索下载任务" value={query} onChange={(event) => setQuery(event.target.value)} /> : null}
    {queue.backgroundWarning ? <p className="network-feedback" role="status">{queue.backgroundWarning}</p> : null}
    {error || queue.error ? <p className="network-feedback error" role="alert">{error || queue.error}</p> : null}
    {queue.loading ? <p role="status">正在读取已保存的下载任务…</p> : null}
    <div className="download-list">{visible.map((task) => {
      const percentage = task.totalBytes ? Math.min(100, Math.round(task.downloadedBytes / task.totalBytes * 100)) : task.state === "completed" ? 100 : 0;
      return <article className="download-item" key={task.id} data-state={task.state} onPointerDown={(event) => hold(event, task)} onPointerMove={(event) => { if (Math.hypot(event.clientX - origin.current.x, event.clientY - origin.current.y) > 12) cancelHold(); }} onPointerUp={cancelHold} onPointerCancel={cancelHold} onContextMenu={(event) => { event.preventDefault(); cancelHold(); setManagedId(task.id); }}>
        <div className="download-thumb" aria-hidden="true"><BookCover title={task.title} libraryId={task.state === "completed" ? task.libraryId : null} format={task.format} /></div><div className="download-copy"><div className="download-heading"><h3>{task.title}</h3><span className={`download-state ${task.state}`}>{labels[task.state]}</span></div><p className="download-meta">{task.format.toUpperCase()} · {bytes(task.downloadedBytes)}{task.totalBytes ? ` / ${bytes(task.totalBytes)}` : ""}{task.totalBytes && task.state !== "completed" ? ` · ${percentage}%` : ""}{task.bytesPerSecond > 0 && task.state === "downloading" ? ` · ${bytes(task.bytesPerSecond)}/s` : ""}</p>
          <div className={`progress-track ${!task.totalBytes && activeDownload(task) ? "indeterminate" : ""}`} role="progressbar" aria-label={task.title} aria-valuenow={task.totalBytes ? percentage : undefined} aria-valuemin={0} aria-valuemax={100}><span style={{ width: `${percentage}%` }} /></div>
          <p className="download-transport">{task.state === "completed" ? `峰值 ${task.peakConnections || 1} 个连接` : `${task.activeConnections} 个连接 · 上限 ${task.connections}`}{task.supportsResume === true ? " · 支持续传" : task.supportsResume === false ? " · 当前服务器使用单连接" : ""}{task.retryCount > 0 ? ` · 已自动重试 ${task.retryCount} 次` : ""}</p>
          {task.error ? <p className="download-error-message" role="status">{task.error}</p> : task.note ? <p className="download-note">{task.note}</p> : null}
          <div className="download-row-actions">{task.state === "completed" ? <button className="secondary-button" onClick={() => void perform(task.id, () => file(task, false))}><BookOpen size={18} />阅读</button> : activeDownload(task) ? <button className="secondary-button" disabled={task.state === "pausing" || task.state === "verifying" || busy === task.id} onClick={() => void perform(task.id, () => queue.pause(task.id))}><Pause size={18} />暂停</button> : <button className="primary-button" aria-label={`继续下载 ${task.title}`} disabled={busy === task.id} onClick={() => void perform(task.id, () => queue.resume(task.id))}><Play size={18} />{task.supportsResume === false ? "重新尝试" : "继续下载"}</button>}<button className="secondary-button" aria-label={`管理下载 ${task.title}`} onClick={() => setManagedId(task.id)}><Ellipsis size={18} />管理</button></div>
          {task.errorDetails ? <details className="download-error-details"><summary>错误详情</summary><p>{task.errorDetails}</p></details> : null}
        </div>
      </article>;
    })}</div>
    {!queue.loading && !visible.length ? <div className="empty-state"><div className="empty-icon" aria-hidden="true"><Download size={24} strokeWidth={1.6} /></div><h2>{queue.tasks.length ? "没有符合条件的任务" : "还没有下载任务"}</h2><p>在书籍详情里选择格式下载。暂停、断网或退出应用后，进度都会留在这里。</p>{!queue.tasks.length ? <div><button className="primary-button" onClick={onFind}>去找书</button></div> : null}</div> : null}
    <Dialog.Root open={Boolean(managed)} onOpenChange={(open) => { if (!open) setManagedId(null); }}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="book-dialog download-task-dialog"><Dialog.Close className="icon-button dialog-close" aria-label="关闭任务管理"><X size={20} /></Dialog.Close><Dialog.Title>{managed?.title || "下载任务"}</Dialog.Title><Dialog.Description className="settings-hint">{managed ? `${labels[managed.state]} · ${bytes(managed.downloadedBytes)}${managed.totalBytes ? ` / ${bytes(managed.totalBytes)}` : ""}` : ""}</Dialog.Description>{managed ? <div className="file-action-list">
      {managed.state === "completed" ? <><button className="settings-row" onClick={() => void perform(managed.id, () => file(managed, false))}><BookOpen size={20} strokeWidth={1.8} /><span><strong>阅读文件</strong><small>内置阅读或交给外部应用</small></span></button><button className="settings-row" onClick={() => void perform(managed.id, () => file(managed, true))}><Ellipsis size={20} strokeWidth={1.8} /><span><strong>文件管理</strong><small>导出、分享、外部打开与删除文件</small></span></button></> : activeDownload(managed) ? <><button className="settings-row" disabled={managed.state === "pausing"} onClick={() => void perform(managed.id, () => queue.pause(managed.id))}><Pause size={20} strokeWidth={1.8} /><span><strong>暂停并保留进度</strong><small>保存分段位置，之后继续下载</small></span></button><button className="settings-row" onClick={() => void perform(managed.id, () => queue.cancel(managed.id))}><X size={20} strokeWidth={1.8} /><span><strong>取消本次下载</strong><small>保留部分文件，可以继续或删除</small></span></button></> : <button className="settings-row" onClick={() => void perform(managed.id, () => queue.resume(managed.id))}><Play size={20} strokeWidth={1.8} /><span><strong>继续下载</strong><small>重新获取有效链接，按服务器能力续传</small></span></button>}
      <button className="settings-row" onClick={() => { setConfirmation({ task: managed, operation: "restart" }); setManagedId(null); }}><RefreshCw size={20} strokeWidth={1.8} /><span><strong>从头重新下载</strong><small>清除未完成的进度，重新取得文件</small></span></button>
      <button className="settings-row" onClick={() => void perform(managed.id, async () => { await queue.remove(managed.id, false); setManagedId(null); })}><Trash2 size={20} strokeWidth={1.8} /><span><strong>移除下载记录</strong><small>书架和已保存的文件保留</small></span></button>
      {managed.state !== "completed" && !managed.libraryId ? <button className="settings-row danger" onClick={() => { setConfirmation({ task: managed, operation: "delete" }); setManagedId(null); }}><Trash2 size={20} strokeWidth={1.8} /><span><strong>删除任务与未完成文件</strong><small>清除这一笔下载的部分文件</small></span></button> : null}
    </div> : null}{error ? <p className="network-feedback error" role="alert">{error}</p> : null}</Dialog.Content></Dialog.Portal></Dialog.Root>
    <Dialog.Root open={Boolean(confirmation)} onOpenChange={(open) => { if (!open) setConfirmation(null); }}><Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="book-dialog"><Dialog.Title>{confirmation?.operation === "restart" ? "从头重新下载？" : "删除未完成下载？"}</Dialog.Title><Dialog.Description>“{confirmation?.task.title}”{confirmation?.operation === "restart" ? "将重新下载，当前未完成的进度会清除。已有完成文件会保留备份。" : "的未完成文件和记录会删除，书架和收藏保留。"}</Dialog.Description><div className="dialog-actions"><Dialog.Close className="secondary-button">保留</Dialog.Close><button className="danger-button" disabled={Boolean(busy)} onClick={() => { if (!confirmation) return; const selected = confirmation; void perform(selected.task.id, async () => { if (selected.operation === "restart") await queue.restart(selected.task.id); else await queue.remove(selected.task.id, true); setConfirmation(null); }); }}>{confirmation?.operation === "restart" ? "重新下载" : "确认删除"}</button></div></Dialog.Content></Dialog.Portal></Dialog.Root>
  </section>;
}
