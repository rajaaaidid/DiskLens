// GLSL ES 3.00 shaders for the aurora background and the glass treemap blocks.

/** Shared aurora field: a few large soft colour blobs drifting slowly. `blur` widens each Gaussian,
 *  which is exactly a Gaussian blur of the sharp field, so the "pre-blurred" copy is free. */
const AURORA_FN = /* glsl */ `
uniform float uTime;
uniform float uTheme;      // 0 = dark, 1 = light
uniform vec2  uResolution; // device px

vec3 aurora(vec2 fragPx, float blur) {
  vec2 uv = fragPx / uResolution;
  float aspect = uResolution.x / uResolution.y;
  vec2 p = vec2(uv.x * aspect, uv.y);

  vec3 baseDark  = vec3(0.027, 0.035, 0.075);
  vec3 baseLight = vec3(0.925, 0.937, 0.965);
  vec3 col = mix(baseDark, baseLight, uTheme);

  // centre (in aspect space), radius, colour (dark) and colour (light)
  vec2  c[5]; float r[5]; vec3 kd[5]; vec3 kl[5];
  float t = uTime * 0.035;
  c[0] = vec2(0.25 * aspect + 0.18 * sin(t * 1.1),        0.80 + 0.10 * cos(t * 0.9));
  c[1] = vec2(0.80 * aspect + 0.15 * cos(t * 0.8 + 1.0),  0.70 + 0.12 * sin(t * 1.3));
  c[2] = vec2(0.55 * aspect + 0.22 * sin(t * 0.6 + 2.0),  0.20 + 0.10 * cos(t * 1.1 + 0.5));
  c[3] = vec2(0.05 * aspect + 0.10 * cos(t * 1.4 + 3.0),  0.25 + 0.15 * sin(t * 0.7));
  c[4] = vec2(0.95 * aspect + 0.12 * sin(t * 0.9 + 4.0),  0.15 + 0.10 * cos(t * 1.2 + 1.5));
  r[0] = 0.42; r[1] = 0.38; r[2] = 0.46; r[3] = 0.30; r[4] = 0.32;
  kd[0] = vec3(0.30, 0.16, 0.62); kl[0] = vec3(0.66, 0.60, 0.98);
  kd[1] = vec3(0.05, 0.36, 0.52); kl[1] = vec3(0.55, 0.85, 0.95);
  kd[2] = vec3(0.12, 0.20, 0.55); kl[2] = vec3(0.62, 0.72, 1.00);
  kd[3] = vec3(0.45, 0.10, 0.38); kl[3] = vec3(0.98, 0.70, 0.86);
  kd[4] = vec3(0.04, 0.38, 0.34); kl[4] = vec3(0.60, 0.92, 0.82);

  for (int i = 0; i < 5; i++) {
    float rr = r[i] * r[i];
    float bb = rr + blur * blur;
    vec2 d = p - c[i];
    float g = (rr / bb) * exp(-dot(d, d) / bb);
    vec3 k = mix(kd[i], kl[i], uTheme);
    // Dark: additive glow. Light: tint the paper toward the pastel.
    col = mix(col + k * g * 0.85, mix(col, k, g * 0.55), uTheme);
  }
  return col;
}

float hash12(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}
`;

export const FULLSCREEN_VS = /* glsl */ `#version 300 es
void main() {
  // One oversized triangle covering the viewport.
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

export const AURORA_FS = /* glsl */ `#version 300 es
precision highp float;
${AURORA_FN}
uniform float uBlur;
uniform float uScale; // fragCoord -> full-res px (for the low-res blurred target)
out vec4 outColor;
void main() {
  vec2 px = gl_FragCoord.xy * uScale;
  vec3 col = aurora(px, uBlur);
  if (uBlur == 0.0) {
    // Vignette + dither to kill banding on the sharp background.
    vec2 uv = px / uResolution;
    float v = smoothstep(1.25, 0.35, length(uv - 0.5));
    col *= mix(0.80, 1.0, mix(v, 1.0, uTheme * 0.7));
    col += (hash12(px) - 0.5) / 255.0 * 2.0;
  }
  outColor = vec4(col, 1.0);
}`;

export const BLOCK_VS = /* glsl */ `#version 300 es
layout(location = 0) in vec2 aCorner;   // 0..1 unit quad
layout(location = 1) in vec4 aRect;     // x, y, w, h in CSS px relative to the treemap origin
layout(location = 2) in vec3 aColor;    // category tint
layout(location = 3) in vec4 aParams;   // depth, alpha, flags, header

