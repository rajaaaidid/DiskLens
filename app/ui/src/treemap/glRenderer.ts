import { AURORA_FS, BLOCK_FS, BLOCK_VS, FULLSCREEN_VS } from "./shaders";

/** Floats per instance: rect(4) + color(3) + params(4). */
export const INSTANCE_FLOATS = 11;

export const FLAG_OVERLAY = 1;
export const FLAG_LEAF = 2;
export const FLAG_AGGREGATE = 4;
export const FLAG_DIR = 8;
/** Folder still being scanned: drawn with a moving shimmer. */
export const FLAG_PENDING = 16;
/** Selection ring (drawn as an extra overlay instance). */
export const FLAG_SELECTED = 32;

export interface Viewport {
  /** Treemap origin + size in CSS px, window space. */
  x: number;
  y: number;
  w: number;
  h: number;
}

function compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader {
  const s = gl.createShader(type)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    throw new Error(`shader compile failed: ${gl.getShaderInfoLog(s)}`);
  }
  return s;
}

function program(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const p = gl.createProgram()!;
  gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, vs));
  gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    throw new Error(`program link failed: ${gl.getProgramInfoLog(p)}`);
  }
  return p;
}

/**
 * Full-window WebGL2 renderer: draws the drifting aurora, keeps a quarter-res blurred copy of it,
 * and draws all treemap blocks as instanced glass quads sampling that copy.
 */
export class GlassRenderer {
  private gl: WebGL2RenderingContext;
  private auroraProg: WebGLProgram;
  private blockProg: WebGLProgram;
  private emptyVao: WebGLVertexArrayObject;
  private blockVao: WebGLVertexArrayObject;
  private instanceBuf: WebGLBuffer;
  private instanceCap = 0;
  private instanceCount = 0;
  private blurFbo: WebGLFramebuffer;
  private blurTex: WebGLTexture;
  private blurW = 0;
  private blurH = 0;
  private frame = 0;
  private uA: Record<string, WebGLUniformLocation | null> = {};
  private uB: Record<string, WebGLUniformLocation | null> = {};

  theme = 0;
  hover = 0;
  /** Tile tint strength: 1 for file types, more for age colours. */
  tintBoost = 1;
  /** Seconds of aurora animation; frozen when reduced motion is on. */
  time = 0;

  /** Background opacity (< 1 lets a native Mica/Acrylic window backdrop show through). */
  opacity = 1;

