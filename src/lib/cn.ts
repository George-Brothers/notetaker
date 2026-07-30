import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Joins class names, with later Tailwind utilities beating earlier ones.
 *
 * `clsx` alone would emit `px-2 px-4` and leave the winner to CSS source order,
 * which is not what a caller passing `px-4` as an override means. `twMerge`
 * resolves that to `px-4`.
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