uniform vec2  uOrigin;      // treemap origin in CSS px (window space)
uniform vec2  uResolution;  // device px
uniform float uDpr;
uniform float uHover;       // 0..1 hover animation for the overlay instance

out vec2  vLocal;   // position relative to rect centre, CSS px
out vec2  vHalf;    // half size, CSS px
out vec3  vColor;
out vec4  vParams;

void main() {
  float flags = aParams.z;
  bool overlay = mod(flags, 2.0) >= 1.0;
  vec4 r = aRect;
  if (overlay) {
    // Lift: grow slightly around the centre and rise a touch.
    vec2 c = r.xy + r.zw * 0.5;
    vec2 hs = r.zw * 0.5 * (1.0 + 0.018 * uHover) + vec2(1.5 * uHover);
    r = vec4(c - hs, hs * 2.0);
    r.y -= 1.5 * uHover;
  }
  float margin = overlay ? 22.0 : 1.0;
  vec2 hs = r.zw * 0.5;
  vec2 local = (aCorner * 2.0 - 1.0) * (hs + margin);
  vec2 px = uOrigin + r.xy + hs + local;          // CSS px, y down
  vec2 dev = px * uDpr;
  vec2 clip = vec2(dev.x / uResolution.x * 2.0 - 1.0, 1.0 - dev.y / uResolution.y * 2.0);
  gl_Position = vec4(clip, 0.0, 1.0);
  vLocal = local;
  vHalf = hs;
  vColor = aColor;
  vParams = aParams;
}`;

export const BLOCK_FS = /* glsl */ `#version 300 es
precision highp float;
in vec2 vLocal;
in vec2 vHalf;
in vec3 vColor;
in vec4 vParams;

uniform sampler2D uBlurTex;  // low-res pre-blurred background
uniform vec2  uResolution;
uniform float uTheme;
uniform float uDpr;
uniform float uHover;
out vec4 outColor;

