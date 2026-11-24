import { useMemo } from "react";
import { formatBytes } from "../lib/format";

/** Squarified treemap (Bruls et al.) over the storage tree's children.
 * ~100 lines, no dependency: the layout works on the measured node list the
 * storage page already has, so switching views costs nothing. */

export interface TreemapItem {
  label: string;
  value: number;
  /** Fill intensity 0..1 — darker means larger share. */
  tone: number;
  id: string;
}

export interface TreemapRect {
  x: number;
  y: number;
  w: number;
  h: number;
  item: TreemapItem;
}

function worst(row: TreemapItem[], length: number, total: number, area: number): number {
  // Normalise row values to the same scale as the rectangle area.
  let sum = 0;
  let max = 0;
  let min = Number.POSITIVE_INFINITY;
  for (const item of row) {
    const a = (item.value / total) * area;
    sum += a;
    if (a > max) max = a;
    if (a < min) min = a;
  }
  const s2 = sum * sum;
  const l2 = length * length;
  return Math.max((l2 * max) / s2, s2 / (l2 * min));
}

function squarify(
  items: TreemapItem[],
  rect: { x: number; y: number; w: number; h: number },
  total: number,
  out: TreemapRect[],
) {
  const area = rect.w * rect.h;
  if (items.length === 0 || area <= 0 || total <= 0) return;
  if (items.length === 1) {
    out.push({ ...rect, item: items[0] });
    return;
  }
  // Greedy squarify: extend a row along the shorter side while the worst
  // aspect ratio improves; lay the row down and recurse the remainder.
  const shorter = Math.min(rect.w, rect.h);
  let row: TreemapItem[] = [];
  let rest = items;
  for (let i = 0; i < items.length; i += 1) {
    const candidate = [...row, items[i]];
    if (row.length === 0 || worst(candidate, shorter, total, area) <= worst(row, shorter, total, area)) {
      row = candidate;
      rest = items.slice(i + 1);
    } else {
      break;
    }
  }
  const rowValue = row.reduce((sum, item) => sum + item.value, 0);
  if (rowValue <= 0) {
    row.forEach((item) => out.push({ ...rect, item }));
    return;
  }
  const thickness = (rowValue / total) * area / shorter;
  let offset = 0;
  for (const item of row) {
    const span = ((item.value / total) * area) / Math.max(thickness, 1e-9);
    if (rect.w >= rect.h) {
      out.push({ x: rect.x, y: rect.y + offset, w: thickness, h: span, item });
    } else {
      out.push({ x: rect.x + offset, y: rect.y, w: span, h: thickness, item });
    }
    offset += span;
  }
  const consumed = (rowValue / total) * area / shorter;
  const remainder =
    rect.w >= rect.h
      ? { x: rect.x + consumed, y: rect.y, w: rect.w - consumed, h: rect.h }
      : { x: rect.x, y: rect.y + consumed, w: rect.w, h: rect.h - consumed };
  const restTotal = total - rowValue;
  squarify(rest, remainder, restTotal, out);
}

export function Treemap({
  items,
  width,
  height,
  onSelect,
}: {
  items: TreemapItem[];
  width: number;
  height: number;
  onSelect: (id: string) => void;
}) {
  const rects = useMemo(() => {
    const sorted = [...items].filter((item) => item.value > 0).sort((a, b) => b.value - a.value);
    const total = sorted.reduce((sum, item) => sum + item.value, 0);
    const out: TreemapRect[] = [];
    squarify(sorted, { x: 0, y: 0, w: width, h: height }, total, out);
    return out;
  }, [items, width, height]);

  return (
    <div className="treemap" style={{ width, height }}>
      {rects.map((rect) => {
        const roomy = rect.w > 84 && rect.h > 42;
        const midSized = rect.w > 46 && rect.h > 20;
        return (
          <button
            key={rect.item.id}
            type="button"
            className="treemap-cell"
            style={{
              left: rect.x,
              top: rect.y,
              width: Math.max(rect.w - 2, 0),
              height: Math.max(rect.h - 2, 0),
              background: `color-mix(in oklab, var(--nex-accent, #e05f21) ${Math.round(28 + rect.item.tone * 68)}%, #f6e8d2)`,
              color: rect.item.tone > 0.55 ? "#fff" : "var(--ink)",
            }}
            title={`${rect.item.label} · ${formatBytes(rect.item.value)}`}
            onClick={() => onSelect(rect.item.id)}
          >
            {roomy ? (
              <span className="treemap-label">
                <b>{rect.item.label}</b>
                <small>{formatBytes(rect.item.value)}</small>
              </span>
            ) : midSized ? (
              <span className="treemap-label is-tiny">{rect.item.label}</span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
