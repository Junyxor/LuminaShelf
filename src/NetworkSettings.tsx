import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type ResolverMode = "system" | "app_hosts" | "doh" | "dot" | "custom_dns";

type ResolverPolicy = {
  mode: ResolverMode;
  appHosts: Record<string, string[]>;
  upstreamIp?: string | null;
  serverName?: string | null;
  dohPath?: string | null;
  fallbackToSystem: boolean;
  timeoutMs: number;
};

type ResolveResult = {
  host: string;
  addresses: string[];
  elapsedMs: number;
  viaAppHosts: boolean;
  usedFallback: boolean;
  mode: ResolverMode;
};

const STORAGE_KEY = "luminashelf.resolverPolicy";

const systemPolicy: ResolverPolicy = {
  mode: "system",
  appHosts: {},
  upstreamIp: null,
  serverName: null,
  dohPath: "/dns-query",
  fallbackToSystem: true,
  timeoutMs: 3000,
};

const modeLabels: Record<ResolverMode, string> = {
  system: "系统 DNS",
  app_hosts: "系统 DNS + App Hosts",
  custom_dns: "自定义 DNS",
  dot: "DNS over TLS",
  doh: "DNS over HTTPS",
};

function formatHostOverrides(entries: Record<string, string[]>) {
  return Object.entries(entries)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([host, addresses]) => `${host} = ${addresses.join(", ")}`)
    .join("\n");
}

function parseHostOverrides(value: string): Record<string, string[]> {
  const entries: Record<string, string[]> = {};
  value.split(/\r?\n/).forEach((rawLine, index) => {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) return;
    const separator = line.indexOf("=");
    if (separator <= 0) {
      throw new Error(`App Hosts 第 ${index + 1} 行缺少 =`);
    }
    const host = line.slice(0, separator).trim();
    const addresses = line
      .slice(separator + 1)
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
    if (!host || addresses.length === 0) {
      throw new Error(`App Hosts 第 ${index + 1} 行格式不完整`);
    }
    entries[host] = addresses;
  });
  return entries;
}

function needsUpstream(mode: ResolverMode) {
  return mode === "custom_dns" || mode === "dot" || mode === "doh";
}

function needsServerName(mode: ResolverMode) {
  return mode === "dot" || mode === "doh";
}

function loadStoredPolicy(): ResolverPolicy | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) as ResolverPolicy : null;
  } catch {
    return null;
  }
}

