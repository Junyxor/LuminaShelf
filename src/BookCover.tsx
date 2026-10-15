import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";

export default function BookCover({ title, authors = [], url, format }: { title: string; authors?: string[]; url?: string | null; format?: string }) {
  const [image, setImage] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const element = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!element.current || !('IntersectionObserver' in window)) { setVisible(true); return; }
    const observer = new IntersectionObserver((entries) => { if (entries.some((entry) => entry.isIntersecting)) { setVisible(true); observer.disconnect(); } }, { rootMargin: "300px" });
    observer.observe(element.current); return () => observer.disconnect();
  }, []);
  const tone = [...title].reduce((sum, char) => sum + char.charCodeAt(0), 0) % 4;
  useEffect(() => {
    setImage(null); if (!url || !visible) return;
    let cancelled = false; let objectUrl: string | undefined;
    void invoke<ArrayBuffer>("book_cover_bytes", { url }).then((bytes) => {
      if (cancelled) return;
      const data = new Uint8Array(bytes);
      const type = data[0] === 0x89 ? "image/png" : data[0] === 0x52 ? "image/webp" : "image/jpeg";
      objectUrl = URL.createObjectURL(new Blob([bytes], { type })); setImage(objectUrl);
    }).catch(() => {});
    return () => { cancelled = true; if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [url, visible]);
  return <div className={`book-cover tone-${tone}`} ref={element}>{image ? <img src={image} alt="" loading="lazy" onError={() => setImage(null)} /> : <div className="type-cover"><span>{format?.toUpperCase() || "LUMINA"}</span><strong>{title}</strong><small>{authors[0] || "星书"}</small></div>}</div>;
}
