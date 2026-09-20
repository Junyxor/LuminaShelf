export type ReaderPosition = { chapterId: string; scroll: number };

export function parseReaderPosition(locator?: string | null): ReaderPosition | null {
  if (!locator) return null;
  try {
    const value = JSON.parse(locator);
    if (typeof value.chapterId === "string") return { chapterId: value.chapterId, scroll: Math.min(1, Math.max(0, Number(value.scroll) || 0)) };
  } catch { /* Earlier versions stored the chapter id directly. */ }
  return { chapterId: locator, scroll: 0 };
}
