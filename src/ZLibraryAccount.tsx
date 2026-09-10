import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState, type FormEvent } from "react";

export type ZLibraryAccountStatus = {
  signedIn: boolean;
  origin: string;
  originMode: "automatic" | "manual";
};

type ZLibraryProfile = {
  downloadsToday: number;
  downloadsLimit: number;
  downloadsRemaining: number;
};

type HistoryBook = {
  id: string;
  title: string;
  authors: string[];
  year?: number | null;
  language?: string | null;
  format?: string | null;
  availableFormats: string[];
  sizeBytes?: number | null;
  coverUrl?: string | null;
};

type ZLibraryHistoryItem = {
  book: HistoryBook;
  downloadedAt?: string | null;
};

type ZLibraryHistoryPage = {
  items: ZLibraryHistoryItem[];
  page: number;
  hasNext: boolean;
};

type ZLibraryLoginResult = {
  status: ZLibraryAccountStatus;
  profile?: ZLibraryProfile | null;
};

function formatBytes(value?: number | null) {
  if (!value || value <= 0) return "—";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 10 || unit === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
}

export default function ZLibraryAccount({
  initialStatus,
  onStatusChange,
}: {
  initialStatus: ZLibraryAccountStatus | null;
  onStatusChange: (status: ZLibraryAccountStatus) => void;
}) {
  const [status, setStatus] = useState<ZLibraryAccountStatus | null>(initialStatus);
  const [originMode, setOriginMode] = useState<"automatic" | "manual">(
    initialStatus?.originMode ?? "automatic",
  );
  const [manualOrigin, setManualOrigin] = useState(initialStatus?.origin ?? "");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [profile, setProfile] = useState<ZLibraryProfile | null>(null);
  const [history, setHistory] = useState<ZLibraryHistoryPage | null>(null);
  const [loading, setLoading] = useState(!initialStatus);
  const [loggingIn, setLoggingIn] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function commitStatus(next: ZLibraryAccountStatus) {
    setStatus(next);
    setOriginMode(next.originMode);
    setManualOrigin(next.origin);
    onStatusChange(next);
  }

  async function loadSignedInData() {
    const [nextProfile, nextHistory] = await Promise.all([
      invoke<ZLibraryProfile>("zlibrary_profile"),
      invoke<ZLibraryHistoryPage>("zlibrary_history", { page: 1 }),
    ]);
    setProfile(nextProfile);
    setHistory(nextHistory);
  }

  useEffect(() => {
    let disposed = false;
    const initialize = async () => {
      setLoading(true);
      setError(null);
      try {
        const nextStatus = initialStatus ?? await invoke<ZLibraryAccountStatus>("zlibrary_status");
        if (disposed) return;
        commitStatus(nextStatus);
        if (nextStatus.signedIn) {
          await loadSignedInData();
        }
      } catch (reason) {
        if (!disposed) setError(String(reason));
      } finally {
        if (!disposed) setLoading(false);
      }
    };
    void initialize();
    return () => { disposed = true; };
  }, []);

  async function login(event: FormEvent) {
    event.preventDefault();
    if (!email.trim() || !password) return;
    setLoggingIn(true);
    setError(null);
    setProfile(null);
    setHistory(null);
    try {
      const result = await invoke<ZLibraryLoginResult>("zlibrary_login", {
        email: email.trim(),
        password,
        automatic: originMode === "automatic",
        origin: originMode === "manual" ? manualOrigin.trim() : null,
      });
      setPassword("");
      commitStatus(result.status);
      if (result.profile) setProfile(result.profile);
      try {
        const nextHistory = await invoke<ZLibraryHistoryPage>("zlibrary_history", { page: 1 });
        setHistory(nextHistory);
        if (!result.profile) {
          setProfile(await invoke<ZLibraryProfile>("zlibrary_profile"));
        }
      } catch (reason) {
        setError(`登录成功，但账户数据加载失败：${String(reason)}`);
      }
    } catch (reason) {
      setPassword("");
      setError(String(reason));
    } finally {
      setLoggingIn(false);
    }
  }

  async function logout() {
    setLoading(true);
    setError(null);
    try {
      const next = await invoke<ZLibraryAccountStatus>("zlibrary_logout");
      commitStatus(next);
      setEmail("");
      setPassword("");
      setProfile(null);
      setHistory(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      await loadSignedInData();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }

  async function loadHistoryPage(page: number) {
    if (page < 1) return;
    setLoading(true);
    setError(null);
    try {
      setHistory(await invoke<ZLibraryHistoryPage>("zlibrary_history", { page }));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }

  if (!status || loading && !status) {
    return (
      <section className="account-shell glass">
        <span className="eyebrow">Z-LIBRARY EAPI</span>
        <h2>正在读取账户状态…</h2>
        {error ? <div className="account-message error">{error}</div> : null}
      </section>
    );
  }

  if (!status.signedIn) {
    return (
      <div className="account-layout signed-out">
        <section className="account-shell glass">
          <div className="account-title-row">
            <div>
              <span className="eyebrow">Z-LIBRARY EAPI</span>
              <h2>连接你的书库账户。</h2>
              <p>自动模式会先用无凭据请求寻找可用 EAPI，确认可用后才提交一次登录。也可以手动固定一个 HTTPS EAPI 地址。</p>
            </div>
            <span className="account-state offline">未登录</span>
          </div>

          <form className="account-login-form" onSubmit={login}>
            <div className="origin-mode-switch">
              <button type="button" className={originMode === "automatic" ? "active" : ""} onClick={() => setOriginMode("automatic")}>
                <strong>自动选择</strong><small>探测健康 EAPI</small>
              </button>
              <button type="button" className={originMode === "manual" ? "active" : ""} onClick={() => setOriginMode("manual")}>
                <strong>手动固定</strong><small>使用指定 HTTPS 地址</small>
              </button>
            </div>

            {originMode === "manual" ? (
              <label className="account-field">
                <span>EAPI Origin</span>
                <input value={manualOrigin} onChange={(event) => setManualOrigin(event.target.value)} placeholder="https://example.com" inputMode="url" spellCheck={false} />
              </label>
            ) : (
              <div className="auto-origin-note">
                <span>上次选择</span><strong>{status.origin}</strong><small>如果已经失效，登录时会重新探测候选。</small>
              </div>
            )}

            <div className="account-credentials">
              <label className="account-field"><span>邮箱</span><input type="email" autoComplete="username" value={email} onChange={(event) => setEmail(event.target.value)} placeholder="name@example.com" /></label>
              <label className="account-field"><span>密码</span><input type="password" autoComplete="current-password" value={password} onChange={(event) => setPassword(event.target.value)} placeholder="••••••••" /></label>
            </div>

            <button className="primary-button account-login-button" disabled={loggingIn || !email.trim() || !password || (originMode === "manual" && !manualOrigin.trim())}>
              {loggingIn ? "正在探测并登录…" : "登录 Z-Library"}
            </button>
          </form>

          {error ? <div className="account-message error">{error}</div> : null}
        </section>

        <aside className="account-security glass">
          <span className="security-mark">◇</span>
          <span className="eyebrow">SESSION SECURITY</span>
          <h3>这一版不保存密码或 Session Key。</h3>
          <p>密码只用于本次登录调用，成功或失败后都会从表单状态清空。`remix-userkey` 只保留在 Rust 进程内存中，因此关闭应用后需要重新登录。</p>
          <div className="security-row"><span>密码落盘</span><strong>否</strong></div>
          <div className="security-row"><span>Session 落盘</span><strong>否</strong></div>
          <div className="security-row"><span>Origin 落盘</span><strong>是 · 非敏感配置</strong></div>
        </aside>
      </div>
    );
  }

  return (
    <div className="account-dashboard">
      <section className="account-shell glass">
        <div className="account-title-row">
          <div>
            <span className="eyebrow">Z-LIBRARY EAPI</span>
            <h2>账户已连接。</h2>
            <p className="account-origin">{status.origin} · {status.originMode === "automatic" ? "自动选择" : "手动固定"}</p>
          </div>
          <div className="account-actions">
            <span className="account-state online">● 已登录</span>
            <button className="ghost-button" disabled={loading} onClick={() => void refresh()}>{loading ? "刷新中…" : "刷新"}</button>
            <button className="ghost-button danger" disabled={loading} onClick={() => void logout()}>退出</button>
          </div>
        </div>

        <div className="quota-grid">
          <article><span>今日已下载</span><strong>{profile?.downloadsToday ?? "—"}</strong></article>
          <article><span>今日额度</span><strong>{profile?.downloadsLimit ?? "—"}</strong></article>
          <article><span>剩余额度</span><strong>{profile?.downloadsRemaining ?? "—"}</strong></article>
        </div>
        {error ? <div className="account-message error">{error}</div> : null}
      </section>

      <section className="account-history glass">
        <div className="account-history-header">
          <div><span className="eyebrow">DOWNLOAD HISTORY</span><h3>下载历史</h3></div>
          <div className="pager">
            <button className="ghost-button" disabled={loading || !history || history.page <= 1} onClick={() => void loadHistoryPage((history?.page ?? 1) - 1)}>上一页</button>
            <span>第 {history?.page ?? 1} 页</span>
            <button className="ghost-button" disabled={loading || !history?.hasNext} onClick={() => void loadHistoryPage((history?.page ?? 1) + 1)}>下一页</button>
          </div>
        </div>

        {history?.items.length ? (
          <div className="account-history-list">
            {history.items.map((item) => (
              <article key={`${item.book.id}:${item.downloadedAt ?? ""}`}>
                {item.book.coverUrl ? <img src={item.book.coverUrl} alt="" loading="lazy" /> : <div className="history-cover-fallback">Z</div>}
                <div className="history-copy">
                  <h4>{item.book.title}</h4>
                  <p>{item.book.authors.join(" · ") || "作者未知"}</p>
                  <small>{item.book.format?.toUpperCase() ?? item.book.availableFormats[0]?.toUpperCase() ?? "—"} · {formatBytes(item.book.sizeBytes)}{item.downloadedAt ? ` · ${item.downloadedAt}` : ""}</small>
                </div>
              </article>
            ))}
          </div>
        ) : (
          <div className="account-history-empty">{loading ? "正在读取历史…" : "这个账户暂时没有返回下载历史。"}</div>
        )}
      </section>
    </div>
  );
}