float hash12(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

float sdRoundBox(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
  float depth  = vParams.x;
  float alpha  = vParams.y;
  float flags  = vParams.z;
  float header = vParams.w;
  bool overlay   = mod(flags, 2.0) >= 1.0;
  bool leaf      = mod(floor(flags / 2.0), 2.0) >= 1.0;
  bool aggregate = mod(floor(flags / 4.0), 2.0) >= 1.0;
  bool isDir     = mod(floor(flags / 8.0), 2.0) >= 1.0;

  float minSide = min(vHalf.x, vHalf.y) * 2.0;
  float radius = clamp(minSide * 0.22, 0.0, 7.0);
  float d = sdRoundBox(vLocal, vHalf, radius);
  float aa = 0.75 / uDpr;
  float inside = 1.0 - smoothstep(-aa, aa, d);

  float ty = clamp((vLocal.y + vHalf.y) / max(vHalf.y * 2.0, 1.0), 0.0, 1.0); // 0 top -> 1 bottom
  float yTop = vLocal.y + vHalf.y;                                          // px from top edge
  float tx = vLocal.x / max(vHalf.x, 1.0);                                   // -1..1

  // Refraction: near the edges the "lens" bends the background inward.
  float edge = 1.0 - smoothstep(0.0, min(10.0, minSide * 0.35), -d);
  vec2 bend = -sign(vLocal) * step(vHalf - min(10.0, minSide * 0.35), abs(vLocal)) * edge * 9.0 * uDpr;
  vec2 uv = (gl_FragCoord.xy + vec2(bend.x, -bend.y)) / uResolution;
  vec3 bg = texture(uBlurTex, uv).rgb;
  // Glass slightly concentrates colour: boost saturation of what is behind.
  float lum = dot(bg, vec3(0.299, 0.587, 0.114));
  bg = mix(vec3(lum), bg, 1.35);

  // --- glass body ---
  float tintA = isDir ? 0.06 + 0.03 * depth : 0.24 + 0.035 * depth;
  tintA = clamp(tintA, 0.06, 0.40);
  if (uTheme > 0.5) tintA *= 1.3;
  vec3 tint = vColor;
  vec3 glass = mix(bg, tint, tintA);

  // frost: lift towards white a little, more in light mode
  glass = mix(glass, vec3(1.0), uTheme > 0.5 ? 0.20 : 0.045);
  // top-to-bottom gradient
  glass *= mix(1.14, 0.86, ty);
  // soft diagonal sheen (visionOS-style), strongest on the upper-left
  float diag = clamp(1.0 - ((vLocal.x / max(vHalf.x, 1.0)) * 0.5 + 0.5 + ty) * 0.75, 0.0, 1.0);
  glass += diag * diag * (uTheme > 0.5 ? 0.05 : 0.06);
  // inner shadow along the bottom edge gives the tile thickness
  glass *=1.0 - 0.18 * (1.0 - smoothstep(0.0, 3.0, vHalf.y - vLocal.y)) * step(-d, 4.0);
  // folder header band reads as slightly denser glass
  if (isDir && header > 0.0 && yTop < header) glass = mix(glass, tint, 0.10);

  // small-files aggregate: fine diagonal hatching
  if (aggregate) {
    float s = mod((gl_FragCoord.x + gl_FragCoord.y) / uDpr, 6.0);
    glass = mix(glass, glass * (uTheme > 0.5 ? 0.92 : 1.18), step(3.0, s) * 0.6);
  }

  // rim light: 1px bright inner edge, stronger along the top
  float rim = 1.0 - smoothstep(0.0, 1.25, -d);
  float rimStrength = mix(0.34, 0.10, ty);
  if (uTheme > 0.5) rimStrength = mix(0.75, 0.40, ty);
  // specular sheen along the top edge
  float sheenH = clamp(min(vHalf.y * 2.0 * 0.22, 16.0), 1.5, 16.0);
  float sheen = exp(-yTop / sheenH) * (1.0 - pow(abs(tx), 6.0)) * (uTheme > 0.5 ? 0.10 : 0.09);

  vec3 col = glass + sheen;
  col = mix(col, vec3(1.0), rim * rimStrength);
  // light theme: thin darker contour just outside the rim so tiles separate on pale glass
  if (uTheme > 0.5) col = mix(col, tint * 0.45, (1.0 - smoothstep(0.0, 0.9, abs(d + 0.5))) * 0.22);

  // grain
  col += (hash12(gl_FragCoord.xy) - 0.5) * 0.028;

  float a = inside;

  if (overlay) {
    // Hover: shadow and glow outside, brighten + glowing rim inside.
    float glowRim = (1.0 - smoothstep(0.0, 2.0, abs(d))) * uHover;
    vec2 sp = vLocal - vec2(0.0, -4.0 * uHover);
    float ds = sdRoundBox(sp, vHalf, radius);
    float shadow = (1.0 - smoothstep(-2.0, 18.0, ds)) * (1.0 - inside) * 0.5 * uHover;
    float glow = exp(-max(d, 0.0) / 5.0) * (1.0 - inside) * 0.6 * uHover;
    vec3 glowCol = mix(vColor, vec3(1.0), 0.5);
    if (leaf) {
      col = mix(col, mix(vColor, vec3(1.0), 0.25), 0.12 * uHover);
      col = col * (1.0 + 0.22 * uHover) + 0.05 * uHover;
      col = mix(col, glowCol, glowRim * 0.9);
      vec3 outside = mix(vec3(0.0), glowCol, glow / max(shadow + glow, 1e-4));
      float outA = clamp(shadow + glow, 0.0, 1.0);
      vec3 rgb = col * inside + outside * outA * (1.0 - inside);
      outColor = vec4(rgb * alpha, (inside + outA * (1.0 - inside)) * alpha);
    } else {
      // Folder: keep children visible, only brighten and glow the rim.
      float insideA = 0.07 * uHover * inside + glowRim * 0.8;
      vec3 rgb = glowCol * (insideA + glow) ;
      float outA = clamp(shadow + glow, 0.0, 1.0) * (1.0 - inside);
      outColor = vec4(rgb * alpha, clamp(insideA + outA, 0.0, 1.0) * alpha);
    }
    return;
  }

  a *= alpha;
  outColor = vec4(col * a, a);
}`;
