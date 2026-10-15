import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { Check, RefreshCw, Plus, Trash2, ChevronDown } from "lucide-react";
import type { ZLibraryAccountStatus } from "./AccountPage";
type Subscription = { id: string; name: string; url: string; enabled: boolean; lastError?: string | null };
type Mirror = { id: string; origin: string; sources: string[]; checkedAt?: number | null; latencyMs?: number | null; available?: boolean | null; error?: string | null; contentAvailable: boolean; redirector: boolean };
type Configuration = { subscriptions: Subscription[]; mirrors: Mirror[]; selectedOrigin: string; lastRefresh?: number | null };
export default function MirrorPreferences({ onSelected }: { onSelected: (status: ZLibraryAccountStatus) => void }) {
  const [config, setConfig] = useState<Configuration | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [origin, setOrigin] = useState("");
  const [sourceName, setSourceName] = useState("");
  const [sourceUrl, setSourceUrl] = useState("");
  const [progress, setProgress] = useState<{ phase: string; done: number; total: number } | null>(null);
  useEffect(() => { let disposed=false;let stop:(()=>void)|undefined;void listen<{phase:string;done:number;total:number}>("mirror-progress",({payload})=>setProgress(payload)).then((unlisten)=>{if(disposed)unlisten();else stop=unlisten;}).catch(()=>{});return()=>{disposed=true;stop?.();};},[]);
  useEffect(() => { let active = true; void invoke<Configuration>("mirror_configuration").then((value) => { if (active) setConfig(value); }).catch((reason) => { if (active) setError(String(reason)); }); return () => { active = false; }; }, []);
  async function perform(work: () => Promise<void>) { setBusy(true); setError(null); try { await work(); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  async function refresh() { await perform(async () => setConfig(await invoke<Configuration>("refresh_book_mirrors"))); }
  async function use(mirror: Mirror) { await perform(async () => { onSelected(await invoke<ZLibraryAccountStatus>("select_book_mirror", { id: mirror.id })); setConfig((value) => value ? { ...value, selectedOrigin: mirror.origin } : value); }); }
  return <section className="settings-section mirror-section" aria-label="书源镜像设置">
    <div className="network-heading"><div><h2>书源镜像</h2><p>Z-Library 的访问地址与发布清单。拉取后逐站检查接口是否可用。</p></div></div>
    <div className="current-network"><span>当前书源地址</span><strong>{config?.selectedOrigin || "正在读取…"}</strong></div>
    <button className="secondary-button network-auto" onClick={() => void refresh()} disabled={busy || !config}><RefreshCw size={19} />{busy ? "正在拉取和检测…" : "拉取并检测镜像"}</button>
    {busy && progress ? <p className="settings-hint" role="status">{progress.phase}{progress.total ? ` ${progress.done} / ${progress.total}` : "…"}</p> : null}
    {config?.lastRefresh ? <p className="settings-hint">上次拉取：{new Date(config.lastRefresh).toLocaleString()}</p> : null}
    {error ? <p className="network-feedback error" role="alert">{error}</p> : null}
    <div className="mirror-list">{config?.mirrors.map((mirror) => <article className="mirror-item" key={mirror.id}>
      <div className="profile-title"><strong>{mirror.origin}</strong>{mirror.origin === config.selectedOrigin ? <span className="profile-current"><Check size={16} />使用中</span> : null}</div>
      <p className={mirror.available === false ? "error-text" : "settings-hint"}>{mirror.available === null || mirror.available === undefined ? "未检测" : mirror.available ? `接口可用 · ${Math.round(mirror.latencyMs || 0)} ms` : "本次连接未通过"}{mirror.redirector ? " · 跳转地址" : ""}{!mirror.contentAvailable ? " · 目录未标记为内容站" : ""}</p>
      {mirror.error ? <p className="settings-hint">{mirror.error}</p> : null}
      <details className="mirror-provenance"><summary>地址来源<ChevronDown size={16} /></summary>{mirror.sources.map((source) => <p key={source}>{source}</p>)}</details>
      <div className="profile-actions"><button className="secondary-button" disabled={busy || mirror.origin === config.selectedOrigin} aria-label={`使用镜像 ${mirror.origin}`} onClick={() => void use(mirror)}>使用此站点</button><button className="icon-button" aria-label={`移除镜像 ${mirror.origin}`} disabled={busy} onClick={() => void perform(async () => setConfig(await invoke<Configuration>("remove_book_mirror", { id: mirror.id })))}><Trash2 size={19} /></button></div>
    </article>)}</div>
    <p className="settings-hint">切换站点会退出当前书源账户。目录条目保留发布来源，是否可用以实际检测为准。</p>
    <details className="network-advanced"><summary>添加镜像与发布源<ChevronDown size={18} /></summary>
      <label className="network-field"><span>镜像站点地址</span><input aria-label="镜像站点地址" type="url" placeholder="https://你的站点" value={origin} onChange={(event) => setOrigin(event.target.value)} /></label>
      <button className="secondary-button" disabled={busy || !origin.trim()} onClick={() => void perform(async () => { setConfig(await invoke<Configuration>("add_book_mirror", { origin })); setOrigin(""); })}><Plus size={18} />添加镜像</button>
      <h3 className="subscription-heading">发布来源</h3>
      {config?.subscriptions.map((source) => <div className="subscription-item" key={source.id}><label className="check-field"><input type="checkbox" checked={source.enabled} disabled={busy} onChange={(event) => { const enabled = event.target.checked; void perform(async () => setConfig(await invoke<Configuration>("save_mirror_subscription", { ...source, enabled }))); }} /><span>{source.name}<small>{source.url}</small></span></label>{source.lastError ? <p className="error-text settings-hint">{source.lastError}</p> : null}</div>)}
      <label className="network-field"><span>来源名称</span><input aria-label="发布源名称" value={sourceName} onChange={(event) => setSourceName(event.target.value)} placeholder="例如：我订阅的镜像清单" /></label>
      <label className="network-field"><span>发布清单地址</span><input aria-label="发布清单地址" type="url" value={sourceUrl} onChange={(event) => setSourceUrl(event.target.value)} placeholder="GitHub 仓库、清单文件或域名目录 HTTPS 地址" /></label>
      <button className="secondary-button" disabled={busy || !sourceName.trim() || !sourceUrl.trim()} onClick={() => void perform(async () => { setConfig(await invoke<Configuration>("save_mirror_subscription", { id: null, name: sourceName, url: sourceUrl, enabled: true })); setSourceName(""); setSourceUrl(""); })}>保存发布源</button>
    </details>
  </section>;
}
