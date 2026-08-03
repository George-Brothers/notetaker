/**
 * The shared primitives, in one file.
 *
 * shadcn/ui's convention is one file per component, which pays off in a large
 * design system. This app uses eight primitives, and splitting them across
 * eight files would mean eight imports to read one button's hover state. They
 * live together until there are enough to justify the folder.
 *
 * Behaviour that is genuinely hard — focus trapping, roving tab index, escape
 * handling, portalling above everything — comes from Radix. Anything that is
 * just a styled element stays a styled element.
 */

import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import * as TabsPrimitive from "@radix-ui/react-tabs";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import * as SwitchPrimitive from "@radix-ui/react-switch";
import * as PopoverPrimitive from "@radix-ui/react-popover";
import { cva, type VariantProps } from "class-variance-authority";
import { X } from "lucide-react";
import { cn } from "../lib/cn";

// --- Button ---------------------------------------------------------------

const button = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium " +
    "transition-colors disabled:pointer-events-none disabled:opacity-45 " +
    "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
  {
    variants: {
      variant: {
        primary: "bg-accent text-accent-fg hover:bg-accent-hover",
        secondary: "bg-raised text-fg border border-border hover:bg-hover",
        ghost: "text-fg-muted hover:bg-hover hover:text-fg",
        danger: "bg-recording text-white hover:brightness-110",
        link: "text-accent underline-offset-4 hover:underline",
      },
      size: {
        sm: "h-7 rounded-[var(--radius-control)] px-2.5 text-[13px]",
        md: "h-9 rounded-[var(--radius-control)] px-3.5 text-sm",
        lg: "h-11 rounded-[var(--radius-control)] px-5 text-[15px]",
        icon: "h-8 w-8 rounded-[var(--radius-control)]",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof button> {}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, type = "button", ...props }, ref) => (
    <button ref={ref} type={type} className={cn(button({ variant, size }), className)} {...props} />
  ),
);
Button.displayName = "Button";

// --- Tooltip --------------------------------------------------------------

/** Wrap the app once; every `Tip` needs an ancestor provider. */
export const TooltipProvider = TooltipPrimitive.Provider;

/**
 * A hint on hover or keyboard focus.
 *
 * `label` is a hint, never the only name for the control — a tooltip is
 * invisible to touch and unreliable for screen readers, so every icon button
 * also carries its own `aria-label`.
 */
export function Tip({
  label,
  children,
  side = "bottom",
}: {
  label: React.ReactNode;
  children: React.ReactNode;
  side?: "top" | "bottom" | "left" | "right";
}) {
  return (
    <TooltipPrimitive.Root delayDuration={400}>
      <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Content
          side={side}
          sideOffset={6}
          className="z-50 max-w-64 rounded-[var(--radius-control)] border border-border bg-raised px-2.5 py-1.5 text-[13px] text-fg shadow-[var(--shadow-pop)]"
        >
          {label}
        </TooltipPrimitive.Content>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  );
}

/** An icon-only button. The label is required — see `Tip`. */
export const IconButton = React.forwardRef<
  HTMLButtonElement,
  ButtonProps & { label: string; tip?: boolean }
>(({ label, tip = true, ...props }, ref) => {
  const btn = <Button ref={ref} size="icon" variant="ghost" aria-label={label} {...props} />;
  return tip ? <Tip label={label}>{btn}</Tip> : btn;
});
IconButton.displayName = "IconButton";

// --- Dialog ---------------------------------------------------------------

export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  wide,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children: React.ReactNode;
  wide?: boolean;
}) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-40 bg-black/35 backdrop-blur-[2px]" />
        <DialogPrimitive.Content
          className={cn(
            "fixed left-1/2 top-1/2 z-50 flex max-h-[85vh] w-[calc(100vw-2rem)] -translate-x-1/2 -translate-y-1/2 flex-col",
            "rounded-[var(--radius-card)] border border-border bg-raised shadow-[var(--shadow-pop)]",
            wide ? "max-w-3xl" : "max-w-lg",
          )}
        >
          <div className="flex items-start justify-between gap-4 border-b border-border px-5 py-4">
            <div>
              <DialogPrimitive.Title className="text-[15px] font-semibold text-fg">
                {title}
              </DialogPrimitive.Title>
              {description && (
                <DialogPrimitive.Description className="mt-1 text-[13px] text-fg-muted">
                  {description}
                </DialogPrimitive.Description>
              )}
            </div>
            <DialogPrimitive.Close asChild>
              <Button size="icon" variant="ghost" aria-label="Close">
                <X size={16} />
              </Button>
            </DialogPrimitive.Close>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

// --- Tabs -----------------------------------------------------------------

export const Tabs = TabsPrimitive.Root;

