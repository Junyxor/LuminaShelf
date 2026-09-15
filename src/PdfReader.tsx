import { invoke } from "@tauri-apps/api/core";
import { GlobalWorkerOptions, getDocument, type PDFDocumentProxy } from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.mjs?url";
import { useEffect, useRef, useState } from "react";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

type ReadingProgress = {
  libraryId: string;
  locator?: string | null;
  fraction: number;
  updatedAtUnixMs: number;
};

type Props = {
  libraryId: string;
  path: string;
  title: string;
  onClose: () => void;
};

export default function PdfReader({ libraryId, path, title, onClose }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [pageNumber, setPageNumber] = useState(1);
  const [zoom, setZoom] = useState(1.15);
  const [loading, setLoading] = useState(true);
  const [rendering, setRendering] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let loadedDocument: PDFDocumentProxy | null = null;

    async function load() {
      setLoading(true);
      setError(null);
      try {
        const [buffer, progress] = await Promise.all([
          invoke<ArrayBuffer>("read_pdf_bytes", { path }),
          invoke<ReadingProgress | null>("reading_progress", { libraryId }),
        ]);
        if (cancelled) return;
        const task = getDocument({ data: new Uint8Array(buffer) });
        loadedDocument = await task.promise;
        if (cancelled) {
          await loadedDocument.destroy();
          return;
        }
        const savedPage = progress?.locator?.startsWith("page:")
          ? Number(progress.locator.slice(5))
          : Math.ceil((progress?.fraction ?? 0) * loadedDocument.numPages);
        setPageNumber(Math.min(loadedDocument.numPages, Math.max(1, savedPage || 1)));
        setDocument(loadedDocument);
      } catch (reason) {
        if (!cancelled) setError(String(reason));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    void load();
    return () => {
      cancelled = true;
      if (loadedDocument) void loadedDocument.destroy();
    };
  }, [libraryId, path]);

  useEffect(() => {
    if (!document) return;
    let cancelled = false;
    let renderTask: ReturnType<Awaited<ReturnType<PDFDocumentProxy["getPage"]>>["render"]> | null = null;

    async function renderPage() {
      const canvas = canvasRef.current;
      if (!canvas || !document) return;
      setRendering(true);
      setError(null);
      try {
        const page = await document.getPage(pageNumber);
        if (cancelled) return;
        const viewport = page.getViewport({ scale: zoom });
        const outputScale = Math.min(window.devicePixelRatio || 1, 2);
        canvas.width = Math.floor(viewport.width * outputScale);
        canvas.height = Math.floor(viewport.height * outputScale);
        canvas.style.width = `${Math.floor(viewport.width)}px`;
        canvas.style.height = `${Math.floor(viewport.height)}px`;
        renderTask = page.render({
          canvas,
          viewport,
          transform: outputScale === 1 ? undefined : [outputScale, 0, 0, outputScale, 0, 0],
        });
        await renderTask.promise;
      } catch (reason) {
        if (!cancelled && String(reason) !== "RenderingCancelledException") setError(String(reason));
      } finally {
        if (!cancelled) setRendering(false);
      }
    }

    void renderPage();
    return () => {
      cancelled = true;
      renderTask?.cancel();
    };
  }, [document, pageNumber, zoom]);

  useEffect(() => {
    if (!document) return;
    void invoke<ReadingProgress>("save_reading_progress", {
      libraryId,
      locator: `page:${pageNumber}`,
      fraction: pageNumber / document.numPages,
    }).catch((reason) => setError(String(reason)));
  }, [document, libraryId, pageNumber]);

  const numPages = document?.numPages ?? 0;

  return (
    <div className="pdf-backdrop" onClick={onClose}>
      <section className="pdf-shell" onClick={(event) => event.stopPropagation()}>
        <header className="pdf-toolbar">
          <button className="ghost-button" onClick={onClose}>← 返回书库</button>
          <div className="pdf-title">
            <strong>{title}</strong>
            <small>{numPages ? `第 ${pageNumber} / ${numPages} 页` : "正在载入 PDF…"}</small>
          </div>
          <div className="pdf-zoom-controls">
            <button onClick={() => setZoom((value) => Math.max(0.6, Math.round((value - 0.1) * 10) / 10))}>−</button>
            <strong>{Math.round(zoom * 100)}%</strong>
            <button onClick={() => setZoom((value) => Math.min(2.5, Math.round((value + 0.1) * 10) / 10))}>+</button>
          </div>
        </header>

        <div className="pdf-stage">
          {loading ? <div className="pdf-status">正在通过 Rust Core 读取 PDF…</div> : null}
          {error ? <div className="pdf-status error">{error}</div> : null}
          {!loading && !error ? (
            <div className={`pdf-canvas-wrap ${rendering ? "rendering" : ""}`}>
              <canvas ref={canvasRef} />
            </div>
          ) : null}
        </div>

        <footer className="pdf-footer">
          <button className="ghost-button" disabled={pageNumber <= 1} onClick={() => setPageNumber((value) => Math.max(1, value - 1))}>上一页</button>
          <div className="pdf-page-jump">
            <input
              type="number"
              min={1}
              max={numPages || 1}
              value={pageNumber}
              onChange={(event) => setPageNumber(Math.min(numPages || 1, Math.max(1, Number(event.target.value) || 1)))}
            />
            <span>/ {numPages || "—"}</span>
          </div>
          <button className="primary-button" disabled={!numPages || pageNumber >= numPages} onClick={() => setPageNumber((value) => Math.min(numPages, value + 1))}>下一页</button>
        </footer>
      </section>
    </div>
  );
}
