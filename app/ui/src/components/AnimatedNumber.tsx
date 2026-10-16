import { useEffect, useRef, useState } from "react";

/** Number that eases toward its target (instant when reduced motion is on). */
export function AnimatedNumber({ value, format, reduceMotion }: { value: number; format: (n: number) => string; reduceMotion: boolean }) {
  const [shown, setShown] = useState(value);
  const from = useRef(value);
  const raf = useRef(0);
  useEffect(() => {
    cancelAnimationFrame(raf.current);
    if (reduceMotion) {
      setShown(value);
      from.current = value;
      return;
    }
    let start = -1;
    const a = from.current;
    const step = (now: number) => {
      if (start < 0) start = now;
      const t = Math.min(1, Math.max(0, (now - start) / 350));
      const e = 1 - Math.pow(1 - t, 3);
      const v = a + (value - a) * e;
      from.current = v;
      setShown(v);
      if (t < 1) raf.current = requestAnimationFrame(step);
    };
    raf.current = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf.current);
  }, [value, reduceMotion]);
  return <>{format(Math.round(shown))}</>;
}
