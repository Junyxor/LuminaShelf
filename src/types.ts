export type BookFormat = "epub" | "pdf" | "mobi" | "azw3" | "txt" | "cbz" | "other";
export type BookSummary = { id: string; title: string; authors: string[]; year?: number | null; language?: string | null; format?: BookFormat | null; availableFormats: BookFormat[]; sizeBytes?: number | null; coverUrl?: string | null };
export type BookDetails = BookSummary & { description?: string | null; identifiers: Array<[string, string]> };
export type LibraryItem = { id: string; title: string; authors: string[]; path: string; format: string; sizeBytes: number; sourceProvider?: string | null; sourceBookId?: string | null };
export type ReadingProgress = { libraryId: string; locator?: string | null; fraction: number; updatedAtUnixMs: number };
export type ReaderBookMetadata = { title: string; chapters: Array<{ id: string; title: string }> };
