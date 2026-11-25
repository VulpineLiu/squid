import { useMemo } from "react";
import type { Category, ScanResult } from "../types";
import { CATEGORY_ORDER, CategoryCard } from "./ui";

type CategoryGroup = { total: number; count: number; items: number[]; selected: number };

function grouped(result: ScanResult) {
  const groups = new Map<Category, CategoryGroup>();
  result.items.forEach((item, index) => {
    const group = groups.get(item.category) ?? {
      total: 0,
      count: 0,
      items: [],
      selected: 0,
    };
    group.total += item.onDisk;
    group.count += 1;
    group.items.push(index);
    group.selected += 1;
    groups.set(item.category, group);
  });
  return groups;
}

/* Overview 页的分类速览：紧凑卡片一排，点击跳转空间清理页。 */
export function CategorySummary({
  result,
  selected,
  onToggle,
  onInspect,
}: {
  result: ScanResult;
  selected: Set<number>;
  onToggle: (category: Category) => void;
  onInspect: (category: Category) => void;
}) {
  const groups = useMemo(() => grouped(result), [result]);
  return (
    <div className="category-grid">
      {CATEGORY_ORDER.filter((category) => groups.has(category)).map((category) => {
        const group = groups.get(category)!;
        const indexes = group.items.filter((index) => !result.items[index].lock);
        const allSelected = indexes.length > 0 && indexes.every((index) => selected.has(index));
        return (
          <CategoryCard
            key={category}
            category={category}
            total={group.total}
            count={group.count}
            selected={allSelected}
            disabled={indexes.length === 0}
            onToggle={() => onToggle(category)}
            onInspect={() => onInspect(category)}
          />
        );
      })}
    </div>
  );
}
