import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { Plus, Check, Pencil, Trash2, Wifi, ChevronDown, AlertCircle, CheckCircle2 } from "lucide-react";
import { cleanPolicy, defaultPolicy, dnsPresets, dohAddress, formatHosts, type ResolverMode, type ResolverPolicy } from "./network-policy";

type Profile = { id: string; name: string; policy: ResolverPolicy };
type Report = { host: string; dns?: { addresses: string[]; elapsedMs: number; usedFallback: boolean; viaAppHosts: boolean }; dnsError?: string; httpStatus?: number; httpError?: string; elapsedMs: number; usable: boolean; viaProxy: boolean };
function address(policy: ResolverPolicy) {
  if (policy.mode === "doh") return dohAddress(policy);
  if (policy.mode === "dot") return `${policy.serverName || ""} · ${policy.upstreamIp || ""}:${policy.upstreamPort || 853}`;
  if (policy.mode === "custom_dns") return `${policy.upstreamIp || ""}:${policy.upstreamPort || 53}`;
  return "系统 DNS";
}
function dnsKey(policy: ResolverPolicy) {
  const value = { ...defaultPolicy, ...policy };
  return JSON.stringify([value.mode, Object.entries(value.appHosts).sort(([a], [b]) => a.localeCompare(b)),
    value.upstreamIp ?? null, value.bootstrapIps ?? [], value.serverName ?? null, value.dohPath ?? "/dns-query",
    value.dohUrl ?? null, value.upstreamPort ?? null, value.proxyUrl ?? null, value.fallbackToSystem, value.timeoutMs]);
}
const protocolName: Record<ResolverMode, string> = { system: "系统", app_hosts: "Hosts", doh: "DoH", dot: "DoT", custom_dns: "DNS" };