  constructor(private canvas: HTMLCanvasElement, translucent = false) {
    const gl = canvas.getContext("webgl2", { antialias: false, alpha: translucent, premultipliedAlpha: true, powerPreference: "high-performance" });
    if (!gl) throw new Error("WebGL2 not available");
    this.gl = gl;
    this.auroraProg = program(gl, FULLSCREEN_VS, AURORA_FS);
    this.blockProg = program(gl, BLOCK_VS, BLOCK_FS);
    for (const n of ["uTime", "uTheme", "uResolution", "uBlur", "uScale", "uOpacity"]) this.uA[n] = gl.getUniformLocation(this.auroraProg, n);
    for (const n of ["uOrigin", "uResolution", "uDpr", "uHover", "uBlurTex", "uTheme", "uTime", "uTintBoost"]) this.uB[n] = gl.getUniformLocation(this.blockProg, n);

    this.emptyVao = gl.createVertexArray()!;

    // Block VAO: static unit quad + per-instance attributes.
    this.blockVao = gl.createVertexArray()!;
    gl.bindVertexArray(this.blockVao);
    const quad = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    this.instanceBuf = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.instanceBuf);
    const stride = INSTANCE_FLOATS * 4;
    const attr = (loc: number, size: number, offsetFloats: number) => {
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, size, gl.FLOAT, false, stride, offsetFloats * 4);
      gl.vertexAttribDivisor(loc, 1);
    };
    attr(1, 4, 0);
    attr(2, 3, 4);
    attr(3, 4, 7);
    gl.bindVertexArray(null);

    this.blurTex = gl.createTexture()!;
    this.blurFbo = gl.createFramebuffer()!;
  }

  /** Match the drawing buffer to the canvas' CSS size × DPR. */
  resize(): void {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(this.canvas.clientWidth * dpr));
    const h = Math.max(1, Math.round(this.canvas.clientHeight * dpr));
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
    }
    const bw = Math.max(1, Math.round(w / 4));
    const bh = Math.max(1, Math.round(h / 4));
    if (bw !== this.blurW || bh !== this.blurH) {
      const gl = this.gl;
      this.blurW = bw;
      this.blurH = bh;
      gl.bindTexture(gl.TEXTURE_2D, this.blurTex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, bw, bh, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.blurFbo);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.blurTex, 0);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      this.frame = 0; // force a blur refresh
    }
  }

  /** Upload instance data (length = count * INSTANCE_FLOATS). */
  setInstances(data: Float32Array, count: number): void {
    const gl = this.gl;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.instanceBuf);
    if (data.byteLength > this.instanceCap) {
      this.instanceCap = Math.max(data.byteLength, this.instanceCap * 2);
      gl.bufferData(gl.ARRAY_BUFFER, this.instanceCap, gl.DYNAMIC_DRAW);
    }
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, data, 0, count * INSTANCE_FLOATS);
    this.instanceCount = count;
  }

  render(vp: Viewport): void {
    const gl = this.gl;
    const dpr = window.devicePixelRatio || 1;
    const W = this.canvas.width;
    const H = this.canvas.height;

    gl.useProgram(this.auroraProg);
    gl.bindVertexArray(this.emptyVao);
    gl.uniform1f(this.uA.uTime, this.time);
    gl.uniform1f(this.uA.uTheme, this.theme);
    gl.uniform2f(this.uA.uResolution, W, H);
    gl.disable(gl.BLEND);

    // The aurora drifts slowly, so the blurred copy only needs refreshing every few frames.
    if (this.frame % 3 === 0) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.blurFbo);
      gl.viewport(0, 0, this.blurW, this.blurH);
      gl.uniform1f(this.uA.uBlur, 0.09);
      gl.uniform1f(this.uA.uScale, W / this.blurW);
      gl.uniform1f(this.uA.uOpacity, 1);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    }
    this.frame++;

    gl.viewport(0, 0, W, H);
    gl.uniform1f(this.uA.uBlur, 0);
    gl.uniform1f(this.uA.uScale, 1);
    gl.uniform1f(this.uA.uOpacity, this.opacity);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    if (this.instanceCount === 0) return;
    gl.useProgram(this.blockProg);
    gl.bindVertexArray(this.blockVao);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.blurTex);
    gl.uniform1i(this.uB.uBlurTex, 0);
    gl.uniform2f(this.uB.uOrigin, vp.x, vp.y);
    gl.uniform2f(this.uB.uResolution, W, H);
    gl.uniform1f(this.uB.uDpr, dpr);
    gl.uniform1f(this.uB.uHover, this.hover);
    gl.uniform1f(this.uB.uTheme, this.theme);
    gl.uniform1f(this.uB.uTime, this.time);
    gl.uniform1f(this.uB.uTintBoost, this.tintBoost);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    // Clip to the treemap area (with room for the hover shadow to spill a little).
    gl.enable(gl.SCISSOR_TEST);
    const pad = 4;
    gl.scissor(
      Math.floor((vp.x - pad) * dpr),
      Math.floor(H - (vp.y + vp.h + pad) * dpr),
      Math.ceil((vp.w + pad * 2) * dpr),
      Math.ceil((vp.h + pad * 2) * dpr),
    );
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, this.instanceCount);
    gl.disable(gl.SCISSOR_TEST);
    gl.bindVertexArray(null);
  }
}
