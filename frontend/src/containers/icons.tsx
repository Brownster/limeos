/*
 * The few extra icons the Containers page needs, on the same 24 px stroke
 * grid as src/ui/icons.tsx. Shapes follow Lucide (https://lucide.dev), ISC
 * License, Copyright (c) Lucide Contributors. Decorative: the adjacent text or
 * the control's label carries the meaning.
 */
import type { ReactNode } from "react";

export type ContainerIconName = "play" | "square" | "rotate" | "more" | "logs" | "copy" | "check" | "search" | "close" | "layers";

const shapes: Record<ContainerIconName, ReactNode> = {
  play: <path d="M6 3l14 9-14 9V3z" />,
  square: <rect width="14" height="14" x="5" y="5" rx="2" />,
  rotate: <><path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" /><path d="M21 3v5h-5" /></>,
  more: <><circle cx="12" cy="12" r="1" /><circle cx="12" cy="5" r="1" /><circle cx="12" cy="19" r="1" /></>,
  logs: <><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" /><path d="M14 2v4a2 2 0 0 0 2 2h4" /><path d="M16 13H8M16 17H8M10 9H8" /></>,
  copy: <><rect width="14" height="14" x="8" y="8" rx="2" /><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" /></>,
  check: <path d="M20 6 9 17l-5-5" />,
  search: <><circle cx="11" cy="11" r="8" /><path d="m21 21-4.3-4.3" /></>,
  close: <><path d="M18 6 6 18" /><path d="m6 6 12 12" /></>,
  layers: <><path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z" /><path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65" /></>,
};

export function CIcon({ name, size = 16, className }: { name: ContainerIconName; size?: number; className?: string }) {
  return (
    <svg className={className ? `icon ${className}` : "icon"} width={size} height={size} viewBox="0 0 24 24" fill="none"
      stroke="currentColor" strokeWidth={1.75} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
      {shapes[name]}
    </svg>
  );
}
