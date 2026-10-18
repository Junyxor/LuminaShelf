import { invoke } from "@tauri-apps/api/core";
import { memo, useEffect, useRef, useState } from "react";

// Object URLs fetched this session, keyed by source, so revisiting a page doesn't refetch.
const covers = new Map<string, Promise<string | null>>();

function imageType(bytes: Uint8Array) {
  if (bytes[0] === 0x89) return "image/png";
  if (bytes[0] === 0x52) return "image/webp";
  if (bytes[0] === 0x47) return "image/gif";
  return "image/jpeg";
}

function cover(key: string, fetch: () => Promise<ArrayBuffer>) {
  let entry = covers.get(key);
  if (!entry) {
    entry = fetch()
      .then((buffer) => (buffer.byteLength ? URL.createObjectURL(new Blob([buffer], { type: imageType(new Uint8Array(buffer)) })) : null))
      .catch(() => { covers.delete(key); return null; });
    covers.set(key, entry);
  }
  return entry;
}

/** Deterministic cloth colour per title, so a generated cover looks the same everywhere. */
export function coverTone(title: string) {
  return [...title].reduce((sum, char) => sum + char.charCodeAt(0), 0) % 6;
}

type Props = { title: string; authors?: string[]; url?: string | null; libraryId?: string | null; format?: string };

function BookCover({ title, authors = [], url, libraryId, format }: Props) {
  const [image, setImage] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const element = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!element.current || !("IntersectionObserver" in window)) { setVisible(true); return; }
    const observer = new IntersectionObserver((entries) => { if (entries.some((entry) => entry.isIntersecting)) { setVisible(true); observer.disconnect(); } }, { rootMargin: "300px" });
    observer.observe(element.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    setImage(null);
    if (!visible) return;
    let cancelled = false;
    void (async () => {
      // A downloaded book's own cover wins; the source's cover image is the fallback.
      const embedded = libraryId ? await cover(`library:${libraryId}`, () => invoke<ArrayBuffer>("library_cover", { libraryId })) : null;
      const found = embedded ?? (url ? await cover(`remote:${url}`, () => invoke<ArrayBuffer>("book_cover_bytes", { url })) : null);
      if (!cancelled) setImage(found);
    })();
    return () => { cancelled = true; };
  }, [url, libraryId, visible]);
  return <div className={`book-cover ${image ? "has-image" : `tone-${coverTone(title)}`}`} ref={element}>
    {image ? <img src={image} alt="" loading="lazy" onError={() => setImage(null)} /> : <div className="type-cover" aria-hidden="true">
      <span className="type-format">{format && format !== "other" ? format.toUpperCase() : "星书"}</span>
      <strong>{title}</strong>
      <i className="type-rule" />
      <small>{authors[0] || ""}</small>
    </div>}
  </div>;
}

export default memo(BookCover);
