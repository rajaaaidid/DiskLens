export type Category =
  | "folder"
  | "video"
  | "image"
  | "audio"
  | "archive"
  | "executable"
  | "document"
  | "code"
  | "system"
  | "other";

/** In-memory tree node used by the UI. The real app gets these from the Rust arena. */
export interface TreeNode {
  id: number;
  name: string;
  /** Logical size in bytes (sum of children for folders). */
  size: number;
  /** Allocated size on disk in bytes. */
  onDisk: number;
  /** Number of files contained (1 for a file). */
  files: number;
  isDir: boolean;
  category: Category;
  /** For folders: the category holding most of the bytes inside (used when drawn as a leaf). */
  dominant: Category;
  /** Unix ms of last modification. */
  mtime: number;
  parent: TreeNode | null;
  children: TreeNode[];
}

export interface Drive {
  letter: string;
  label: string;
  fs: string;
  total: number;
  free: number;
  /** fixed | removable | network | ram | other */
  kind?: string;
}