export default function DnsSettings() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [draft, setDraft] = useState<ResolverPolicy>(defaultPolicy);
  const [applied, setApplied] = useState<ResolverPolicy>(defaultPolicy);
  const [hosts, setHosts] = useState("");
  const [bootstrap, setBootstrap] = useState("");
  const [editingId, setEditingId] = useState("");
  const [name, setName] = useState("当前配置");
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [testHost, setTestHost] = useState("gutendex.com");
  const [report, setReport] = useState<Report | null>(null);
  const [rowReports, setRowReports] = useState<Record<string, string>>({});
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    let active = true;
    void Promise.all([invoke<ResolverPolicy>("resolver_policy"), invoke<Profile[]>("dns_profiles")]).then(([policy, items]) => {
      if (!active) return;
      setProfiles(items); adopt(policy); setApplied(policy);
      const selected = items.find((item) => dnsKey(item.policy) === dnsKey(policy));
      if (selected) { setEditingId(selected.id); setName(selected.name); }
      setLoaded(true);
    }).catch((reason) => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, []);

  function adopt(policy: ResolverPolicy) {
    if (policy.mode === "doh" && !policy.dohUrl && policy.serverName) policy = { ...policy, dohUrl: dohAddress(policy) };
    setDraft({ ...defaultPolicy, ...policy }); setHosts(formatHosts(policy.appHosts)); setBootstrap((policy.bootstrapIps || []).join("\n")); setDirty(false);
  }
  function change(patch: Partial<ResolverPolicy>) { setDraft((current) => ({ ...current, ...patch })); setDirty(true); setMessage(null); setReport(null); }
  function candidate() {
    const policy = cleanPolicy(draft, hosts);
    policy.bootstrapIps = [...new Set(bootstrap.split(/[\s,;]+/).filter(Boolean))];
    return policy;
  }
  function newProfile() {
    setEditingId(""); setName(""); adopt({ ...applied, mode: "doh", dohUrl: "", upstreamIp: null, bootstrapIps: [], serverName: null, upstreamPort: null }); setDirty(true); setReport(null); setError(null); setMessage(null);
    document.getElementById("dns-editor")?.scrollIntoView({ block: "start", behavior: "smooth" });
  }
  function edit(profile: Profile) { adopt(profile.policy); setName(profile.name); setEditingId(profile.id); setError(null); setMessage(null); document.getElementById("dns-editor")?.scrollIntoView({ block: "start", behavior: "smooth" }); }
  async function save() {
    setBusy("save"); setError(null); setMessage(null);
    try {
      if (!name.trim()) throw new Error("给这条 DNS 配置起一个名称");
      const profile = await invoke<Profile>("save_dns_profile", { profile: { id: editingId, name: name.trim(), policy: candidate() } });
      setProfiles((current) => [...current.filter((item) => item.id !== profile.id), profile]); setEditingId(profile.id);
      const policy = await invoke<ResolverPolicy>("apply_dns_profile", { id: profile.id }); adopt(policy); setApplied(policy);
      setMessage("已保存并应用到搜索、下载和封面连接");
    } catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  }
  async function use(profile: Profile) {
    setBusy(profile.id); setError(null);
    try { const policy = await invoke<ResolverPolicy>("apply_dns_profile", { id: profile.id }); adopt(policy); setApplied(policy); setName(profile.name); setEditingId(profile.id); setMessage(`正在使用 ${profile.name}`); }
    catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  }
  async function remove(profile: Profile) {
    setBusy(profile.id); setError(null);
    try {
      const policy = await invoke<ResolverPolicy>("remove_dns_profile", { id: profile.id }); setProfiles((items) => items.filter((item) => item.id !== profile.id)); setApplied(policy);
      if (editingId === profile.id) { adopt(policy); setEditingId(""); setName("当前配置"); }
      setMessage(dnsKey(profile.policy) === dnsKey(applied) && policy.mode === "system" ? "已移除配置，当前使用系统 DNS" : "已移除 DNS 配置");
    } catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  }
  async function testProfile(profile: Profile) {
    try {
      const result = await invoke<Report>("test_network", { policy: { ...profile.policy, fallbackToSystem: false }, host: testHost.trim() || "gutendex.com" });
      setRowReports((items) => ({ ...items, [profile.id]: result.usable ? `${Math.round(result.elapsedMs)} ms · 可用` : result.dnsError || result.httpError || "不可用" }));
    } catch (reason) { setRowReports((items) => ({ ...items, [profile.id]: String(reason) })); }
  }
  async function testAll() {
    setBusy("all"); let index = 0;
    const worker = async () => { while (index < profiles.length) { const item = profiles[index++]; await testProfile(item); } };
    try { await Promise.all([worker(), worker()]); } finally { setBusy(null); }
  }
  async function test() {
    setBusy("test"); setError(null); setReport(null); setMessage(null);
    try { setReport(await invoke<Report>("test_network", { policy: candidate(), host: testHost.trim() })); } catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  }
  async function auto() {
    setBusy("auto"); setError(null); setReport(null);
    try {
      const result = await invoke<{ policy: ResolverPolicy; report: Report; zlibrary?: Report | null; label: string }>("optimize_network");
      const zlibrary = result.zlibrary ? (result.zlibrary.usable ? "Z-Library 可连接" : "Z-Library 仍无法连接，可配置代理") : null;
      adopt(result.policy); setApplied(result.policy); setReport(result.report); setMessage(`已选用 ${result.label}，实际书源连接正常${zlibrary ? `；${zlibrary}` : ""}`);
      const selected = profiles.find((item) => dnsKey(item.policy) === dnsKey(result.policy)); setEditingId(selected?.id || ""); setName(selected?.name || result.label);
    } catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  }
  async function reset() {
    setBusy("reset"); setError(null);
    try { const policy = await invoke<ResolverPolicy>("set_resolver_policy", { policy: defaultPolicy }); adopt(policy); setApplied(policy); setEditingId(""); setName("系统网络"); setMessage("已恢复系统网络"); } catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  }

  return <section className="settings-section network-section" aria-label="网络设置">
    <div className="network-heading"><div><h2>DNS 服务器</h2><p>保存自己的 DNS、DoH 或 DoT 配置，随时切换。连接不上书源时，先试试“自动检测并选择”。</p></div><Wifi size={22} /></div>
    <div className="current-network"><span>当前连接</span><strong>{protocolName[applied.mode]} · {address(applied)}</strong></div>
    <button className="secondary-button network-auto" onClick={() => void auto()} disabled={!loaded || Boolean(busy)}>{busy === "auto" ? "正在测试可用连接…" : "自动检测并选择"}</button>
    <div className="profile-toolbar"><button className="primary-button" onClick={newProfile} disabled={!loaded || Boolean(busy)}><Plus size={18} />添加 DNS</button><button className="secondary-button" onClick={() => void testAll()} disabled={!profiles.length || Boolean(busy)}>{busy === "all" ? "正在测试…" : "测试全部"}</button></div>
    <div className="dns-profile-list">{profiles.length ? profiles.map((profile) => <article className="dns-profile" key={profile.id}>
      <div className="profile-title"><strong>{profile.name}</strong><span className="format-badge">{protocolName[profile.policy.mode]}</span>{dnsKey(profile.policy) === dnsKey(applied) ? <span className="profile-current"><Check size={16} />使用中</span> : null}</div>
      <p className="profile-address">{address(profile.policy)}</p>
      {rowReports[profile.id] ? <p className="profile-test-result" role="status">{rowReports[profile.id]}</p> : null}
      <div className="profile-actions"><button className="secondary-button" disabled={Boolean(busy)} aria-label={`使用 ${profile.name}`} onClick={() => void use(profile)}>使用</button><button className="text-button" disabled={Boolean(busy)} aria-label={`测试 ${profile.name}`} onClick={() => { setBusy(profile.id); void testProfile(profile).finally(() => setBusy(null)); }}>测试</button><button className="icon-button" disabled={Boolean(busy)} aria-label={`编辑 ${profile.name}`} onClick={() => edit(profile)}><Pencil size={19} /></button><button className="icon-button" disabled={Boolean(busy)} aria-label={`删除 ${profile.name}`} onClick={() => void remove(profile)}><Trash2 size={19} /></button></div>
    </article>) : <p className="settings-hint">还没有保存的配置。点击“添加 DNS”，填写自己的服务地址。</p>}</div>
    <div id="dns-editor" className="dns-editor">
      <h3>{editingId ? "编辑配置" : "DNS 配置"}</h3>
      <label className="network-field"><span>配置名称</span><input aria-label="配置名称" placeholder="例如：我的 DNS / 家庭网络" value={name} onChange={(event) => { setName(event.target.value); setDirty(true); }} maxLength={80} /></label>
      <label className="network-field"><span id="resolver-mode-label">DNS 解析方式</span><select aria-labelledby="resolver-mode-label" value={draft.mode} onChange={(event) => {
        const mode = event.target.value as ResolverMode; change({ mode, ...(mode === "doh" ? { dohUrl: draft.dohUrl || "", upstreamIp: null, serverName: null, upstreamPort: null } : {}) });
      }}><option value="system">系统 DNS</option><option value="doh">加密 DNS（DoH）</option><option value="custom_dns">自定义普通 DNS</option><option value="dot">加密 DNS（DoT）</option><option value="app_hosts">系统 DNS 与 Hosts</option></select></label>
      {draft.mode === "doh" ? <>
        <label className="network-field"><span id="doh-url-label">DoH 地址</span><input aria-labelledby="doh-url-label" aria-describedby="doh-url-help" type="url" autoCapitalize="none" autoCorrect="off" spellCheck={false} placeholder="https://你的域名/dns-query" value={draft.dohUrl || ""} onChange={(event) => change({ dohUrl: event.target.value })} /><small id="doh-url-help">完整 HTTPS 地址，支持自定义端口、路径和查询参数。</small></label>
        <div className="dns-presets" role="group" aria-label="公共 DoH 服务">{dnsPresets.map((preset) => <button key={preset.name} className={draft.dohUrl === preset.url ? "selected" : ""} onClick={() => { change({ dohUrl: preset.url, upstreamIp: null, serverName: null, upstreamPort: null }); if (!name || name === "当前配置") setName(`${preset.name} DoH`); }}>{preset.name}</button>)}</div>
        <label className="network-field"><span>引导 IP（选填）</span><textarea aria-label="引导 IP" rows={2} spellCheck={false} placeholder={"留空自动引导，或每行填写一个 IP\n支持 IPv4、IPv6"} value={bootstrap} onChange={(event) => { setBootstrap(event.target.value); setDirty(true); }} /><small>支持多个地址，逗号或换行分隔；仍验证 DoH 地址的域名证书。</small></label>
      </> : null}
      {draft.mode === "custom_dns" || draft.mode === "dot" ? <label className="network-field"><span>DNS 服务器 IP</span><input aria-label="DNS 服务器 IP" spellCheck={false} placeholder="223.5.5.5" value={draft.upstreamIp || ""} onChange={(event) => change({ upstreamIp: event.target.value })} /></label> : null}
      {draft.mode === "dot" ? <label className="network-field"><span>DoT 服务域名</span><input aria-label="DoT 服务域名" placeholder="dns.alidns.com" value={draft.serverName || ""} onChange={(event) => change({ serverName: event.target.value })} /></label> : null}
      {draft.mode !== "system" && draft.mode !== "app_hosts" ? <label className="check-field"><input type="checkbox" checked={draft.fallbackToSystem} onChange={(event) => change({ fallbackToSystem: event.target.checked })} /><span>解析失败时使用系统 DNS<small>测试结果会标明是否发生回退。</small></span></label> : null}
      <details className="network-advanced"><summary>高级设置<ChevronDown size={18} /></summary>
        {draft.mode === "custom_dns" || draft.mode === "dot" ? <label className="network-field"><span>DNS 端口（选填）</span><input aria-label="DNS 端口" type="number" min={1} max={65535} placeholder={draft.mode === "dot" ? "853" : "53"} value={draft.upstreamPort || ""} onChange={(event) => change({ upstreamPort: event.target.value ? Number(event.target.value) : null })} /></label> : null}
        <label className="network-field"><span id="dns-timeout-label">解析超时</span><select aria-labelledby="dns-timeout-label" value={draft.timeoutMs} onChange={(event) => change({ timeoutMs: Number(event.target.value) })}>{[3000, 5000, 10000, 15000].map((value) => <option key={value} value={value}>{value / 1000} 秒</option>)}</select></label>
        <label className="network-field"><span>代理地址（选填）</span><input aria-label="代理地址" type="url" autoCapitalize="none" spellCheck={false} placeholder="http://192.168.1.100:7890" value={draft.proxyUrl || ""} onChange={(event) => change({ proxyUrl: event.target.value })} /><small>支持 HTTP、HTTPS、SOCKS5。</small></label>
        <label className="network-field"><span>应用内 Hosts</span><textarea aria-label="应用内 Hosts" rows={5} spellCheck={false} placeholder={"203.0.113.10 books.example.org\n2001:db8::10 *.example.org"} value={hosts} onChange={(event) => { setHosts(event.target.value); setDirty(true); }} /><small>支持“IP 域名”与“域名 = IP”。</small></label>
      </details>
      <div className="network-actions"><button className="primary-button" disabled={!loaded || Boolean(busy)} onClick={() => void save()}>{busy === "save" ? "正在应用" : "保存并应用"}</button><button className="text-button" disabled={Boolean(busy)} onClick={() => void reset()}>恢复系统网络</button>{dirty ? <span>修改尚未保存</span> : null}</div>
    </div>
    {message ? <p className="network-feedback success" role="status"><CheckCircle2 size={19} />{message}</p> : null}
    {error ? <p className="network-feedback error" role="alert"><AlertCircle size={19} />{error}</p> : null}
    <div className="network-test"><h3>测试当前填写的配置</h3><p>检查域名解析与真实 HTTPS 连接。列表测速会关闭系统回退。</p><div className="network-test-controls"><input aria-label="测试域名" value={testHost} onChange={(event) => setTestHost(event.target.value)} /><button className="secondary-button" onClick={() => void test()} disabled={Boolean(busy) || !testHost.trim()}>{busy === "test" ? "测试中" : "测试配置"}</button></div>
      {report ? <div className="network-report" aria-live="polite"><p className={report.usable ? "report-good" : "error-text"}>{report.usable ? "实际连接正常" : "连接未通过"}</p><div><span>域名解析</span><strong>{report.dns ? `${Math.round(report.dns.elapsedMs)} ms${report.dns.usedFallback ? " · 使用系统回退" : ""}` : report.viaProxy ? "由代理解析" : "失败"}</strong></div>{report.dns ? <p className="report-addresses">{report.dns.addresses.join(" · ")}</p> : null}<div><span>HTTPS 连接</span><strong>{report.httpStatus ? `HTTP ${report.httpStatus}` : "未建立"}</strong></div>{report.dnsError ? <p className="error-text">{report.dnsError}</p> : null}{report.httpError ? <p className="error-text">{report.httpError}</p> : null}{report.dns?.usedFallback ? <p className="fallback-note">本次使用了系统 DNS 回退。</p> : null}</div> : null}
    </div>
  </section>;
}
