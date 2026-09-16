import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";

type ReaderChapter = {
  id: string;
  title: string;
  text: string;
  path?: string;
};

type ReaderBook = {
  title: string;
  chapters: ReaderChapter[];
};

type ReaderTheme = "light" | "sepia" | "dark";

type ReaderPreferences = {
  fontSize: number;
  lineHeight: number;
  theme: ReaderTheme;
};

type Props = {
  book: ReaderBook;
  fallbackTitle: string;
  format: string;
  chapterIndex: number;
  progressFraction: number;
  onChapterChange: (index: number) => void;
  onClose: () => void;
};

const defaultPreferences: ReaderPreferences = {
  fontSize: 19,
  lineHeight: 1.9,
  theme: "light",
};

function loadPreferences(): ReaderPreferences {
  try {
    const raw = localStorage.getItem("luminashelf.reader.preferences");
    if (!raw) return defaultPreferences;
    const value = JSON.parse(raw) as Partial<ReaderPreferences>;
    return {
      fontSize: Math.min(30, Math.max(14, Number(value.fontSize) || defaultPreferences.fontSize)),
      lineHeight: Math.min(2.4, Math.max(1.4, Number(value.lineHeight) || defaultPreferences.lineHeight)),
      theme: value.theme === "sepia" || value.theme === "dark" ? value.theme : "light",
    };
  } catch {
    return defaultPreferences;
  }
}

