import { invoke } from "@tauri-apps/api/core";
import type {
  PDFDataRangeTransport as PDFDataRangeTransportType,
  PDFDocumentLoadingTask,
  PDFDocumentProxy,
} from "pdfjs-dist";
import { useEffect, useRef, useState } from "react";
import "./pdf.css";
import ReaderBookmarks from "./ReaderBookmarks";

const PDF_RANGE_CHUNK_SIZE = 64 * 1024;
const PDF_RANGE_LENGTH_PREFIX_BYTES = 8;

type PdfJsRuntime = typeof import("pdfjs-dist/legacy/build/pdf.mjs");

let pdfJsRuntimePromise: Promise<PdfJsRuntime> | null = null;

function loadPdfJsRuntime(): Promise<PdfJsRuntime> {
  if (!pdfJsRuntimePromise) {
    pdfJsRuntimePromise = Promise.all([
      import("pdfjs-dist/legacy/build/pdf.mjs"),
      import("pdfjs-dist/legacy/build/pdf.worker.mjs?url"),
    ]).then(([pdfJs, worker]) => {
      pdfJs.GlobalWorkerOptions.workerSrc = worker.default;
      return pdfJs;
    });
  }
  return pdfJsRuntimePromise;
}

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

type ActiveRenderTask = {
  promise: Promise<unknown>;
  cancel: () => void;
};

type PdfRangePayload = {
  length: number;
  data: Uint8Array;
};

function decodePdfRangePayload(buffer: ArrayBuffer): PdfRangePayload {
  if (buffer.byteLength < PDF_RANGE_LENGTH_PREFIX_BYTES) {
    throw new Error("PDF range response is missing its length prefix");
  }
  const rawLength = new DataView(buffer).getBigUint64(0, true);
  if (rawLength > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error("PDF is too large for JavaScript range addressing");
  }
  return {
    length: Number(rawLength),
    data: new Uint8Array(buffer, PDF_RANGE_LENGTH_PREFIX_BYTES).slice(),
  };
}

type RangeTransportHandle = PDFDataRangeTransportType & {
  abort: () => void;
};

function createTauriPdfRangeTransport(
  pdfJs: PdfJsRuntime,
  path: string,
  length: number,
  initialData: Uint8Array,
  onRangeError: (reason: unknown) => void,
): RangeTransportHandle {
  return new (class extends pdfJs.PDFDataRangeTransport {
    private aborted = false;

    constructor() {
      super(length, initialData);
    }

    requestDataRange(begin: number, end: number) {
      if (this.aborted) return;
      void invoke<ArrayBuffer>("read_pdf_bytes", {
        path,
        start: begin,
        end,
      })
        .then((buffer) => {
          if (this.aborted) return;
          const payload = decodePdfRangePayload(buffer);
          this.onDataRange(begin, payload.data);
        })
        .catch((reason) => {
          if (!this.aborted) onRangeError(reason);
        });
    }

    abort() {
      this.aborted = true;
    }
  })();
}

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
    let loadingTask: PDFDocumentLoadingTask | null = null;
    let rangeTransport: RangeTransportHandle | null = null;

    async function load() {
      setLoading(true);
      setError(null);
      try {
        const [pdfJs, initialBuffer, progress] = await Promise.all([
          loadPdfJsRuntime(),
          invoke<ArrayBuffer>("read_pdf_bytes", {
            path,
            start: 0,
            end: PDF_RANGE_CHUNK_SIZE,
          }),
          invoke<ReadingProgress | null>("reading_progress", { libraryId }),
        ]);
        if (cancelled) return;

        const initial = decodePdfRangePayload(initialBuffer);
        rangeTransport = createTauriPdfRangeTransport(
          pdfJs,
          path,
          initial.length,
          initial.data,
          (reason) => {
            if (!cancelled) {
              setError(String(reason));
              setLoading(false);
              void loadingTask?.destroy();
            }
          },
        );
        loadingTask = pdfJs.getDocument({
          range: rangeTransport,
          rangeChunkSize: PDF_RANGE_CHUNK_SIZE,
          disableAutoFetch: true,
          disableStream: true,
        });
        const loadedDocument = await loadingTask.promise;
        if (cancelled) return;
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
      rangeTransport?.abort();
      if (loadingTask) void loadingTask.destroy();
    };
  }, [libraryId, path]);

  useEffect(() => {
    if (!document) return;
    let cancelled = false;
    let renderTask: ActiveRenderTask | null = null;

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
        const message = String(reason);
        if (!cancelled && !message.includes("RenderingCancelledException")) setError(message);
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
            <ReaderBookmarks libraryId={libraryId} label={`第 ${pageNumber} 页`} locator={() => `page:${pageNumber}`} fraction={() => numPages ? pageNumber / numPages : 0}
              onSelect={(bookmark) => { if (bookmark.locator.startsWith("page:")) setPageNumber(Math.min(numPages || 1, Math.max(1, Number(bookmark.locator.slice(5)) || 1))); }} />
            <button onClick={() => setZoom((value) => Math.max(0.6, Math.round((value - 0.1) * 10) / 10))}>−</button>
            <strong>{Math.round(zoom * 100)}%</strong>
            <button onClick={() => setZoom((value) => Math.min(2.5, Math.round((value + 0.1) * 10) / 10))}>+</button>
          </div>
        </header>

        <div className="pdf-stage">
          {loading ? <div className="pdf-status">正在通过 Rust Core 分块读取 PDF…</div> : null}
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
