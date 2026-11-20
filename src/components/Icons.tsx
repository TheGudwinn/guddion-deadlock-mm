import type { ReactNode } from "react";

const base = { width: 22, height: 22, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.7, strokeLinecap: "round", strokeLinejoin: "round", "aria-hidden": true } as const;

const paths: Record<string, ReactNode> = {
  home: <path d="M4 11.2 12 4l8 7.2V19a1 1 0 0 1-1 1h-4.5v-5.5h-5V20H5a1 1 0 0 1-1-1v-7.8Z" />,
  catalog: (
    <>
      <rect x="4" y="4" width="6.5" height="6.5" rx="1.6" />
      <rect x="13.5" y="4" width="6.5" height="6.5" rx="1.6" />
      <rect x="4" y="13.5" width="6.5" height="6.5" rx="1.6" />
      <rect x="13.5" y="13.5" width="6.5" height="6.5" rx="1.6" />
    </>
  ),
  profiles: (
    <>
      <circle cx="9" cy="8.5" r="3.2" />
      <path d="M3.5 19c.5-3.2 2.7-5 5.5-5s5 1.8 5.5 5" />
      <path d="M15.6 5.6a3 3 0 0 1 0 5.8M17 14.3c1.8.5 3 2.2 3.4 4.7" />
    </>
  ),
  library: <path d="M3.5 7.5a1.5 1.5 0 0 1 1.5-1.5h4l1.8 2H19a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5v-10Z" />,
  downloads: (
    <>
      <path d="M12 4v10.5M7.5 10.5 12 15l4.5-4.5" />
      <path d="M5 19h14" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3.5v2.2M12 18.3v2.2M3.5 12h2.2M18.3 12h2.2M6 6l1.6 1.6M16.4 16.4 18 18M18 6l-1.6 1.6M7.6 16.4 6 18" />
    </>
  ),
  performance: (
    <>
      <path d="M4 17a8 8 0 1 1 16 0" />
      <path d="M12 17l3.6-5.2" />
      <circle cx="12" cy="17" r="1.1" />
    </>
  ),
  skins: (
    <>
      <circle cx="12" cy="7.5" r="3" />
      <path d="M5 20c.4-4 3.2-6.5 7-6.5s6.6 2.5 7 6.5" />
    </>
  ),
  fixes: <path d="M14.5 5.5a4 4 0 0 0-4.7 5.1L4.6 15.8a1.6 1.6 0 0 0 2.3 2.3l5.2-5.2a4 4 0 0 0 5.1-4.7l-2.4 2.4-2-.5-.5-2 2.2-2.2Z" />,
  play: <path d="M8 5.5v13l10.5-6.5L8 5.5Z" />,
  trend: <path d="M3 17l6-6 4 4 8-8M15 7h6v6" />,
  heart: <path d="M12 19.5 5.2 12.8a4.1 4.1 0 0 1 5.8-5.8l1 1 1-1a4.1 4.1 0 0 1 5.8 5.8L12 19.5Z" />,
  down: <path d="M6 9.5 12 15.5 18 9.5" />,
  up: <path d="M6 14.5 12 8.5l6 6" />,
  left: <path d="M14.5 6 8.5 12l6 6" />,
  right: <path d="M9.5 6l6 6-6 6" />,
  check: <path d="M5.5 12.5 10 17l8.5-9.5" />,
  close: <path d="M6 6l12 12M18 6 6 18" />,
  sidebar: (
    <>
      <rect x="3.5" y="4.5" width="17" height="15" rx="3" />
      <path d="M9.5 4.5v15" />
    </>
  ),
  layers: (
    <>
      <path d="M12 4 3.5 8.5 12 13l8.5-4.5L12 4Z" />
      <path d="M3.5 12.5 12 17l8.5-4.5M3.5 16 12 20.5 20.5 16" />
    </>
  ),
  bell: <path d="M6 16.5V11a6 6 0 1 1 12 0v5.5l1.5 1.5h-15L6 16.5ZM10 20.5a2 2 0 0 0 4 0" />,
  external: <path d="M9 5h10v10M19 5 8.5 15.5M5 9v10h10" />,
  trash: <path d="M5 7h14M10 7V5h4v2M7 7l.8 12h8.4L17 7M10.5 11v5M13.5 11v5" />,
  sort: <path d="M7 5v14M7 19l-3-3M7 19l3-3M17 19V5M17 5l-3 3M17 5l3 3" />,
};

export type IconName = keyof typeof paths;

export default function Icon({ name, size = 22 }: { name: IconName; size?: number }) {
  return (
    <svg {...base} width={size} height={size}>
      {paths[name]}
    </svg>
  );
}