export function TabList({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <TabsPrimitive.List
      className={cn("inline-flex items-center gap-1 rounded-[var(--radius-control)] bg-sunken p-0.5", className)}
    >
      {children}
    </TabsPrimitive.List>
  );
}

export function Tab({ value, children }: { value: string; children: React.ReactNode }) {
  return (
    <TabsPrimitive.Trigger
      value={value}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-[calc(var(--radius-control)-2px)] px-2.5 py-1 text-[13px] font-medium",
        "text-fg-muted transition-colors hover:text-fg",
        "data-[state=active]:bg-raised data-[state=active]:text-fg data-[state=active]:shadow-[var(--shadow-card)]",
      )}
    >
      {children}
    </TabsPrimitive.Trigger>
  );
}

export const TabPanel = TabsPrimitive.Content;

// --- Switch ---------------------------------------------------------------

export function Switch({
  checked,
  onCheckedChange,
  label,
}: {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  label: string;
}) {
  return (
    <SwitchPrimitive.Root
      checked={checked}
      onCheckedChange={onCheckedChange}
      aria-label={label}
      className={cn(
        "relative h-5 w-9 shrink-0 rounded-full border border-border transition-colors",
        "data-[state=checked]:border-accent data-[state=checked]:bg-accent data-[state=unchecked]:bg-sunken",
      )}
    >
      <SwitchPrimitive.Thumb
        className={cn(
          "block h-3.5 w-3.5 translate-x-0.5 rounded-full bg-raised shadow-sm transition-transform",
          "data-[state=checked]:translate-x-[18px]",
        )}
      />
    </SwitchPrimitive.Root>
  );
}

// --- Popover --------------------------------------------------------------

export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;

export function PopoverContent({
  children,
  align = "start",
  className,
}: {
  children: React.ReactNode;
  align?: "start" | "center" | "end";
  className?: string;
}) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        align={align}
        sideOffset={6}
        className={cn(
          "z-50 rounded-[var(--radius-card)] border border-border bg-raised p-1 shadow-[var(--shadow-pop)]",
          className,
        )}
      >
        {children}
      </PopoverPrimitive.Content>
    </PopoverPrimitive.Portal>
  );
}

// --- Small display pieces -------------------------------------------------

/** A quiet label: a task name, a mode, a speaker. */
export function Chip({
  children,
  color,
  className,
}: {
  children: React.ReactNode;
  /** A CSS colour for the dot. Omit for no dot. */
  color?: string;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border border-border bg-raised px-2 py-0.5 text-[12px] text-fg-muted",
        className,
      )}
    >
      {color && (
        <span
          aria-hidden
          className="h-1.5 w-1.5 shrink-0 rounded-full"
          style={{ backgroundColor: color }}
        />
      )}
      {children}
    </span>
  );
}

/**
 * The empty state for a pane.
 *
 * A component rather than an inline paragraph because there are six of them and
 * they are the screens a new user sees most — an app with nothing in it yet is
 * the first impression, and "no data" is not an acceptable one.
 */
export function Empty({
  icon,
  title,
  children,
}: {
  icon?: React.ReactNode;
  title: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 px-8 py-16 text-center">
      {icon && <div className="mb-1 text-fg-faint">{icon}</div>}
      <p className="text-sm font-medium text-fg">{title}</p>
      {children && <p className="max-w-sm text-[13px] leading-relaxed text-fg-muted">{children}</p>}
    </div>
  );
}

/** An inline problem the user can act on. Never a stack trace. */
export function Notice({
  tone = "error",
  children,
  className,
  role,
}: {
  tone?: "error" | "warn" | "ok";
  children: React.ReactNode;
  className?: string;
  role?: "alert" | "status";
}) {
  const tones = {
    error: "bg-error-soft text-error",
    warn: "bg-warn-soft text-warn",
    ok: "bg-ok-soft text-ok",
  } as const;
  return (
    <p
      role={role ?? (tone === "error" ? "alert" : undefined)}
      className={cn(
        "rounded-[var(--radius-control)] px-3 py-2 text-[13px] leading-snug",
        tones[tone],
        className,
      )}
    >
      {children}
    </p>
  );
}

/** A keyboard shortcut, rendered as keycaps. */
export function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="rounded border border-border bg-sunken px-1.5 py-0.5 font-sans text-[11px] font-medium text-fg-muted">
      {children}
    </kbd>
  );
}

/**
 * The modifier key label for this machine: ⌘ on a Mac, Ctrl everywhere else.
 *
 * A function rather than a constant so it is evaluated in the browser rather
 * than baked in at build time — the same bundle is served to a Mac and a PC.
 */
export function modKey(): string {
  if (typeof navigator === "undefined") return "Ctrl";
  return /mac|iphone|ipad/i.test(navigator.platform || navigator.userAgent) ? "⌘" : "Ctrl";
}
