/**
 * 分支模糊过滤（纯函数，ticket 14）：
 * 子序列匹配，大小写不敏感。命中返回该分支名中匹配字符的下标（用于
 * 高亮），未命中返回 null。顺序保持原列表顺序，不做相关度排序。
 */

export type BranchMatch = { branch: string; indices: number[] };

/** query 的每个字符是否按顺序出现在 branch 中；是则返回命中的下标。 */
function matchIndices(branch: string, query: string): number[] | null {
  const haystack = branch.toLowerCase();
  const needle = query.toLowerCase();
  const indices: number[] = [];
  let from = 0;
  for (const ch of needle) {
    const idx = haystack.indexOf(ch, from);
    if (idx === -1) return null;
    indices.push(idx);
    from = idx + 1;
  }
  return indices;
}

export function filterBranches(branches: string[], query: string): BranchMatch[] {
  const needle = query.trim();
  if (needle === "") {
    return branches.map((branch) => ({ branch, indices: [] }));
  }
  const result: BranchMatch[] = [];
  for (const branch of branches) {
    const indices = matchIndices(branch, needle);
    if (indices !== null) result.push({ branch, indices });
  }
  return result;
}
