import type { Category } from "./types";

/** Tint per category as linear-ish sRGB 0..1, shared by the WebGL renderer and CSS swatches. */
export const CATEGORY_COLORS: Record<Category, [number, number, number]> = {
  folder: [0.74, 0.79, 0.92],
  video: [0.55, 0.36, 0.96], // violet
  image: [0.13, 0.83, 0.93], // cyan
  audio: [0.52, 0.8, 0.09], // lime
  archive: [0.96, 0.62, 0.04], // amber
  executable: [0.96, 0.25, 0.37], // rose
  document: [0.06, 0.73, 0.51], // emerald
  code: [0.23, 0.51, 0.96], // blue
  system: [0.39, 0.45, 0.55], // slate
  other: [0.61, 0.64, 0.69], // gray
};

export const CATEGORY_LABELS: Record<Category, string> = {
  folder: "Folders",
  video: "Video",
  image: "Images",
  audio: "Audio",
  archive: "Archives",
  executable: "Executables",
  document: "Documents",
  code: "Code",
  system: "System",
  other: "Other",
};

export const CATEGORY_ORDER: Category[] = [
  "video",
  "image",
  "audio",
  "archive",
  "executable",
  "document",
  "code",
  "system",
  "other",
];

export function cssColor(c: Category, alpha = 1): string {
  const [r, g, b] = CATEGORY_COLORS[c];
  return `rgba(${Math.round(r * 255)}, ${Math.round(g * 255)}, ${Math.round(b * 255)}, ${alpha})`;
}

const EXT: Record<string, Category> = {};
const add = (cat: Category, exts: string) => exts.split(" ").forEach((e) => (EXT[e] = cat));
add("video", "mp4 mkv mov avi webm wmv m4v");
add("image", "jpg jpeg png gif webp heic raw cr2 nef psd tif tiff bmp svg");
add("audio", "mp3 flac wav ogg m4a aac opus");
add("archive", "zip 7z rar tar gz xz iso cab pak vpk bz2 zst");
add("executable", "exe dll msi sys drv ocx msix appx");
add("document", "pdf doc docx xls xlsx ppt pptx txt md rtf odt epub");
add("code", "js ts tsx jsx rs py c cpp h cs java go json lock map pdb obj lib");
add("system", "vhdx vhd wim esd dat log etl evtx mui cat manifest");

export function categoryForName(name: string): Category {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return "other";
  return EXT[name.slice(dot + 1).toLowerCase()] ?? "other";
}
