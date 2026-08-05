import { useId } from "react";

/** Echo, the listener — inline for empty states. Decorative only. */
export function EchoMark({ size = 96, dim = false }: { size?: number; dim?: boolean }) {
  const gid = useId();
  return (
    <svg width={size} height={size} viewBox="0 0 120 120" aria-hidden="true"
         style={dim ? { opacity: 0.45 } : undefined}>
      <defs>
        <linearGradient id={`echo-bg-${gid}`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="var(--c-accent)" />
          <stop offset="1" stopColor="var(--c-accent-2)" />
        </linearGradient>
      </defs>
      <path d="M34 78 V54 a26 26 0 0 1 52 0 V78 a6 6 0 0 1 -9 5 l-4 -2.5 a6 6 0 0 0 -6.5 0 l-3 2 a6 6 0 0 1 -7 0 l-3 -2 a6 6 0 0 0 -6.5 0 l-4 2.5 a6 6 0 0 1 -9 -5 Z" fill={`url(#echo-bg-${gid})`}/>
      <ellipse cx="51" cy="50" rx="3.2" ry="4.8" fill="var(--c-app)"/>
      <ellipse cx="69" cy="50" rx="3.2" ry="4.8" fill="var(--c-app)"/>
      <g stroke="var(--c-app)" strokeWidth="3.6" strokeLinecap="round">
        <line x1="51" y1="66" x2="51" y2="70"/><line x1="58" y1="63" x2="58" y2="73"/><line x1="65" y1="65" x2="65" y2="71"/>
      </g>
    </svg>
  );
}
