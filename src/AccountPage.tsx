import { invoke } from "@tauri-apps/api/core";
import { UserRound, LogOut, RefreshCw, AlertCircle, Check } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";

export type ZLibraryAccountStatus = { signedIn: boolean; origin: string; originMode: "automatic" | "manual" };
type Profile = { downloadsToday: number; downloadsLimit: number; downloadsRemaining: number };
type History = { items: Array<{ book: { id: string; title: string; authors: string[] }; downloadedAt?: string }>; page: number; hasNext: boolean };

export default function AccountPage({ initialStatus, onStatusChange }: { initialStatus: ZLibraryAccountStatus | null; onStatusChange: (status: ZLibraryAccountStatus) => void }) {
  const [status, setStatus] = useState(initialStatus);
  const [mode, setMode] = useState<"automatic" | "manual">(initialStatus?.originMode || "automatic");
  const [origin, setOrigin] = useState(initialStatus?.origin || "");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [profile, setProfile] = useState<Profile | null>(null);
  const [history, setHistory] = useState<History | null>(null);
  function commit(next: ZLibraryAccountStatus) { setStatus(next); setMode(next.originMode); setOrigin(next.origin); onStatusChange(next); }
  async function loadData(page = 1) {
    const results = await Promise.allSettled([invoke<Profile>("zlibrary_profile"), invoke<History>("zlibrary_history", { page })]);
    if (results[0].status === "fulfilled") setProfile(results[0].value);
    if (results[1].status === "fulfilled") setHistory(results[1].value);
    const failed = results.find((result) => result.status === "rejected");
    if (failed?.status === "rejected") setError(String(failed.reason));
  }
  useEffect(() => {
    let active = true;
    void (async () => { try { const next = initialStatus || await invoke<ZLibraryAccountStatus>("zlibrary_status"); if (!active) return; commit(next); if (next.signedIn) await loadData(); } catch (reason) { if (active) setError(String(reason)); } })();
    return () => { active = false; };
  }, []);
  async function login(event: FormEvent) {
    event.preventDefault(); if (!email.trim() || !password) return;
    setBusy(true); setError(null);
    try {
      const result = await invoke<{ status: ZLibraryAccountStatus; profile?: Profile }>("zlibrary_login", { email: email.trim(), password, automatic: mode === "automatic", origin: mode === "manual" ? origin.trim() : null });
      commit(result.status); if (result.profile) setProfile(result.profile); await loadData();
    } catch (reason) { setError(String(reason)); }
    finally { setPassword(""); setBusy(false); }
  }
  async function logout() { setBusy(true); try { commit(await invoke<ZLibraryAccountStatus>("zlibrary_logout")); setProfile(null); setHistory(null); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  async function refresh(page = 1) { setBusy(true); setError(null); try { await loadData(page); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  return <div className="account-page">
    <div className="account-heading"><UserRound size={30} strokeWidth={1.5} /><div><h2>Z-Library</h2><p>{status?.signedIn ? "账户已连接" : "登录你的书源账户"}</p></div>{status?.signedIn ? <span className="account-connected"><Check size={17} />已登录</span> : null}</div>
    {error ? <div className="account-error" role="alert"><AlertCircle size={19} /><span>{error}</span></div> : null}
    {!status?.signedIn ? <form onSubmit={login} className="account-form">
      <label className="network-field"><span>邮箱</span><input type="email" autoComplete="username" value={email} onChange={(event) => setEmail(event.target.value)} placeholder="你的账户邮箱" required /></label>
      <label className="network-field"><span>密码</span><input type="password" autoComplete="current-password" value={password} onChange={(event) => setPassword(event.target.value)} placeholder="你的账户密码" required /></label>
      <label className="network-field"><span>站点地址</span><select value={mode} onChange={(event) => setMode(event.target.value as "automatic" | "manual")}><option value="automatic">自动检测可用站点</option><option value="manual">使用我指定的站点</option></select></label>
      {mode === "manual" ? <label className="network-field"><span>指定站点</span><input type="url" spellCheck={false} autoCapitalize="none" value={origin} onChange={(event) => setOrigin(event.target.value)} placeholder="https://你的站点域名" required /></label> : null}
      <button className="primary-button full-width" disabled={busy || !email.trim() || !password}>{busy ? "正在连接账户…" : "登录"}</button>
      <p className="account-help">站点无法连接时，可以在设置中检查网络或更换站点地址。</p>
    </form> : <>
      <div className="account-site"><span>当前站点</span><strong>{status.origin}</strong></div>
      {profile ? <div className="account-quota"><span>今日可下载</span><strong>{profile.downloadsRemaining}<small> / {profile.downloadsLimit} 本</small></strong></div> : null}
      <div className="account-buttons"><button className="secondary-button" disabled={busy} onClick={() => void refresh()}><RefreshCw size={18} />刷新账户</button><button className="text-button" disabled={busy} onClick={() => void logout()}><LogOut size={18} />退出登录</button></div>
      <section className="account-history"><h3>下载记录</h3>{history?.items.length ? history.items.map((item) => <article key={`${item.book.id}:${item.downloadedAt}`}><h4>{item.book.title}</h4><p>{item.book.authors.join(" · ") || "作者未提供"}{item.downloadedAt ? ` · ${item.downloadedAt}` : ""}</p></article>) : <p>暂无下载记录</p>}{history ? <div className="account-buttons"><button className="secondary-button" disabled={busy || history.page <= 1} onClick={() => void refresh(history.page - 1)}>上一页</button><button className="secondary-button" disabled={busy || !history.hasNext} onClick={() => void refresh(history.page + 1)}>下一页</button></div> : null}</section>
    </>}
  </div>;
}