export default function ReaderView({
  book,
  fallbackTitle,
  format,
  chapterIndex,
  progressFraction,
  onChapterChange,
  onClose,
}: Props) {
  const [preferences, setPreferences] = useState<ReaderPreferences>(() => loadPreferences());
  const [tocOpen, setTocOpen] = useState(false);
  const [chapterCache, setChapterCache] = useState<Record<string, ReaderChapter>>({});
  const [chapterLoading, setChapterLoading] = useState(false);
  const [chapterError, setChapterError] = useState<string | null>(null);
  const chapter = book.chapters[chapterIndex];
  const chapterCount = book.chapters.length;
  const loadedChapter = chapter ? chapterCache[chapter.id] : undefined;
  const chapterText = chapter?.text || loadedChapter?.text || "";
  const bookCacheKey = `${book.title}:${book.chapters.length}:${book.chapters[0]?.path ?? ""}`;

  useEffect(() => {
    localStorage.setItem("luminashelf.reader.preferences", JSON.stringify(preferences));
  }, [preferences]);

  useEffect(() => {
    setChapterCache({});
    setChapterError(null);
  }, [bookCacheKey]);

  useEffect(() => {
    const keepIds = new Set(
      [chapterIndex - 1, chapterIndex, chapterIndex + 1]
        .map((index) => book.chapters[index]?.id)
        .filter((id): id is string => Boolean(id)),
    );
    setChapterCache((current) => {
      const entries = Object.entries(current).filter(([id]) => keepIds.has(id));
      if (entries.length === Object.keys(current).length) return current;
      return Object.fromEntries(entries);
    });
  }, [book.chapters, chapterIndex]);

  useEffect(() => {
    if (!chapter) return;
    if (chapter.text || chapterCache[chapter.id]) {
      setChapterLoading(false);
      setChapterError(null);
      return;
    }
    if (!chapter.path) {
      setChapterLoading(false);
      setChapterError("章节正文路径缺失，无法按需加载。");
      return;
    }

    let cancelled = false;
    setChapterLoading(true);
    setChapterError(null);
    invoke<ReaderChapter>("open_local_book_chapter", {
      path: chapter.path,
      chapterId: chapter.id,
    })
      .then((loaded) => {
        if (cancelled) return;
        setChapterCache((current) => ({ ...current, [chapter.id]: loaded }));
      })
      .catch((reason) => {
        if (!cancelled) setChapterError(String(reason));
      })
      .finally(() => {
        if (!cancelled) setChapterLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [chapter?.id, chapter?.path, chapter?.text]);

  useEffect(() => {
    if (!chapterText) return;
    const adjacent = [book.chapters[chapterIndex - 1], book.chapters[chapterIndex + 1]].filter(
      (item): item is ReaderChapter => Boolean(item),
    );
    const pending = adjacent.filter(
      (item) => !item.text && !chapterCache[item.id] && Boolean(item.path),
    );
    if (pending.length === 0) return;

    let cancelled = false;
    void Promise.allSettled(
      pending.map(async (item) => {
        const loaded = await invoke<ReaderChapter>("open_local_book_chapter", {
          path: item.path,
          chapterId: item.id,
        });
        if (!cancelled) {
          setChapterCache((current) => ({ ...current, [item.id]: loaded }));
        }
      }),
    );

    return () => {
      cancelled = true;
    };
  }, [book.chapters, chapterIndex, chapterText]);

  const chapterLabel = useMemo(
    () => `${chapter?.title ?? "未命名章节"} · ${chapterIndex + 1}/${chapterCount}`,
    [chapter?.title, chapterIndex, chapterCount],
  );

  if (!chapter) return null;

  function changeFontSize(delta: number) {
    setPreferences((current) => ({
      ...current,
      fontSize: Math.min(30, Math.max(14, current.fontSize + delta)),
    }));
  }

  function changeLineHeight(delta: number) {
    setPreferences((current) => ({
      ...current,
      lineHeight: Math.round(Math.min(2.4, Math.max(1.4, current.lineHeight + delta)) * 10) / 10,
    }));
  }

  return (
    <div className="reader-backdrop" onClick={onClose}>
      <section className={`reader-shell reader-theme-${preferences.theme}`} onClick={(event) => event.stopPropagation()}>
        <header className="reader-toolbar">
          <button className="ghost-button" onClick={onClose}>← 返回书库</button>
          <div className="reader-title">
            <strong>{book.title || fallbackTitle}</strong>
            <small>{chapterLabel}</small>
          </div>
          <div className="reader-toolbar-actions">
            <button className="ghost-button reader-icon-button" onClick={() => setTocOpen((open) => !open)} aria-label="章节目录">目录</button>
            <span className="format-badge">{format.toUpperCase()}</span>
          </div>
        </header>

        <div className="reader-layout">
          {tocOpen ? (
            <aside className="reader-toc" aria-label="章节目录">
              <div className="reader-toc-heading">
                <strong>章节目录</strong>
                <button onClick={() => setTocOpen(false)} aria-label="关闭目录">×</button>
              </div>
              <div className="reader-toc-list">
                {book.chapters.map((item, index) => (
                  <button
                    key={`${item.id}:${index}`}
                    className={index === chapterIndex ? "active" : ""}
                    disabled={chapterLoading}
                    onClick={() => {
                      onChapterChange(index);
                      setTocOpen(false);
                    }}
                  >
                    <span>{index + 1}</span>
                    <strong>{item.title || `第 ${index + 1} 章`}</strong>
                  </button>
                ))}
              </div>
            </aside>
          ) : null}

          <div className="reader-body">
            <article>
              <h2>{chapter.title}</h2>
              <div
                className="reader-text"
                style={{ fontSize: `${preferences.fontSize}px`, lineHeight: preferences.lineHeight }}
              >
                {chapterLoading
                  ? "正在加载本章…"
                  : chapterError
                    ? `章节加载失败：${chapterError}`
                    : chapterText || "本章没有可显示正文。"}
              </div>
            </article>
          </div>
        </div>

        <footer className="reader-footer reader-footer-expanded">
          <div className="reader-position">
            <small>{Math.round(progressFraction * 100)}% · 进度自动保存到本地 SQLite</small>
            <div className="reader-progress-track"><i style={{ width: `${progressFraction * 100}%` }} /></div>
          </div>

          <div className="reader-controls" aria-label="阅读设置">
            <div className="reader-control-group">
              <span>字号</span>
              <button onClick={() => changeFontSize(-1)} disabled={preferences.fontSize <= 14}>A−</button>
              <strong>{preferences.fontSize}</strong>
              <button onClick={() => changeFontSize(1)} disabled={preferences.fontSize >= 30}>A+</button>
            </div>
            <div className="reader-control-group">
              <span>行距</span>
              <button onClick={() => changeLineHeight(-0.1)} disabled={preferences.lineHeight <= 1.4}>−</button>
              <strong>{preferences.lineHeight.toFixed(1)}</strong>
              <button onClick={() => changeLineHeight(0.1)} disabled={preferences.lineHeight >= 2.4}>+</button>
            </div>
            <div className="reader-theme-picker" aria-label="阅读主题">
              {(["light", "sepia", "dark"] as ReaderTheme[]).map((theme) => (
                <button
                  key={theme}
                  className={`${theme} ${preferences.theme === theme ? "active" : ""}`}
                  onClick={() => setPreferences((current) => ({ ...current, theme }))}
                  aria-label={theme === "light" ? "浅色主题" : theme === "sepia" ? "米黄主题" : "深色主题"}
                  title={theme === "light" ? "浅色" : theme === "sepia" ? "米黄" : "深色"}
                />
              ))}
            </div>
          </div>

          <div className="reader-nav-actions">
            <button className="ghost-button" disabled={chapterLoading || chapterIndex === 0} onClick={() => onChapterChange(chapterIndex - 1)}>上一章</button>
            <button className="primary-button" disabled={chapterLoading || chapterIndex >= chapterCount - 1} onClick={() => onChapterChange(chapterIndex + 1)}>下一章</button>
          </div>
        </footer>
      </section>
    </div>
  );
}
