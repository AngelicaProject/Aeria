import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Tooltip } from "radix-ui";
import { UiIcon, type UiIconName, type UiIconSize } from "./UiIcon";

type IconButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> & {
  icon: UiIconName;
  label: string;
  shortcut?: string | undefined;
  size?: UiIconSize;
  tone?: "ghost" | "subtle";
  pressed?: boolean | undefined;
  children?: ReactNode;
};

/** Icon-only button with an accessible name and a themed tooltip. */
export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { icon, label, shortcut, size = "sm", tone = "ghost", pressed, className, children, ...rest },
  ref,
) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>
        <button
          ref={ref}
          type="button"
          aria-label={label}
          aria-pressed={pressed}
          className={`icon-button icon-button-${tone}${className ? ` ${className}` : ""}`}
          {...rest}
        >
          <UiIcon icon={icon} size={size} />
          {children}
        </button>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content className="tooltip" sideOffset={6} collisionPadding={8}>
          {label}
          {shortcut ? <kbd>{shortcut}</kbd> : null}
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
});
