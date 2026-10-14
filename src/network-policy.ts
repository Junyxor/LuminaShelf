export type ResolverMode = "system" | "app_hosts" | "doh" | "dot" | "custom_dns";
export type ResolverPolicy = {
  mode: ResolverMode; appHosts: Record<string, string[]>; upstreamIp?: string | null;
  serverName?: string | null; dohPath?: string | null; dohUrl?: string | null;
  upstreamPort?: number | null; proxyUrl?: string | null; fallbackToSystem: boolean; timeoutMs: number;
};
export const defaultPolicy: ResolverPolicy = { mode: "system", appHosts: {}, upstreamIp: null, serverName: null, dohPath: "/dns-query", dohUrl: null, upstreamPort: null, proxyUrl: null, fallbackToSystem: true, timeoutMs: 5000 };
export const dnsPresets = [
  { name: "阿里", url: "https://dns.alidns.com/dns-query" },
  { name: "腾讯", url: "https://doh.pub/dns-query" },
  { name: "Cloudflare", url: "https://cloudflare-dns.com/dns-query" },
] as const;
export function formatHosts(entries: Record<string, string[]>) { return Object.entries(entries).flatMap(([host, ips]) => ips.map((ip) => `${ip} ${host}`)).join("\n"); }
export function parseHosts(text: string): Record<string, string[]> {
  const entries: Record<string, string[]> = {};
  const put = (raw: string, ips: string[]) => {
    const host = raw.trim().replace(/\.$/, "").toLowerCase();
    if (!host || /[\s/=]/.test(host) || !ips.length) throw new Error("Hosts 域名或 IP 不完整");
    entries[host] = [...new Set([...(entries[host] || []), ...ips])];
  };
  text.split(/\r?\n/).forEach((raw, index) => {
    const line = raw.split("#")[0].trim(); if (!line) return;
    if (line.includes("=")) {
      const [host, values] = line.split("=", 2); put(host, values.split(/[,\s]+/).filter(Boolean));
    } else {
      const [ipText, ...hosts] = line.split(/\s+/);
      const ips = ipText.split(",").filter(Boolean);
      if (!hosts.length) throw new Error(`Hosts 第 ${index + 1} 行缺少域名`);
      hosts.forEach((host) => put(host, ips));
    }
  });
  return entries;
}
export function cleanPolicy(draft: ResolverPolicy, hosts: string): ResolverPolicy {
  const policy: ResolverPolicy = { ...draft, appHosts: parseHosts(hosts), upstreamIp: draft.upstreamIp?.trim() || null, serverName: draft.serverName?.trim() || null, dohUrl: draft.dohUrl?.trim() || null, proxyUrl: draft.proxyUrl?.trim() || null, upstreamPort: draft.upstreamPort || null };
  if (policy.mode === "doh") {
    if (!policy.dohUrl && policy.serverName) policy.dohUrl = `https://${policy.serverName}${policy.dohPath || "/dns-query"}`;
    const endpoint = new URL(policy.dohUrl || "");
    if (endpoint.protocol !== "https:") throw new Error("DoH 地址需要以 https:// 开头");
    if (endpoint.hash) throw new Error("DoH 地址不能包含 # 片段");
  }
  return policy;
}
