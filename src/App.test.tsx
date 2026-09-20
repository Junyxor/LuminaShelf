import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

const bridge = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  back: vi.fn(),
}));
vi.mock("@tauri-apps/api/app", () => ({ onBackButtonPress: bridge.back }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => void) => {
    bridge.listeners.set(name, callback);
    return () => bridge.listeners.delete(name);
  }),
}));

const book = { id: "local-1", title: "本地阅读", authors: ["作者"], path: "/books/local.txt", format: "txt", sizeBytes: 12 };
const task = { id: "public:1:txt", providerId: "public", bookId: "1", title: "待下载的书", format: "txt", destination: "/books/download.txt", state: "paused", downloadedBytes: 4, totalBytes: 12 };
let tasks: unknown[] = [];
let books: unknown[] = [];

beforeEach(() => {
  localStorage.clear();
  bridge.listeners.clear();
  bridge.invoke.mockReset();
  bridge.back.mockReset().mockResolvedValue({ unregister: vi.fn(async () => {}) });
  tasks = [task];
  books = [book];
  bridge.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "core_status": return { name: "LuminaShelf", version: "1.0.0", rustCore: true, networkStack: "Rust", platform: "windows" };
      case "provider_descriptors": return ["public", "second"].map((id) => ({ id, name: id, capabilities: { searchable: true, downloadable: true, authenticated: false } }));
      case "zlibrary_restore_session": return { status: { signedIn: false, secureSessionStorage: false } };
      case "download_tasks": return tasks;
      case "library_items": return books;
      default: throw new Error(`Unexpected IPC: ${command}`);
    }
  });
});
afterEach(cleanup);

async function openPage(name: string) {
  await screen.findByText("Rust Core online");
  await userEvent.click(screen.getAllByRole("button", { name })[0]);
}

describe("library and download workflows across the IPC boundary", () => {
  it("restores a paused task, resumes it, and adds the completed book to the existing library", async () => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (command: string, args: unknown) => {
      if (command === "enqueue_download") {
        books = [book, { ...book, id: "download", title: "待下载的书", path: "/books/download.txt" }];
        queueMicrotask(() => {
          bridge.listeners.get("download-task-updated")?.({ payload: { ...task, state: "completed" } });
          bridge.listeners.get("library-updated")?.({ payload: {} });
        });
        return { ...task, state: "queued" };
      }
      return original(command, args);
    });
    render(<App />);
    await openPage("下载");
    expect(await screen.findByText("已暂停")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "继续" }));
    await screen.findByText("已完成");
    await openPage("书库");
    expect(screen.getByRole("heading", { name: "本地阅读" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "待下载的书" })).toBeTruthy();
  });

  it("allows cancellation while connecting and waits for the backend state", async () => {
    tasks = [{ ...task, state: "connecting" }];
    let finish: (value: boolean) => void = () => {};
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((command: string, args: unknown) => command === "cancel_download"
      ? new Promise<boolean>((resolve) => { finish = resolve; }) : original(command, args));
    render(<App />);
    await openPage("下载");
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.getByRole("button", { name: "取消" }).hasAttribute("disabled")).toBe(true);
    await act(async () => {
      tasks = [{ ...task, state: "cancelled" }];
      bridge.listeners.get("download-task-updated")?.({ payload: tasks[0] });
      finish(true);
    });
    expect(await screen.findByText("已取消")).toBeTruthy();
    expect(screen.queryByText("已暂停")).toBeNull();
  });

  it("keeps existing books when importing, and surfaces partial import errors", async () => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (command: string, args: unknown) => command === "import_library_files"
      ? { items: [book, { ...book, id: "new", title: "新导入", path: "/managed/new.txt" }], imported: 1, errors: ["坏文件无法读取"] }
      : original(command, args));
    render(<App />);
    await openPage("书库");
    await userEvent.click(screen.getByRole("button", { name: "导入电子书" }));
    expect(await screen.findByRole("heading", { name: "新导入" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "本地阅读" })).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("坏文件无法读取");
    expect(screen.getByRole("status").textContent).toContain("已导入 1 本");
    await userEvent.type(screen.getByRole("textbox", { name: "筛选书库" }), "新导入");
    expect(screen.queryByRole("heading", { name: "本地阅读" })).toBeNull();
  });

  it("does not show old-provider results after switching providers during a search", async () => {
    let finish: (value: unknown) => void = () => {};
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((command: string, args: unknown) => command === "search_books"
      ? new Promise((resolve) => { finish = resolve; }) : original(command, args));
    render(<App />);
    await openPage("搜索");
    await userEvent.type(screen.getByRole("textbox", { name: "搜索电子书" }), "Novel");
    await userEvent.click(screen.getAllByRole("button", { name: "搜索" }).find((button) => button.classList.contains("primary-button"))!);
    await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("search_books", expect.objectContaining({ providerId: "public" })));
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "数据源" }), "second");
    await act(async () => { finish({ items: [{ id: "old", title: "过期结果", authors: [], availableFormats: ["txt"] }], page: 1, hasNext: false }); });
    expect(screen.queryByText("过期结果")).toBeNull();
    expect(screen.getByText("从一本书开始")).toBeTruthy();
  });
});

it("uses Android file import without desktop directory inputs and registers Back", async () => {
  const original = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (command: string, args: unknown) => command === "core_status"
    ? { name: "LuminaShelf", version: "1.0.0", rustCore: true, networkStack: "Rust", platform: "android" }
    : original(command, args));
  render(<App />);
  await openPage("书库");
  expect(screen.getByRole("button", { name: "导入电子书" })).toBeTruthy();
  expect(screen.queryByRole("textbox", { name: "书库目录" })).toBeNull();
  expect(screen.queryByRole("button", { name: "选择目录" })).toBeNull();
  await waitFor(() => expect(bridge.back).toHaveBeenCalled());
  await act(async () => { bridge.back.mock.calls.at(-1)![0]({ canGoBack: false }); });
  expect(screen.getByRole("heading", { name: "总览", level: 1 })).toBeTruthy();
});
