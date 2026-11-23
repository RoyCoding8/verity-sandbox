import type { SVGProps } from "react";

export interface IconProps
  extends Omit<SVGProps<SVGSVGElement>, "width" | "height"> {
  size?: number;
  strokeWidth?: number;
}

const DEFAULT_SIZE = 16;
const DEFAULT_STROKE = 1.75;

function baseProps(
  size: number,
  strokeWidth: number,
  rest: Omit<IconProps, "size" | "strokeWidth">
): SVGProps<SVGSVGElement> {
  return {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth,
    strokeLinecap: "round",
    strokeLinejoin: "round",
    "aria-hidden": true,
    ...rest,
  };
}

export function VerityLogoMark({
  size = 18,
  strokeWidth = 2,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M12 2.5 20 6.5v5.75c0 4.6-3.4 8.2-8 9.25-4.6-1.05-8-4.65-8-9.25V6.5L12 2.5Z" />
      <path d="m8.75 12.1 2.25 2.35 4.5-4.9" />
    </svg>
  );
}

export function PlusIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </svg>
  );
}

export function SunIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2" />
      <path d="M12 20v2" />
      <path d="m4.93 4.93 1.41 1.41" />
      <path d="m17.66 17.66 1.41 1.41" />
      <path d="M2 12h2" />
      <path d="M20 12h2" />
      <path d="m6.34 17.66-1.41 1.41" />
      <path d="m19.07 4.93-1.41 1.41" />
    </svg>
  );
}

export function MoonIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
    </svg>
  );
}

export function SettingsIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2Z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

export function CloseIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </svg>
  );
}

export function CheckIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M20 6 9 17l-5-5" />
    </svg>
  );
}

export function AlertIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z" />
      <path d="M12 9v4" />
      <path d="M12 17h.01" />
    </svg>
  );
}

export function XCircleIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <circle cx="12" cy="12" r="10" />
      <path d="m15 9-6 6" />
      <path d="m9 9 6 6" />
    </svg>
  );
}

export function InfoIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <circle cx="12" cy="12" r="10" />
      <path d="M12 16v-4" />
      <path d="M12 8h.01" />
    </svg>
  );
}

export function GitBranchIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <line x1="6" x2="6" y1="3" y2="15" />
      <circle cx="18" cy="6" r="3" />
      <circle cx="6" cy="18" r="3" />
      <path d="M18 9a9 9 0 0 1-9 9" />
    </svg>
  );
}

export function GitMergeIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <circle cx="18" cy="18" r="3" />
      <circle cx="6" cy="6" r="3" />
      <path d="M6 21V9a9 9 0 0 0 9 9" />
    </svg>
  );
}

export function GitForkIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <circle cx="12" cy="18" r="3" />
      <circle cx="6" cy="6" r="3" />
      <circle cx="18" cy="6" r="3" />
      <path d="M18 9v2c0 .6-.4 1-1 1H7c-.6 0-1-.4-1-1V9" />
      <path d="M12 12v3" />
    </svg>
  );
}

export function CopyIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
      <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
    </svg>
  );
}

export function DownloadIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <polyline points="7 10 12 15 17 10" />
      <line x1="12" x2="12" y1="15" y2="3" />
    </svg>
  );
}

export function EditIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
      <path d="m15 5 4 4" />
    </svg>
  );
}

export function TrashIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M3 6h18" />
      <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
      <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
      <line x1="10" x2="10" y1="11" y2="17" />
      <line x1="14" x2="14" y1="11" y2="17" />
    </svg>
  );
}

export function SearchIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <circle cx="11" cy="11" r="8" />
      <path d="m21 21-4.3-4.3" />
    </svg>
  );
}

export function PaperclipIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" />
    </svg>
  );
}

export function SendIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="m22 2-7 20-4-9-9-4Z" />
      <path d="M22 2 11 13" />
    </svg>
  );
}

export function StopIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <rect width="12" height="12" x="6" y="6" rx="2" fill="currentColor" />
    </svg>
  );
}

export function ChevronDownIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

export function ChevronRightIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="m9 18 6-6-6-6" />
    </svg>
  );
}

export function PanelRightIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <rect width="18" height="18" x="3" y="3" rx="2" />
      <path d="M15 3v18" />
    </svg>
  );
}

export function TerminalIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <polyline points="4 17 10 11 4 5" />
      <line x1="12" x2="20" y1="19" y2="19" />
    </svg>
  );
}

export function ShieldCheckIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1Z" />
      <path d="m9 12 2 2 4-4" />
    </svg>
  );
}

export function SparklesIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0Z" />
      <path d="M20 3v4" />
      <path d="M22 5h-4" />
    </svg>
  );
}

export function RefreshIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
      <path d="M8 16H3v5" />
    </svg>
  );
}

export function ClockIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <circle cx="12" cy="12" r="10" />
      <polyline points="12 6 12 12 16 14" />
    </svg>
  );
}

export function CpuIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <rect width="16" height="16" x="4" y="4" rx="2" />
      <rect width="6" height="6" x="9" y="9" rx="1" />
      <path d="M15 2v2" />
      <path d="M15 20v2" />
      <path d="M2 15h2" />
      <path d="M2 9h2" />
      <path d="M20 15h2" />
      <path d="M20 9h2" />
      <path d="M9 2v2" />
      <path d="M9 20v2" />
    </svg>
  );
}

export function FolderIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
    </svg>
  );
}

export function MessageIcon({
  size = DEFAULT_SIZE,
  strokeWidth = DEFAULT_STROKE,
  ...rest
}: IconProps) {
  return (
    <svg {...baseProps(size, strokeWidth, rest)}>
      <path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z" />
    </svg>
  );
}