export default function NetworkSettings({ networkStack }: { networkStack: string }) {
  const [draft, setDraft] = useState<ResolverPolicy | null>(null);
  const [hostOverrides, setHostOverrides] = useState("");
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [testHost, setTestHost] = useState("example.com");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<ResolveResult | null>(null);

  useEffect(() => {
    const stored = loadStoredPolicy();
    const initialize = stored
      ? invoke<ResolverPolicy>("set_resolver_policy", { policy: stored })
      : invoke<ResolverPolicy>("resolver_policy");

    initialize
      .then((policy) => {
        setDraft(policy);
        setHostOverrides(formatHostOverrides(policy.appHosts));
      })
      .catch(async (reason) => {
        localStorage.removeItem(STORAGE_KEY);
        setError(`保存的网络策略无法应用：${String(reason)}`);
        try {
          const policy = await invoke<ResolverPolicy>("resolver_policy");
          setDraft(policy);
          setHostOverrides(formatHostOverrides(policy.appHosts));
        } catch (fallbackReason) {
          setError(String(fallbackReason));
        }
      });
  }, []);

  async function applyPolicy(policy: ResolverPolicy, successMessage: string) {
    const saved = await invoke<ResolverPolicy>("set_resolver_policy", { policy });
    localStorage.setItem(STORAGE_KEY, JSON.stringify(saved));
    setDraft(saved);
    setHostOverrides(formatHostOverrides(saved.appHosts));
    setMessage(successMessage);
    setTestResult(null);
  }

  async function save() {
    if (!draft) return;
    setSaving(true);
    setError(null);
    setMessage(null);
    try {
      const policy: ResolverPolicy = {
        ...draft,
        appHosts: parseHostOverrides(hostOverrides),
        upstreamIp: draft.upstreamIp?.trim() || null,
        serverName: draft.serverName?.trim() || null,
        dohPath: draft.dohPath?.trim() || "/dns-query",
      };
      await applyPolicy(policy, "网络策略已应用并保存");
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  }

  async function resetNetwork() {
    setSaving(true);
    setError(null);
    setMessage(null);
    try {
      await applyPolicy(systemPolicy, "已恢复系统 DNS 默认策略");
      localStorage.removeItem(STORAGE_KEY);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  async function testResolver() {
    const host = testHost.trim();
    if (!host) return;
    setTesting(true);
    setError(null);
    setTestResult(null);
    try {
      setTestResult(await invoke<ResolveResult>("resolve_host", { host }));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setTesting(false);
    }
  }

  if (!draft) {
    return (
      <article className="setting-card network-settings-card glass">
        <span className="eyebrow">NETWORK</span>
        <h3>网络与解析</h3>
        <p>{error ?? "正在读取 Rust Core 的 ResolverPolicy…"}</p>
      </article>
    );
  }

  return (
    <article className="setting-card network-settings-card glass">
      <div className="setting-card-heading">
        <div>
          <span className="eyebrow">NETWORK</span>
          <h3>网络与解析</h3>
        </div>
        <span className="status-dot">{modeLabels[draft.mode]}</span>
      </div>

      <div className="network-settings-layout">
        <div className="network-form">
          <label>
            <span>解析模式<small>直接切换 Core 使用的 DNS 传输</small></span>
            <select value={draft.mode} onChange={(event) => setDraft((current) => current ? { ...current, mode: event.target.value as ResolverMode } : current)}>
              <option value="system">系统 DNS</option>
              <option value="app_hosts">系统 DNS + App Hosts</option>
              <option value="custom_dns">自定义 DNS</option>
              <option value="dot">DNS over TLS</option>
              <option value="doh">DNS over HTTPS</option>
            </select>
          </label>

          {needsUpstream(draft.mode) ? (
            <label className="stacked">
              <span>上游 IP<small>例如 1.1.1.1；Core 会直接连接这个地址</small></span>
              <input value={draft.upstreamIp ?? ""} onChange={(event) => setDraft((current) => current ? { ...current, upstreamIp: event.target.value } : current)} placeholder="1.1.1.1" spellCheck={false} />
            </label>
          ) : null}

          {needsServerName(draft.mode) ? (
            <label className="stacked">
              <span>TLS Server Name<small>用于证书验证，不会关闭 TLS 主机名校验</small></span>
              <input value={draft.serverName ?? ""} onChange={(event) => setDraft((current) => current ? { ...current, serverName: event.target.value } : current)} placeholder="cloudflare-dns.com" spellCheck={false} />
            </label>
          ) : null}

          {draft.mode === "doh" ? (
            <label className="stacked">
              <span>DoH Path<small>通常保持 /dns-query</small></span>
              <input value={draft.dohPath ?? "/dns-query"} onChange={(event) => setDraft((current) => current ? { ...current, dohPath: event.target.value } : current)} placeholder="/dns-query" spellCheck={false} />
            </label>
          ) : null}

          <label>
            <span>解析超时<small>单次 DNS 查询的最长等待时间</small></span>
            <select value={draft.timeoutMs} onChange={(event) => setDraft((current) => current ? { ...current, timeoutMs: Number(event.target.value) } : current)}>
              <option value={1000}>1 秒</option>
              <option value={3000}>3 秒</option>
              <option value={5000}>5 秒</option>
              <option value={10000}>10 秒</option>
            </select>
          </label>

          {needsUpstream(draft.mode) ? (
            <label className="toggle-row">
              <span>失败时回退系统 DNS<small>自定义 / DoT / DoH 失败时继续尝试系统解析</small></span>
              <button type="button" className={`toggle ${draft.fallbackToSystem ? "on" : ""}`} onClick={() => setDraft((current) => current ? { ...current, fallbackToSystem: !current.fallbackToSystem } : current)} aria-pressed={draft.fallbackToSystem}><i /></button>
            </label>
          ) : null}

          <label className="stacked">
            <span>App Hosts<small>每行：域名 = IP, IP；支持 *.example.com</small></span>
            <textarea value={hostOverrides} onChange={(event) => setHostOverrides(event.target.value)} placeholder={"api.example.com = 203.0.113.10\n*.example.org = 2001:db8::10"} spellCheck={false} />
          </label>

          <div className="network-actions">
            <button className="primary-button" disabled={saving} onClick={() => void save()}>{saving ? "应用中…" : "应用网络策略"}</button>
            <button className="ghost-button" disabled={saving} onClick={() => void resetNetwork()}>恢复系统默认</button>
            <span className="network-stack-label">{networkStack}</span>
          </div>
          {message ? <div className="network-message success">{message}</div> : null}
          {error ? <div className="network-message error">{error}</div> : null}
        </div>

        <div className="resolver-test">
          <span className="eyebrow">LIVE TEST</span>
          <h4>解析测试</h4>
          <p>用当前已经应用的策略解析一个域名，确认地址、耗时和是否发生系统回退。</p>
          <div className="resolver-test-form">
            <input value={testHost} onChange={(event) => setTestHost(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void testResolver(); }} placeholder="example.com" spellCheck={false} />
            <button className="ghost-button" disabled={testing || !testHost.trim()} onClick={() => void testResolver()}>{testing ? "测试中…" : "测试"}</button>
          </div>
          {testResult ? (
            <div className="resolver-result">
              <div><span>Host</span><strong>{testResult.host}</strong></div>
              <div><span>地址</span><strong>{testResult.addresses.join(" · ") || "无"}</strong></div>
              <div><span>耗时</span><strong>{testResult.elapsedMs.toFixed(1)} ms</strong></div>
              <div><span>路径</span><strong>{testResult.viaAppHosts ? "App Hosts" : testResult.usedFallback ? "系统回退" : modeLabels[testResult.mode]}</strong></div>
            </div>
          ) : <div className="resolver-test-empty">保存策略后，可以在这里直接验证实际解析路径。</div>}
        </div>
      </div>
    </article>
  );
}
