import { describe, expect, it } from "vitest";
import { filterBranches } from "./branchFilter";

describe("filterBranches（分支模糊过滤）", () => {
  it("空关键字返回全部分支且无高亮索引", () => {
    const branches = ["main", "develop"];
    expect(filterBranches(branches, "")).toEqual([
      { branch: "main", indices: [] },
      { branch: "develop", indices: [] },
    ]);
  });

  it("空白关键字等同空关键字", () => {
    expect(filterBranches(["main"], "   ")).toEqual([{ branch: "main", indices: [] }]);
  });

  it("子序列匹配大小写不敏感：rel 命中 release/2.1 与 hotfix-release", () => {
    const result = filterBranches(["release/2.1", "hotfix-release", "main"], "rel");
    expect(result.map((r) => r.branch)).toEqual(["release/2.1", "hotfix-release"]);
  });

  it("命中片段返回高亮索引（大小写不敏感时索引指向原字符串）", () => {
    // "rel" 依次命中 "Release/2.1" 的下标 0、1、2（首字母大写）。
    expect(filterBranches(["Release/2.1"], "REL")).toEqual([
      { branch: "Release/2.1", indices: [0, 1, 2] },
    ]);
  });

  it("非连续子序列也可命中：hfx 命中 hotfix-release", () => {
    // h(0) f(3) x(5) → "hotfix-release" 的非连续子序列。
    const result = filterBranches(["hotfix-release"], "hfx");
    expect(result).toEqual([{ branch: "hotfix-release", indices: [0, 3, 5] }]);
  });

  it("不匹配的分支被过滤掉", () => {
    expect(filterBranches(["main", "develop"], "zzz")).toEqual([]);
  });

  it("保持原列表顺序（不做相关度排序）", () => {
    const result = filterBranches(["z-rel", "a-release", "release-b"], "rel");
    expect(result.map((r) => r.branch)).toEqual(["z-rel", "a-release", "release-b"]);
  });
});
