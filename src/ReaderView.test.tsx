import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import ReaderView from "./ReaderView";
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
afterEach(cleanup);

it("keeps saving position after selecting the already-active chapter", async () => {
  bridge.invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "open_local_book_chapter") return { id: "text", title: "Chapter", text: "A long book." };
    if (command === "bookmarks") return [];
    return {};
  });
  const { container } = render(<ReaderView libraryId="book" initialScroll={0} book={{ title: "Book", chapters: [{ id: "text", title: "Chapter" }] }}
    bookPath="/book.txt" fallbackTitle="Book" format="txt" chapterIndex={0} progressFraction={0} onChapterChange={vi.fn()} onClose={vi.fn()} />);
  const body = container.querySelector(".reader-body")!;
  Object.defineProperties(body, { scrollHeight: { value: 1000 }, clientHeight: { value: 200 } });
  await screen.findByText("A long book.");
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("save_reading_progress", expect.anything()));
  await act(async () => { body.scrollTop = 240; fireEvent.scroll(body); });
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("save_reading_progress", expect.objectContaining({ fraction: 0.3 })));
  await userEvent.click(screen.getByRole("button", { name: "章节目录" }));
  await userEvent.click(screen.getByRole("button", { name: /1\s*Chapter/ }));
  await act(async () => { body.scrollTop = 640; fireEvent.scroll(body); });
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("save_reading_progress", expect.objectContaining({ fraction: 0.8 })));
});
