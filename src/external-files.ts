import { invoke } from "@tauri-apps/api/core";
import type { LibraryItem } from "./types";
type Bridge = { openFile: (path: string, title: string) => string; shareFile: (path: string, mime: string, title: string) => string };
export async function openExternalFile(item: LibraryItem) {
  const bridge = (window as Window & { LuminaFiles?: Bridge }).LuminaFiles;
  if (bridge) { const error = bridge.openFile(item.path, item.title); if (error) throw new Error(error); }
  else await invoke("open_external_file", { libraryId: item.id });
}
