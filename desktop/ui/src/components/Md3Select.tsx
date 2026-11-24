import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { CheckIcon, ChevronDownIcon } from "./Icons";
import "./Md3Select.css";

export interface Md3SelectOption {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
}

export interface Md3SelectProps {
  value: string;
  options: readonly Md3SelectOption[];
  onChange: (value: string) => void;
  ariaLabel: string;
  placeholder?: string;
  disabled?: boolean;
  variant?: "pill" | "tonal" | "field";
  placement?: "bottom" | "top";
  align?: "start" | "end";
  leadingIcon?: ReactNode;
  className?: string;
  id?: string;
}

type ResolvedPlacement = "bottom" | "top";

interface MenuGeometry {
  readonly placement: ResolvedPlacement;
  readonly top?: number;
  readonly bottom?: number;
  readonly left?: number;
  readonly right?: number;
  readonly minWidth: number;
  readonly maxHeight: number;
}

const MENU_GAP_PX = 6;
const VIEWPORT_MARGIN_PX = 12;
const MAX_MENU_HEIGHT_PX = 280;
const MIN_MENU_WIDTH_PX = 180;
const TYPEAHEAD_RESET_MS = 500;

function computeMenuGeometry(
  rect: DOMRect,
  viewport: { readonly width: number; readonly height: number },
  preferredPlacement: ResolvedPlacement,
  align: "start" | "end"
): MenuGeometry {
  const spaceBelow = viewport.height - rect.bottom - MENU_GAP_PX - VIEWPORT_MARGIN_PX;
  const spaceAbove = rect.top - MENU_GAP_PX - VIEWPORT_MARGIN_PX;

  let resolvedPlacement: ResolvedPlacement = preferredPlacement;
  if (
    preferredPlacement === "bottom" &&
    spaceBelow < MAX_MENU_HEIGHT_PX &&
    spaceAbove > spaceBelow
  ) {
    resolvedPlacement = "top";
  } else if (
    preferredPlacement === "top" &&
    spaceAbove < MAX_MENU_HEIGHT_PX &&
    spaceBelow > spaceAbove
  ) {
    resolvedPlacement = "bottom";
  }

  const availableVertical = resolvedPlacement === "bottom" ? spaceBelow : spaceAbove;
  const maxHeight = Math.max(0, Math.floor(Math.min(MAX_MENU_HEIGHT_PX, availableVertical)));

  const maxViewportWidth = Math.max(0, viewport.width - VIEWPORT_MARGIN_PX * 2);
  const minWidth = Math.min(Math.max(Math.round(rect.width), MIN_MENU_WIDTH_PX), maxViewportWidth);

  const vertical =
    resolvedPlacement === "bottom"
      ? { top: Math.round(rect.bottom + MENU_GAP_PX) }
      : { bottom: Math.round(viewport.height - rect.top + MENU_GAP_PX) };

  const maxOffset = Math.max(VIEWPORT_MARGIN_PX, viewport.width - VIEWPORT_MARGIN_PX - minWidth);
  const horizontal =
    align === "end"
      ? {
          right: Math.max(
            VIEWPORT_MARGIN_PX,
            Math.min(Math.round(viewport.width - rect.right), maxOffset)
          ),
        }
      : {
          left: Math.max(VIEWPORT_MARGIN_PX, Math.min(Math.round(rect.left), maxOffset)),
        };

  return {
    placement: resolvedPlacement,
    ...vertical,
    ...horizontal,
    minWidth,
    maxHeight,
  };
}

function findNextEnabledIndex(
  options: readonly Md3SelectOption[],
  startIndex: number,
  direction: 1 | -1
): number {
  const count = options.length;
  if (count === 0) {
    return -1;
  }
  for (let step = 1; step <= count; step += 1) {
    const candidate = (startIndex + direction * step + count) % count;
    if (!options[candidate]?.disabled) {
      return candidate;
    }
  }
  return -1;
}

function findEdgeEnabledIndex(
  options: readonly Md3SelectOption[],
  edge: "first" | "last"
): number {
  if (edge === "first") {
    return options.findIndex((opt) => !opt.disabled);
  }
  for (let i = options.length - 1; i >= 0; i -= 1) {
    if (!options[i]?.disabled) {
      return i;
    }
  }
  return -1;
}

