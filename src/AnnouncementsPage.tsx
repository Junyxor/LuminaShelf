import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { RefreshCw, ExternalLink, Download } from "lucide-react";
type Item = { id: string; title: string; body: string; publishedAt?: string | null; url?: string | null; kind?: string | null };
type Feed = { appItems: Item[]; siteItems: Item[]; siteOrigin: string; fetchedAtUnixMs: number; appSource: string; siteError?: string | null; appError?: string | null };
type Update = { status: string; currentVersion: string; latestVersion?: string | null; releaseUrl?: string | null; notes?: string | null; publishedAt?: string | null; apkUrl?: string | null; apkBytes?: number | null; checkedAt: number };
export default function AnnouncementsPage({ version }: { version: string }) {
  const [feed, setFeed] = useState<Feed | null>(null);
  const [section, setSection] = useState<"app" | "site">("app");
  const [loading, setLoading] = useState(false);
  const [checking, setChecking] = useState(false);
  const [update, setUpdate] = useState<Update | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { let active = true; setLoading(true); void invoke<Feed>("announcement_feeds", { refresh: false }).then((value) => { if (active) setFeed(value); }).catch((reason) => { if (active) setError(String(reason)); }).finally(() => { if (active) setLoading(false); }); return () => { active = false; }; }, []);
  async function refresh() { setLoading(true); setError(null); try { setFeed(await invoke<Feed>("announcement_feeds", { refresh: true })); } catch (reason) { setError(String(reason)); } finally { setLoading(false); } }
  async function check() { setChecking(true); setError(null); try { setUpdate(await invoke<Update>("check_app_update")); } catch (reason) { setError(String(reason)); } finally { setChecking(false); } }
  function open(url: string) { void invoke("open_announcement_link", { url }).catch((reason) => setError(String(reason))); }
  const items = section === "app" ? feed?.appItems : feed?.siteItems;
  return <div className="settings-content announcements-page">
    <section className="update-card"><h2>版本更新</h2><p>当前版本 {version}</p><button className="secondary-button" disabled={checking} onClick={() => void check()}><RefreshCw size={18} />{checking ? "正在检查…" : "检查更新"}</button>
      {update ? <div className="update-result" role="status"><h3>{update.status === "no_releases" ? "项目尚无已发布版本信息" : update.status === "available" ? `发现新版本 ${update.latestVersion}` : `当前已安装 ${version}`}</h3>{update.latestVersion ? <p>最新正式发布：{update.latestVersion}{update.publishedAt ? ` · ${new Date(update.publishedAt).toLocaleDateString()}` : ""}</p> : <p>后续正式安装包将在项目 GitHub Releases 提供。</p>}{update.notes ? <p className="release-notes">{update.notes}</p> : null}{update.releaseUrl ? <button className="text-button" onClick={() => open(update.releaseUrl!)}><Download size={18} />{update.apkUrl ? "前往发布页下载安装包" : "查看发布说明"}</button> : null}</div> : null}
    </section>
    <div className="discovery-heading"><h2>公告栏</h2><button className="icon-button" aria-label="刷新公告" disabled={loading} onClick={() => void refresh()}><RefreshCw size={21} /></button></div>
    <div className="announcement-tabs" role="tablist" aria-label="公告来源"><button className={section === "app" ? "primary-button" : "secondary-button"} role="tab" aria-selected={section === "app"} onClick={() => setSection("app")}>星书公告</button><button className={section === "site" ? "primary-button" : "secondary-button"} role="tab" aria-selected={section === "site"} onClick={() => setSection("site")}>当前书源公告</button></div>
    {error ? <p className="network-feedback error" role="alert">{error}</p> : null}
    {loading && !feed ? <p role="status" className="settings-hint">正在读取公告…</p> : null}
    {feed ? <p className="discovery-source">{section === "app" ? feed.appSource : feed.siteOrigin} · 刷新于 {new Date(feed.fetchedAtUnixMs).toLocaleString()}</p> : null}
    {section === "app" && feed?.appError ? <p className="settings-hint">在线公告暂不可用，显示已保存的公告副本。</p> : null}
    {section === "site" && feed?.siteError ? <p className="network-feedback error" role="status">{feed.siteError}</p> : null}
    {items?.map((item) => <article className="announcement-card" key={item.id}><h3>{item.title}</h3>{item.publishedAt ? <p className="announcement-date">{item.publishedAt}</p> : null}<p>{item.body}</p>{item.url ? <button className="text-button" onClick={() => open(item.url!)}><ExternalLink size={17} />查看原文或活动入口</button> : null}</article>)}
    {section === "site" && feed && !items?.length ? <section className="announcement-card"><h3>暂未取得当前书源公告</h3><p>公告和捐赠活动仅展示网站实际发布的内容。当前没有取得可核实的活动或捐赠链接，可以前往书源首页查看。</p><button className="secondary-button" onClick={() => open(feed.siteOrigin)}><ExternalLink size={18} />打开书源官网</button></section> : null}
  </div>;
}
