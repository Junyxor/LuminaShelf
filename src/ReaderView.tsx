import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

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

const MAX_FIND_MATCHES = 500;

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
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [activeMatchIndex, setActiveMatchIndex] = useState(0);
  const [chapterCache, setChapterCache] = useState<Record<string, ReaderChapter>>({});
  const [chapterLoading, setChapterLoading] = useState(false);
  const [chapterError, setChapterError] = useState<string | null>(null);
  const matchRefs = useRef<Array<HTMLElement | null>>([]);
  const chapter = book.chapters[chapterIndex];
  const chapterCount = book.chapters.length;
  const loadedChapter = chapter ? chapterCache[chapter.id] : undefined;
  const chapterText = chapter?.text || loadedChapter?.text || "";

  useEffect(() => {
    localStorage.setItem("luminashelf.reader.preferences", JSON.stringify(preferences));
  }, [preferences]);

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
    const next = book.chapters[chapterIndex + 1];
    if (!next || next.text || chapterCache[next.id] || !next.path) return;

    let cancelled = false;
    invoke<ReaderChapter>("open_local_book_chapter", {
      path: next.path,
      chapterId: next.id,
    })
      .then((loaded) => {
        if (!cancelled) {
          setChapterCache((current) => ({ ...current, [next.id]: loaded }));
        }
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, [book.chapters, chapterCache, chapterIndex, chapterText]);

  useEffect(() => {
    function handleFindShortcut(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        setSearchOpen(true);
      } else if (searchOpen && event.key === "Escape") {
        event.preventDefault();
        setSearchOpen(false);
      }
    }
    window.addEventListener("keydown", handleFindShortcut);
    return () => window.removeEventListener("keydown", handleFindShortcut);
  }, [searchOpen]);

  const searchIndex = useMemo(() => {
    const needle = searchQuery.trim().toLowerCase();
    if (!needle || !chapterText) {
      return { positions: [] as number[], queryLength: 0, truncated: false };
    }

    const source = chapterText.toLowerCase();
    const positions: number[] = [];
    let offset = 0;
    let truncated = false;
    while (offset <= source.length - needle.length) {
      const index = source.indexOf(needle, offset);
      if (index < 0) break;
      positions.push(index);
      const nextOffset = index + Math.max(1, needle.length);
      if (positions.length >= MAX_FIND_MATCHES) {
        truncated = source.indexOf(needle, nextOffset) >= 0;
        break;
      }
      offset = nextOffset;
    }
    return { positions, queryLength: needle.length, truncated };
  }, [chapterText, searchQuery]);

  useEffect(() => {
    setActiveMatchIndex(0);
    matchRefs.current = [];
  }, [chapter?.id, searchQuery]);

  useEffect(() => {
    if (!searchOpen || searchIndex.positions.length === 0) return;
    matchRefs.current[activeMatchIndex]?.scrollIntoView({
      behavior: "smooth",
      block: "center",
      inline: "nearest",
    });
  }, [activeMatchIndex, searchIndex.positions.length, searchOpen]);

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

  function moveMatch(delta: number) {
    const count = searchIndex.positions.length;
    if (count === 0) return;
    setActiveMatchIndex((current) => (current + delta + count) % count);
  }

  function renderChapterText(): ReactNode {
    if (!searchQuery.trim() || searchIndex.positions.length === 0) {
      return chapterText || "本章没有可显示正文。";
    }

    const nodes: ReactNode[] = [];
    let cursor = 0;
    searchIndex.positions.forEach((position, index) => {
      if (position > cursor) {
        nodes.push(chapterText.slice(cursor, position));
      }
      const end = position + searchIndex.queryLength;
      nodes.push(
        <mark
          key={`${position}:${index}`}
          ref={(node) => {
            matchRefs.current[index] = node;
          }}
          className={`reader-search-hit ${index === activeMatchIndex ? "active" : ""}`}
        >
          {chapterText.slice(position, end)}
        </mark>,
      );
      cursor = end;
    });
    if (cursor < chapterText.length) {
      nodes.push(chapterText.slice(cursor));
    }
    return nodes;
  }

  const matchCountLabel = searchQuery.trim()
    ? searchIndex.positions.length > 0
      ? `${Math.min(activeMatchIndex + 1, searchIndex.positions.length)}/${searchIndex.positions.length}${searchIndex.truncated ? "+" : ""}`
      : "0/0"
    : "查找";

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
            <button
              className="ghost-button reader-icon-button"
              onClick={() => setSearchOpen((open) => !open)}
              aria-label="在本章查找"
              aria-pressed={searchOpen}
            >
              查找
            </button>
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

          {searchOpen ? (
            <div className="reader-find-panel" role="search">
              <input
                autoFocus
                value={searchQuery}
                onChange={(event) => setSearchQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    moveMatch(event.shiftKey ? -1 : 1);
                  } else if (event.key === "Escape") {
                    event.preventDefault();
                    setSearchOpen(false);
                  }
                }}
                placeholder="在本章查找…"
                aria-label="在本章查找"
              />
              <span className="reader-find-count">{matchCountLabel}</span>
              <button onClick={() => moveMatch(-1)} disabled={searchIndex.positions.length === 0} aria-label="上一个匹配">↑</button>
              <button onClick={() => moveMatch(1)} disabled={searchIndex.positions.length === 0} aria-label="下一个匹配">↓</button>
              <button onClick={() => setSearchOpen(false)} aria-label="关闭查找">×</button>
            </div>
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
                    : renderChapterText()}
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
            <button className="ghost-button" disabled={chapterIndex === 0} onClick={() => onChapterChange(chapterIndex - 1)}>上一章</button>
            <button className="primary-button" disabled={chapterIndex >= chapterCount - 1} onClick={() => onChapterChange(chapterIndex + 1)}>下一章</button>
          </div>
        </footer>
      </section>
    </div>
  );
}
