import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { CheckCircle2, AlertCircle, ChevronDown, Wifi, ListChecks, RotateCcw } from "lucide-react";
import { cleanPolicy, defaultPolicy, dnsPresets, formatHosts, type ResolverMode, type ResolverPolicy } from "./network-policy";

type Report = { host: string; dns?: { addresses: string[]; elapsedMs: number; usedFallback: boolean; viaAppHosts: boolean }; dnsError?: string; httpStatus?: number; httpError?: string; elapsedMs: number; usable: boolean; viaProxy: boolean };
export default function NetworkPreferences() {
  const [draft, setDraft] = useState<ResolverPolicy>(defaultPolicy);
  const [hosts, setHosts] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState<"save" | "test" | "auto" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [testHost, setTestHost] = useState("gutendex.com");
  const [report, setReport] = useState<Report | null>(null);
  const [dirty, setDirty] = useState(false);
  useEffect(() => { let active = true; void invoke<ResolverPolicy>("resolver_policy").then((policy) => { if (active) { adopt(policy); setLoaded(true); } }).catch((reason) => { if (active) setError(String(reason)); }); return () => { active = false; }; }, []);
  function adopt(policy: ResolverPolicy) {
    if (policy.mode === "doh" && !policy.dohUrl && policy.serverName) policy = { ...policy, dohUrl: `https://${policy.serverName}${policy.dohPath || "/dns-query"}` };
    setDraft({ ...defaultPolicy, ...policy }); setHosts(formatHosts(policy.appHosts)); setDirty(false);
  }
  function change(patch: Partial<ResolverPolicy>) { setDraft((current) => ({ ...current, ...patch })); setDirty(true); setMessage(null); setReport(null); }
  async function save() {
    setBusy("save"); setError(null); setMessage(null);
    try { const candidate = cleanPolicy(draft, hosts); adopt(await invoke<ResolverPolicy>("set_resolver_policy", { policy: candidate })); setMessage("已保存并应用到搜索、下载和封面连接"); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setBusy(null); }
  }
  async function test() {
    setBusy("test"); setError(null); setReport(null); setMessage(null);
    try { setReport(await invoke<Report>("test_network", { policy: cleanPolicy(draft, hosts), host: testHost.trim() })); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setBusy(null); }
  }
  async function auto() {
    setBusy("auto"); setError(null); setMessage(null); setReport(null);
    try { const result = await invoke<{ policy: ResolverPolicy; report: Report; label: string }>("optimize_network"); adopt(result.policy); setReport(result.report); setMessage(`已选用 ${result.label}，实际书源连接正常`); }
    catch (reason) { setError(String(reason)); }
    finally { setBusy(null); }
  }
  async function reset() {
    setBusy("save"); setError(null); setReport(null);
    try { adopt(await invoke<ResolverPolicy>("set_resolver_policy", { policy: defaultPolicy })); setMessage("已恢复系统网络"); }
    catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  }
  return <section className="settings-section network-section" aria-label="网络设置">
    <div className="network-heading"><div><h2>网络连接</h2><p>选择解析方式，也可以使用自己的加密 DNS 服务。</p></div><Wifi size={24} strokeWidth={1.6} /></div>
    <button className="secondary-button network-auto" onClick={() => void auto()} disabled={!loaded || Boolean(busy)}><ListChecks size={19} />{busy === "auto" ? "正在测试可用连接…" : "自动检测并选择"}</button>
    <label className="network-field"><span id="resolver-mode-label">DNS 解析方式</span><select aria-labelledby="resolver-mode-label" value={draft.mode} onChange={(event) => {
      const mode = event.target.value as ResolverMode;
      change({ mode, ...(mode === "doh" ? { dohUrl: draft.dohUrl || dnsPresets[0].url, upstreamIp: null, serverName: null, upstreamPort: null } : {}) });
    }}><option value="system">系统 DNS</option><option value="doh">加密 DNS（DoH）</option><option value="custom_dns">自定义普通 DNS</option><option value="dot">加密 DNS（DoT）</option><option value="app_hosts">系统 DNS 与 Hosts</option></select></label>
    {draft.mode === "doh" ? <>
      <label className="network-field"><span id="doh-url-label">DoH 地址</span><input aria-labelledby="doh-url-label" aria-describedby="doh-url-help" type="url" autoCapitalize="none" autoCorrect="off" spellCheck={false} placeholder="https://你的域名/dns-query" value={draft.dohUrl || ""} onChange={(event) => change({ dohUrl: event.target.value })} /><small id="doh-url-help">完整 HTTPS 地址，支持自定义端口、路径和查询参数。</small></label>
      <div className="dns-presets" role="group" aria-label="公共 DoH 服务">{dnsPresets.map((preset) => <button key={preset.name} className={draft.dohUrl === preset.url ? "selected" : ""} onClick={() => change({ dohUrl: preset.url, upstreamIp: null, serverName: null, upstreamPort: null })}>{preset.name}</button>)}</div>
    </> : null}
    {draft.mode === "custom_dns" || draft.mode === "dot" ? <label className="network-field"><span>DNS 服务器 IP</span><input spellCheck={false} placeholder="223.5.5.5" value={draft.upstreamIp || ""} onChange={(event) => change({ upstreamIp: event.target.value })} /></label> : null}
    {draft.mode === "dot" ? <label className="network-field"><span>DoT 服务域名</span><input spellCheck={false} placeholder="dns.alidns.com" value={draft.serverName || ""} onChange={(event) => change({ serverName: event.target.value })} /><small>用于验证服务的 HTTPS/TLS 证书。</small></label> : null}
    {draft.mode !== "system" && draft.mode !== "app_hosts" ? <label className="check-field"><input type="checkbox" checked={draft.fallbackToSystem} onChange={(event) => change({ fallbackToSystem: event.target.checked })} /><span>解析失败时使用系统 DNS<small>测试结果会标明是否发生回退。</small></span></label> : null}
    <details className="network-advanced"><summary>高级设置<ChevronDown size={18} /></summary>
      {draft.mode === "doh" ? <label className="network-field"><span>引导 IP（选填）</span><input spellCheck={false} placeholder="留空自动解析 DoH 服务域名" value={draft.upstreamIp || ""} onChange={(event) => change({ upstreamIp: event.target.value })} /><small>服务域名无法解析时可填写 IP，证书仍按地址中的域名验证。</small></label> : null}
      {draft.mode === "custom_dns" || draft.mode === "dot" ? <label className="network-field"><span>DNS 端口（选填）</span><input type="number" min={1} max={65535} placeholder={draft.mode === "dot" ? "853" : "53"} value={draft.upstreamPort || ""} onChange={(event) => change({ upstreamPort: event.target.value ? Number(event.target.value) : null })} /></label> : null}
      <label className="network-field"><span>解析超时</span><select value={draft.timeoutMs} onChange={(event) => change({ timeoutMs: Number(event.target.value) })}>{[3000, 5000, 10000, 15000].map((value) => <option key={value} value={value}>{value / 1000} 秒</option>)}</select></label>
      <label className="network-field"><span>代理地址（选填）</span><input type="url" autoCapitalize="none" spellCheck={false} placeholder="http://192.168.1.100:7890" value={draft.proxyUrl || ""} onChange={(event) => change({ proxyUrl: event.target.value })} /><small>支持 HTTP、HTTPS、SOCKS5；搜索、下载、封面和 DoH 请求使用此代理。</small></label>
      <label className="network-field"><span>应用内 Hosts</span><textarea rows={5} spellCheck={false} placeholder={"203.0.113.10 books.example.org\n2001:db8::10 *.example.org"} value={hosts} onChange={(event) => { setHosts(event.target.value); setDirty(true); setReport(null); }} /><small>支持常见的“IP 域名”格式，也兼容“域名 = IP”。</small></label>
    </details>
    <div className="network-actions"><button className="primary-button" disabled={!loaded || Boolean(busy)} onClick={() => void save()}>{busy === "save" ? "正在应用" : "保存并应用"}</button><button className="text-button" disabled={Boolean(busy)} onClick={() => void reset()}><RotateCcw size={18} />恢复默认</button>{dirty ? <span>修改尚未保存</span> : null}</div>
    {message ? <p className="network-feedback success" role="status"><CheckCircle2 size={19} />{message}</p> : null}
    {error ? <p className="network-feedback error" role="alert"><AlertCircle size={19} />{error}</p> : null}
    <div className="network-test"><h3>测试连接</h3><p>测试上方填写的配置，检查域名解析和实际 HTTPS 连接。</p><div className="network-test-controls"><input aria-label="测试域名" placeholder="gutendex.com" value={testHost} onChange={(event) => setTestHost(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !busy) void test(); }} /><button className="secondary-button" onClick={() => void test()} disabled={Boolean(busy) || !testHost.trim()}>{busy === "test" ? "测试中" : "测试配置"}</button></div>
      {report ? <div className="network-report" aria-live="polite"><p className={report.usable ? "report-good" : "error-text"}>{report.usable ? <CheckCircle2 size={19} /> : <AlertCircle size={19} />}{report.usable ? "实际连接正常" : "连接未通过"}</p><div><span>域名解析</span><strong>{report.dns ? `${Math.round(report.dns.elapsedMs)} ms${report.dns.usedFallback ? " · 使用系统回退" : report.dns.viaAppHosts ? " · Hosts" : ""}` : report.viaProxy ? "由代理解析" : "失败"}</strong></div>{report.dns ? <p className="report-addresses">{report.dns.addresses.join(" · ")}</p> : null}<div><span>HTTPS 连接</span><strong>{report.httpStatus ? `HTTP ${report.httpStatus}` : "未建立"}</strong></div><div><span>总耗时</span><strong>{(report.elapsedMs / 1000).toFixed(1)} 秒</strong></div>{report.dnsError ? <p className="error-text">{report.dnsError}</p> : null}{report.httpError ? <p className="error-text">{report.httpError}</p> : null}{report.dns?.usedFallback ? <p className="fallback-note">自定义 DNS 未生效，本次使用了系统 DNS。</p> : null}</div> : null}
    </div>
  </section>;
}