function findTypeaheadIndex(
  options: readonly Md3SelectOption[],
  query: string,
  currentIndex: number
): number {
  const count = options.length;
  if (count === 0 || query.length === 0) {
    return -1;
  }
  const normalized = query.toLowerCase();
  const start =
    normalized.length === 1
      ? currentIndex >= 0
        ? (currentIndex + 1) % count
        : 0
      : Math.max(0, currentIndex);

  for (let offset = 0; offset < count; offset += 1) {
    const index = (start + offset) % count;
    const option = options[index];
    if (option && !option.disabled && option.label.toLowerCase().startsWith(normalized)) {
      return index;
    }
  }
  return -1;
}

export function Md3Select({
  value,
  options,
  onChange,
  ariaLabel,
  placeholder,
  disabled = false,
  variant = "pill",
  placement = "bottom",
  align = "start",
  leadingIcon,
  className,
  id,
}: Md3SelectProps) {
  const [open, setOpen] = useState(false);
  const [highlightedIndex, setHighlightedIndex] = useState(-1);
  const [menuGeometry, setMenuGeometry] = useState<MenuGeometry | null>(null);

  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const typeaheadRef = useRef<{ buffer: string; timer: number | null }>({
    buffer: "",
    timer: null,
  });
  const listboxId = useId();

  const selectedIndex = useMemo(
    () => options.findIndex((opt) => opt.value === value),
    [options, value]
  );
  const selectedOption = selectedIndex >= 0 ? options[selectedIndex] : undefined;

  const clearTypeahead = useCallback(() => {
    if (typeaheadRef.current.timer !== null) {
      window.clearTimeout(typeaheadRef.current.timer);
      typeaheadRef.current.timer = null;
    }
    typeaheadRef.current.buffer = "";
  }, []);

  useEffect(() => clearTypeahead, [clearTypeahead]);

  const updateMenuGeometry = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) {
      return;
    }
    const rect = trigger.getBoundingClientRect();
    setMenuGeometry(
      computeMenuGeometry(
        rect,
        { width: window.innerWidth, height: window.innerHeight },
        placement,
        align
      )
    );
  }, [align, placement]);

  const closeMenu = useCallback(
    (restoreFocus: boolean) => {
      clearTypeahead();
      setOpen(false);
      setHighlightedIndex(-1);
      setMenuGeometry(null);
      if (restoreFocus) {
        triggerRef.current?.focus();
      }
    },
    [clearTypeahead]
  );

  const openMenu = useCallback(
    (fallbackEdge: "first" | "last" = "first") => {
      if (disabled) {
        return;
      }
      const initialIndex =
        selectedIndex >= 0 && !options[selectedIndex]?.disabled
          ? selectedIndex
          : findEdgeEnabledIndex(options, fallbackEdge);
      setHighlightedIndex(initialIndex);
      setOpen(true);
    },
    [disabled, options, selectedIndex]
  );

  const selectOption = useCallback(
    (option: Md3SelectOption) => {
      if (option.disabled) {
        return;
      }
      if (option.value !== value) {
        onChange(option.value);
      }
      closeMenu(true);
    },
    [closeMenu, onChange, value]
  );

  useLayoutEffect(() => {
    if (!open) {
      return;
    }
    updateMenuGeometry();
  }, [open, options.length, updateMenuGeometry]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) {
        return;
      }
      const insideRoot = rootRef.current?.contains(target) ?? false;
      const insideMenu = menuRef.current?.contains(target) ?? false;
      if (!insideRoot && !insideMenu) {
        closeMenu(false);
      }
    };
    const handleDocumentKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        closeMenu(true);
      }
    };
    const handleWindowScroll = (event: Event) => {
      if (event.target === menuRef.current) {
        return;
      }
      updateMenuGeometry();
    };
    const handleWindowResize = () => {
      updateMenuGeometry();
    };

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleDocumentKeyDown, { capture: true });
    window.addEventListener("scroll", handleWindowScroll, true);
    window.addEventListener("resize", handleWindowResize);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleDocumentKeyDown, { capture: true });
      window.removeEventListener("scroll", handleWindowScroll, true);
      window.removeEventListener("resize", handleWindowResize);
    };
  }, [closeMenu, open, updateMenuGeometry]);

  useEffect(() => {
    if (!open || highlightedIndex < 0) {
      return;
    }
    const activeEl = optionRefs.current[highlightedIndex];
    activeEl?.scrollIntoView({ block: "nearest" });
  }, [highlightedIndex, open]);

  useEffect(() => {
    if (disabled && open) {
      clearTypeahead();
      setOpen(false);
      setHighlightedIndex(-1);
      setMenuGeometry(null);
    }
  }, [clearTypeahead, disabled, open]);

  useEffect(() => {
    optionRefs.current.length = options.length;
    if (!open) {
      return;
    }
    if (
      highlightedIndex >= options.length ||
      (highlightedIndex >= 0 && options[highlightedIndex]?.disabled)
    ) {
      const nextIndex =
        selectedIndex >= 0 && !options[selectedIndex]?.disabled
          ? selectedIndex
          : findEdgeEnabledIndex(options, "first");
      setHighlightedIndex(nextIndex);
    }
  }, [highlightedIndex, open, options, selectedIndex]);

  const handleTypeahead = (char: string, event: KeyboardEvent<HTMLButtonElement>) => {
    if (typeaheadRef.current.timer !== null) {
      window.clearTimeout(typeaheadRef.current.timer);
    }
    const nextBuffer = typeaheadRef.current.buffer + char;
    typeaheadRef.current.buffer = nextBuffer;
    typeaheadRef.current.timer = window.setTimeout(() => {
      typeaheadRef.current.buffer = "";
      typeaheadRef.current.timer = null;
    }, TYPEAHEAD_RESET_MS);

    const anchorIndex = open ? highlightedIndex : selectedIndex;
    let matchIndex = findTypeaheadIndex(options, nextBuffer, anchorIndex);
    if (matchIndex < 0 && nextBuffer.length > 1) {
      typeaheadRef.current.buffer = char;
      matchIndex = findTypeaheadIndex(options, char, anchorIndex);
    }
    if (matchIndex < 0) {
      return;
    }

    event.preventDefault();
    if (open) {
      setHighlightedIndex(matchIndex);
    } else {
      const matched = options[matchIndex];
      if (matched && matched.value !== value) {
        onChange(matched.value);
      }
    }
  };

  const handleTriggerKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) {
      return;
    }

    switch (event.key) {
      case "ArrowDown": {
        event.preventDefault();
        if (!open) {
          openMenu("first");
        } else {
          setHighlightedIndex((prev) =>
            findNextEnabledIndex(options, prev < 0 ? -1 : prev, 1)
          );
        }
        break;
      }
      case "ArrowUp": {
        event.preventDefault();
        if (!open) {
          openMenu("last");
        } else {
          setHighlightedIndex((prev) =>
            findNextEnabledIndex(
              options,
              prev < 0 ? options.length : prev,
              -1
            )
          );
        }
        break;
      }
      case "Home": {
        if (open) {
          event.preventDefault();
          setHighlightedIndex(findEdgeEnabledIndex(options, "first"));
        }
        break;
      }
      case "End": {
        if (open) {
          event.preventDefault();
          setHighlightedIndex(findEdgeEnabledIndex(options, "last"));
        }
        break;
      }
      case "Enter":
      case " ": {
        if (event.key === " " && typeaheadRef.current.buffer.length > 0) {
          handleTypeahead(" ", event);
          break;
        }
        event.preventDefault();
        if (!open) {
          openMenu("first");
        } else if (highlightedIndex >= 0) {
          const target = options[highlightedIndex];
          if (target && !target.disabled) {
            selectOption(target);
          }
        }
        break;
      }
      case "Escape": {
        if (open) {
          event.preventDefault();
          event.stopPropagation();
          closeMenu(true);
        }
        break;
      }
      case "Tab": {
        if (open) {
          closeMenu(false);
        }
        break;
      }
      default: {
        if (
          event.key.length === 1 &&
          !event.ctrlKey &&
          !event.metaKey &&
          !event.altKey
        ) {
          handleTypeahead(event.key, event);
        }
        break;
      }
    }
  };

  const activeDescendantId =
    open && highlightedIndex >= 0
      ? `${listboxId}-opt-${highlightedIndex}`
      : undefined;

  const rootClasses = [
    "md3-select-root",
    `md3-select-root--${variant}`,
    open ? "is-open" : "",
    disabled ? "is-disabled" : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  const resolvedPlacement = menuGeometry?.placement ?? placement;
  const menuClasses = [
    "md3-select-menu",
    `md3-select-menu--${resolvedPlacement}`,
    `md3-select-menu--align-${align}`,
  ].join(" ");

  const menuStyle: CSSProperties | undefined = menuGeometry
    ? {
        top: menuGeometry.top,
        bottom: menuGeometry.bottom,
        left: menuGeometry.left,
        right: menuGeometry.right,
        minWidth: `${menuGeometry.minWidth}px`,
        maxHeight: `${menuGeometry.maxHeight}px`,
      }
    : undefined;

  const triggerValueText = selectedOption ? selectedOption.label : placeholder ?? "";

  const menuPortal =
    open && menuGeometry && typeof document !== "undefined"
      ? createPortal(
          <div
            ref={menuRef}
            id={listboxId}
            role="listbox"
            aria-label={ariaLabel}
            className={menuClasses}
            style={menuStyle}
            onMouseDown={(event) => {
              event.preventDefault();
            }}
          >
            {options.length === 0 ? (
              <div
                className="md3-select-option is-disabled"
                role="option"
                aria-disabled="true"
              >
                <span className="md3-select-option-text">
                  <span className="md3-select-option-label">No options available</span>
                </span>
              </div>
            ) : (
              options.map((option, index) => {
                const isSelected = option.value === value;
                const isHighlighted = index === highlightedIndex;
                const optionClasses = [
                  "md3-select-option",
                  isSelected ? "is-selected" : "",
                  isHighlighted ? "is-highlighted" : "",
                  option.disabled ? "is-disabled" : "",
                ]
                  .filter(Boolean)
                  .join(" ");

                return (
                  <button
                    key={option.value}
                    ref={(el) => {
                      optionRefs.current[index] = el;
                    }}
                    id={`${listboxId}-opt-${index}`}
                    type="button"
                    role="option"
                    tabIndex={-1}
                    disabled={option.disabled}
                    aria-selected={isSelected}
                    aria-disabled={option.disabled || undefined}
                    title={option.label}
                    className={optionClasses}
                    onMouseDown={(event) => {
                      event.preventDefault();
                    }}
                    onMouseEnter={() => {
                      if (!option.disabled) {
                        setHighlightedIndex(index);
                      }
                    }}
                    onClick={() => selectOption(option)}
                  >
                    <span className="md3-select-option-text">
                      <span className="md3-select-option-label">{option.label}</span>
                      {option.description ? (
                        <span className="md3-select-option-desc">
                          {option.description}
                        </span>
                      ) : null}
                    </span>
                    {isSelected ? (
                      <span className="md3-select-option-check" aria-hidden="true">
                        <CheckIcon size={14} />
                      </span>
                    ) : null}
                  </button>
                );
              })
            )}
          </div>,
          document.body
        )
      : null;

  return (
    <div ref={rootRef} className={rootClasses}>
      <button
        ref={triggerRef}
        id={id}
        type="button"
        className={`md3-select-trigger md3-select-trigger--${variant}`}
        disabled={disabled}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        aria-activedescendant={activeDescendantId}
        onClick={() => {
          if (open) {
            closeMenu(false);
          } else {
            openMenu("first");
          }
        }}
        onKeyDown={handleTriggerKeyDown}
      >
        <span className="md3-select-trigger-main">
          {leadingIcon ? (
            <span className="md3-select-leading-icon" aria-hidden="true">
              {leadingIcon}
            </span>
          ) : null}
          <span
            className={
              selectedOption
                ? "md3-select-value"
                : "md3-select-value md3-select-value--placeholder"
            }
            title={triggerValueText || undefined}
          >
            {triggerValueText}
          </span>
        </span>
        <span className="md3-select-chevron" aria-hidden="true">
          <ChevronDownIcon size={14} />
        </span>
      </button>

      {menuPortal}
    </div>
  );
}
