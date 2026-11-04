// 可搜索 Combobox（票 14 / 21 / 24 共用）：Popover + cmdk，过滤走前端模糊
// 匹配（filterBranches 子序列匹配，与分支/构件选择器同款），命中片段高亮。
// 单选模式选中后收起下拉；多选模式勾选后保持展开，可继续搜索/调整。

import { useState, type ReactNode } from "react";
import { ChevronsUpDownIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { filterBranches } from "@/lib/branchFilter";

/** 选项文本渲染：命中字符按下标高亮（模糊搜索匹配片段）。 */
export function HighlightedText({ text, indices }: { text: string; indices: number[] }) {
  if (indices.length === 0) return <>{text}</>;
  const parts: ReactNode[] = [];
  let cursor = 0;
  for (const idx of indices) {
    if (idx > cursor) parts.push(text.slice(cursor, idx));
    parts.push(
      <span key={idx} data-highlight className="font-semibold text-primary">
        {text[idx]}
      </span>,
    );
    cursor = idx + 1;
  }
  if (cursor < text.length) parts.push(text.slice(cursor));
  return <>{parts}</>;
}

function FuzzyCombobox({
  items,
  selected,
  onSelect,
  multiple = false,
  disabled = false,
  ariaLabel,
  searchPlaceholder,
  emptyText,
  renderValue,
  className = "",
  testId,
}: {
  items: string[];
  /** 当前选中集合：触发器回显（renderValue）与多选勾选态。 */
  selected: string[];
  onSelect: (name: string) => void;
  /** 多选：勾选后下拉保持展开；默认单选，选中即收起。 */
  multiple?: boolean;
  disabled?: boolean;
  ariaLabel: string;
  searchPlaceholder: string;
  emptyText: string;
  /** 触发器展示内容（默认回显选中集合，顿号连接）。 */
  renderValue?: (selected: string[]) => ReactNode;
  className?: string;
  testId?: string;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const matches = filterBranches(items, query);
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        // 关闭时清空搜索关键字，下次打开展示完整列表。
        if (!next) setQuery("");
      }}
    >
      <PopoverTrigger asChild disabled={disabled}>
        <Button
          variant="outline"
          role="combobox"
          aria-label={ariaLabel}
          aria-expanded={open}
          data-testid={testId}
          className={`justify-between font-normal ${className}`}
        >
          <span className="min-w-0 truncate">
            {renderValue ? renderValue(selected) : selected.join("、")}
          </span>
          <ChevronsUpDownIcon className="size-4 shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-(--radix-popover-trigger-width) p-0">
        {/* 过滤由前端模糊匹配完成，关闭 cmdk 内置过滤。 */}
        <Command shouldFilter={false}>
          <CommandInput
            value={query}
            onValueChange={setQuery}
            placeholder={searchPlaceholder}
          />
          <CommandList>
            {matches.map(({ branch: name, indices }) => {
              const checked = selected.includes(name);
              return (
                <CommandItem
                  key={name}
                  value={name}
                  onSelect={() => {
                    onSelect(name);
                    if (!multiple) setOpen(false);
                  }}
                >
                  {multiple && (
                    <input
                      type="checkbox"
                      aria-label={name}
                      checked={checked}
                      onChange={() => {}}
                      className="pointer-events-none"
                    />
                  )}
                  <HighlightedText text={name} indices={indices} />
                </CommandItem>
              );
            })}
            {matches.length === 0 && (
              <div className="py-6 text-center text-sm text-muted-foreground">{emptyText}</div>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

export default FuzzyCombobox;
