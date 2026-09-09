import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";

type CoreStatus = {
  name: string;
  version: string;
  rustCore: boolean;
  networkStack: string;
};

type ProviderDescriptor = {
  id: string;
  name: string;
  capabilities: {
    authenticated: boolean;
    downloadable: boolean;
    searchable: boolean;
    paginated: boolean;
  };
};

type Page = "home" | "search" | "downloads" | "library" | "network" | "account";

const nav: Array<{ id: Page; label: string; glyph: string }> = [
  { id: "home", label: "总览", glyph: "◈" },
  { id: "search", label: "搜索", glyph: "⌕" },
  { id: "downloads", label: "下载", glyph: "⇣" },
  { id: "library", label: "书库", glyph: "▤" },
  { id: "network", label: "网络", glyph: "◎" },
  { id: "account", label: "账户", glyph: "○" },
];

export default function App() {
  const [page, setPage] = useState<Page>("home");
  const [status, setStatus] = useState<CoreStatus | null>(null);
  const [providers, setProviders] = useState<ProviderDescriptor[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([
      invoke<CoreStatus>("core_status"),
      invoke<ProviderDescriptor[]>("provider_descriptors"),
    ])
      .then(([nextStatus, nextProviders]) => {
        setStatus(nextStatus);
        setProviders(nextProviders);
      })
      .catch((reason) => setError(String(reason)));
  }, []);

  const active = useMemo(() => nav.find((item) => item.id === page)!, [page]);

  return (
    <div className="app-shell">
      <aside className="sidebar glass">
        <div className="brand">
          <div className="brand-mark">Z</div>
          <div><strong>LuminaShelf</strong><span>Rust library client</span></div>
        </div>
        <nav>
          {nav.map((item) => (
            <button key={item.id} className={page === item.id ? "active" : ""} onClick={() => setPage(item.id)}>
              <span className="glyph">{item.glyph}</span><span>{item.label}</span>
            </button>
          ))}
        </nav>
        <div className="core-pill"><i />{status?.rustCore ? "Rust Core online" : "Connecting core…"}</div>
      </aside>

      <main>
        <header>
          <div><span className="eyebrow">LUMINASHELF / {active.label}</span><h1>{active.label}</h1></div>
          <div className="version-chip">v{status?.version ?? "0.5.0"}</div>
        </header>

        {error ? <section className="notice error">Core bridge: {error}</section> : null}

        <section className="hero glass">
          <div>
            <span className="eyebrow">HIGH PERFORMANCE E-BOOK CLIENT</span>
            <h2>把网络、检索、下载与书库<br />留在一个高速核心里。</h2>
            <p>UI 只是薄壳。DNS / DoH / DoT、镜像探测、Provider 与分段下载全部由 Rust Core 执行。</p>
          </div>
          <div className="orb"><span>Z</span></div>
        </section>

        <section className="cards">
          <article className="card glass"><span>CORE</span><strong>{status?.networkStack ?? "Rust + Tokio"}</strong><small>统一网络栈</small></article>
          <article className="card glass"><span>PROVIDERS</span><strong>{providers.length}</strong><small>{providers.map((item) => item.name).join(" · ") || "正在加载"}</small></article>
          <article className="card glass"><span>ARCH</span><strong>Thin UI</strong><small>Tauri 2 + React</small></article>
        </section>

        <section className="workspace glass">
          <div className="workspace-title"><div><span className="eyebrow">WORKSPACE</span><h3>{active.label}</h3></div><span className="status-dot">● READY</span></div>
          <p>{page === "home" ? "核心桥已经接通。下一步逐页接入搜索、下载任务、本地书库、镜像测速与账号 session。" : `${active.label} 工作区已经进入统一导航，后续直接绑定对应 Rust command，不再改应用骨架。`}</p>
        </section>
      </main>

      <nav className="mobile-nav glass">
        {nav.map((item) => <button key={item.id} className={page === item.id ? "active" : ""} onClick={() => setPage(item.id)}><span>{item.glyph}</span><small>{item.label}</small></button>)}
      </nav>
    </div>
  );
}
