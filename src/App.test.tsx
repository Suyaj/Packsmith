import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import App from "./App";
import { resources } from "@/i18n";
import "@/i18n";

function renderApp(initialPath: string) {
  return render(
    <MemoryRouter initialEntries={[initialPath]}>
      <App />
    </MemoryRouter>,
  );
}

describe("应用骨架", () => {
  it("默认进入首页，标题来自中文资源", () => {
    renderApp("/");

    expect(
      screen.getByRole("heading", { level: 1, name: "首页" }),
    ).toBeInTheDocument();
  });

  it.each(["项目管理", "服务器管理", "设置"] as const)(
    "可通过导航进入%s页，页面标题来自中文资源",
    async (title) => {
      renderApp("/");
      await userEvent.click(screen.getByRole("link", { name: title }));

      expect(
        screen.getByRole("heading", { level: 1, name: title }),
      ).toBeInTheDocument();
    },
  );

  it("i18n 资源仅包含中文", () => {
    expect(Object.keys(resources)).toEqual(["zh"]);
  });
});

