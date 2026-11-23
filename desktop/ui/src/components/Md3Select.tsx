import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
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

  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const listboxId = useId();

  const selectedIndex = useMemo(
    () => options.findIndex((opt) => opt.value === value),
    [options, value]
  );
  const selectedOption = selectedIndex >= 0 ? options[selectedIndex] : undefined;

  const closeMenu = useCallback((restoreFocus: boolean) => {
    setOpen(false);
    setHighlightedIndex(-1);
    if (restoreFocus) {
      triggerRef.current?.focus();
    }
  }, []);

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

  useEffect(() => {
    if (!open) {
      return;
    }
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (
        rootRef.current &&
        target instanceof Node &&
        !rootRef.current.contains(target)
      ) {
        closeMenu(false);
      }
    };
    const handleDocumentKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeMenu(true);
      }
    };

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleDocumentKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleDocumentKeyDown);
    };
  }, [closeMenu, open]);

  useEffect(() => {
    if (!open || highlightedIndex < 0) {
      return;
    }
    const activeEl = optionRefs.current[highlightedIndex];
    activeEl?.scrollIntoView({ block: "nearest" });
  }, [highlightedIndex, open]);

  useEffect(() => {
    if (disabled && open) {
      setOpen(false);
      setHighlightedIndex(-1);
    }
  }, [disabled, open]);

  useEffect(() => {
    optionRefs.current.length = options.length;
    if (!open) {
      return;
    }
    if (highlightedIndex >= options.length || (highlightedIndex >= 0 && options[highlightedIndex]?.disabled)) {
      const nextIndex =
        selectedIndex >= 0 && !options[selectedIndex]?.disabled
          ? selectedIndex
          : findEdgeEnabledIndex(options, "first");
      setHighlightedIndex(nextIndex);
    }
  }, [highlightedIndex, open, options, selectedIndex]);

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
      default:
        break;
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

  const menuClasses = [
    "md3-select-menu",
    `md3-select-menu--${placement}`,
    `md3-select-menu--align-${align}`,
  ].join(" ");

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
          >
            {selectedOption ? selectedOption.label : placeholder ?? ""}
          </span>
        </span>
        <span className="md3-select-chevron" aria-hidden="true">
          <ChevronDownIcon size={14} />
        </span>
      </button>

      {open ? (
        <div
          id={listboxId}
          role="listbox"
          aria-label={ariaLabel}
          className={menuClasses}
        >
          {options.length === 0 ? (
            <div className="md3-select-option is-disabled" role="option" aria-disabled="true">
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
        </div>
      ) : null}
    </div>
  );
}
