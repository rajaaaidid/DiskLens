import { GlassRenderer, type Viewport } from "./glRenderer";

type FrameHook = (now: number) => void;

/**
 * Owns the full-window background canvas and the single render loop.
 * The aurora drifts continuously; the treemap registers a hook that updates instances
 * while it animates, and reports the viewport it occupies.
 */
class Stage {
  renderer: GlassRenderer | null = null;
  private hooks = new Set<FrameHook>();
  private viewport: Viewport = { x: 0, y: 0, w: 0, h: 0 };
  private last = 0;
  private raf = 0;
  reduceMotion = false;
  /** Measured frames per second (rolling). */
  fps = 0;
  private fpsAcc = 0;
  private fpsFrames = 0;

  attach(canvas: HTMLCanvasElement): void {
    this.renderer = new GlassRenderer(canvas);
    this.renderer.resize();
    const loop = (now: number) => {
      this.raf = requestAnimationFrame(loop);
      const dt = this.last ? Math.min(0.1, (now - this.last) / 1000) : 0;
      this.last = now;
      this.fpsAcc += dt;
      this.fpsFrames++;
      if (this.fpsAcc >= 0.5) {
        this.fps = this.fpsFrames / this.fpsAcc;
        this.fpsAcc = 0;
        this.fpsFrames = 0;
      }
      const r = this.renderer!;
      if (!this.reduceMotion) r.time += dt;
      for (const h of this.hooks) h(now);
      r.resize();
      r.render(this.viewport);
    };
    this.raf = requestAnimationFrame(loop);
  }

  /**
   * Render `frames` frames back to back, forcing the GPU to finish, and return ms per frame.
   * Independent of requestAnimationFrame throttling (hidden windows, headless).
   */
  bench(frames = 120): { msPerFrame: number; fpsEquivalent: number } {
    const r = this.renderer!;
    const gl = (r as unknown as { gl: WebGL2RenderingContext }).gl;
    const px = new Uint8Array(4);
    r.render(this.viewport);
    gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
    const t0 = performance.now();
    for (let i = 0; i < frames; i++) {
      r.time += 1 / 60;
      r.render(this.viewport);
    }
    gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
    const ms = (performance.now() - t0) / frames;
    return { msPerFrame: +ms.toFixed(3), fpsEquivalent: Math.round(1000 / ms) };
  }

  detach(): void {
    cancelAnimationFrame(this.raf);
    this.renderer = null;
  }

  setViewport(vp: Viewport): void {
    this.viewport = vp;
  }

  addHook(h: FrameHook): () => void {
    this.hooks.add(h);
    return () => this.hooks.delete(h);
  }
}

export const stage = new Stage();
(window as unknown as { __stage: Stage }).__stage = stage;
