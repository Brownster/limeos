/**
 * What the Storage page shows, from the shared overview and (for
 * administrators) the on-demand storage inventory. Pure and dependency-free
 * so node tests import it directly. Rules:
 * - capacity is summed from `size_bytes` only where it is known, and says
 *   how many sizes are missing; nothing is estimated;
 * - usage exists only for the root filesystem (the host's disk percentage);
 *   every other filesystem shows no bar rather than an invented one;
 * - a whole disk is "In use" when anything on it is mounted, which the
 *   observer's own "unmounted" for a partitioned disk does not say.
 */
import type { HostMetrics, Resource, StorageInventory } from "../../../contracts/generated/types";

export type Tone = "neutral" | "accent" | "success" | "warning" | "danger" | "info";
export interface Status { label: string; tone: Tone }

export interface FilesystemNode {
  resource: Resource;
  /** 0 for a partition on the disk, 1 for something inside it (LVM, crypt…). */
  depth: number;
  /** Mounted at "/": the host's disk percentage is its usage. */
  root: boolean;
}
export interface DiskNode {
  disk: Resource;
  /** The disk itself when it carries a filesystem, then its partitions. */
  filesystems: FilesystemNode[];
  mounted: number;
  status: Status;
}
export interface StorageTree {
  disks: DiskNode[];
  /** Filesystems whose parent disk isn't reported. */
  orphans: FilesystemNode[];
  pools: Resource[];
}

const hasFilesystem = (r: Resource) => r.filesystem !== null || r.mountpoint !== null;

export function filesystemStatus(r: Resource): Status {
  switch (r.status) {
    case "mounted": return { label: "Mounted", tone: "success" };
    case "unmounted": return { label: "Not mounted", tone: "neutral" };
    case "swap": return { label: "Swap", tone: "info" };
    case "ambiguous": return { label: "Can't identify", tone: "warning" };
    case "missing": return { label: "Missing", tone: "danger" };
    default: return { label: r.status.replaceAll("_", " "), tone: "neutral" };
  }
}

function diskStatus(disk: Resource, filesystems: FilesystemNode[]): Status {
  if (disk.status === "missing") return { label: "Missing", tone: "danger" };
  if (disk.status === "ambiguous" || filesystems.some((f) => f.resource.status === "ambiguous")) return { label: "Can't identify", tone: "warning" };
  if (filesystems.some((f) => f.resource.status === "missing")) return { label: "Partition missing", tone: "danger" };
  if (filesystems.some((f) => f.resource.status === "mounted")) return { label: "In use", tone: "success" };
  if (filesystems.some((f) => f.resource.status === "swap")) return { label: "Swap", tone: "info" };
  return { label: filesystems.length ? "Not mounted" : "No filesystem", tone: "neutral" };
}

const byName = (a: Resource, b: Resource) => a.name.localeCompare(b.name, undefined, { numeric: true });

/** Disks with their partitions nested by `parent`, orphans and pools. */
export function storageTree(resources: readonly Resource[] | null | undefined): StorageTree {
  const all = resources ?? [];
  const disks = all.filter((r) => r.kind === "disk").sort(byName);
  const partitions = all.filter((r) => r.kind === "partition");
  const children = new Map<string, Resource[]>();
  for (const p of partitions) if (p.parent) children.set(p.parent, [...(children.get(p.parent) ?? []), p]);
  const placed = new Set<string>();
  const walk = (id: string, depth: number, out: FilesystemNode[]) => {
    for (const child of (children.get(id) ?? []).sort(byName)) {
      if (placed.has(child.id)) continue;
      placed.add(child.id);
      out.push({ resource: child, depth: Math.min(depth, 3), root: child.mountpoint === "/" });
      walk(child.id, depth + 1, out);
    }
  };
  const nodes = disks.map((disk) => {
    const filesystems: FilesystemNode[] = hasFilesystem(disk) ? [{ resource: disk, depth: 0, root: disk.mountpoint === "/" }] : [];
    walk(disk.id, 0, filesystems);
    return {
      disk,
      filesystems,
      mounted: filesystems.filter((f) => f.resource.status === "mounted").length,
      status: diskStatus(disk, filesystems),
    };
  });
  const orphans: FilesystemNode[] = [];
  for (const p of partitions.sort(byName)) {
    if (placed.has(p.id)) continue;
    placed.add(p.id);
    orphans.push({ resource: p, depth: 0, root: p.mountpoint === "/" });
    walk(p.id, 1, orphans);
  }
  return { disks: nodes, orphans, pools: all.filter((r) => r.kind === "pool").sort(byName) };
}

export interface StorageSummary {
  disks: number;
  /** Filesystems: partitions plus disks that carry one directly. */
  filesystems: number;
  mounted: number;
  notMounted: number;
  pools: number;
  poolsMounted: number;
  /** Sum of the disks' known sizes in bytes, or null when none is known. */
  capacity: number | null;
  /** Disks whose size isn't reported. */
  sizeUnknown: number;
  /** The host's root filesystem use (0–100), or null. */
  rootPercent: number | null;
}

export function storageSummary(tree: StorageTree, host: HostMetrics | null | undefined): StorageSummary {
  const filesystems = [...tree.disks.flatMap((d) => d.filesystems), ...tree.orphans];
  const sizes = tree.disks.map((d) => d.disk.size_bytes);
  const known = sizes.filter((s): s is number => typeof s === "number" && Number.isFinite(s) && s >= 0);
  const root = host?.disk_percent;
  return {
    disks: tree.disks.length,
    filesystems: filesystems.length,
    mounted: filesystems.filter((f) => f.resource.status === "mounted").length,
    notMounted: filesystems.filter((f) => f.resource.status === "unmounted").length,
    pools: tree.pools.length,
    poolsMounted: tree.pools.filter((p) => p.status === "mounted").length,
    capacity: known.length ? known.reduce((a, b) => a + b, 0) : null,
    sizeUnknown: sizes.length - known.length,
    rootPercent: typeof root === "number" && Number.isFinite(root) ? root : null,
  };
}

/** "serial:WD-123" → "WD-123"; null when the disk is identified otherwise. */
export function serialOf(r: Resource): string | null {
  const id = r.identity ?? r.id;
  return id.startsWith("serial:") ? id.slice(7) : null;
}
/** "uuid:abcd" → "abcd". */
export function uuidOf(r: Resource): string | null {
  const id = r.identity ?? r.id;
  return id.startsWith("uuid:") ? id.slice(5) : null;
}

// ---- The storage inventory (administrators) -------------------------------

export interface FstabFact { mountpoint: string; managed: boolean }
export interface FilesystemFacts {
  uuid: string;
  filesystem: string;
  serial: string | null;
  /** Backs the running system (root or boot). */
  boot: boolean;
  swap: boolean;
  mounts: { mountpoint: string; writable: boolean; filesystem: string; root: string }[];
  fstab: FstabFact[];
}

/** Facts per filesystem UUID, with the fstab entries that name it (by UUID or device number). */
export function inventoryFacts(inventory: StorageInventory): Map<string, FilesystemFacts> {
  const facts = new Map<string, FilesystemFacts>();
  for (const d of inventory.devices) {
    const fstab = inventory.fstab_entries
      .filter((e) => e.filesystem_uuid === d.filesystem_uuid || (e.device !== null && e.device.major === d.device.major && e.device.minor === d.device.minor))
      .map((e) => ({ mountpoint: e.mountpoint, managed: e.managed }));
    facts.set(d.filesystem_uuid, {
      uuid: d.filesystem_uuid,
      filesystem: d.filesystem,
      serial: d.serial,
      boot: d.boot_backing,
      swap: d.in_use_as_swap === true,
      mounts: d.mounts.map((m) => ({ mountpoint: m.mountpoint, writable: m.writable, filesystem: m.filesystem, root: m.filesystem_root })),
      fstab,
    });
  }
  return facts;
}

/** fstab entries no observed filesystem answers to: often a disk that is missing. */
export function unmatchedFstab(inventory: StorageInventory): { mountpoint: string; uuid: string | null; managed: boolean }[] {
  return inventory.fstab_entries
    .filter((e) => !inventory.devices.some((d) => e.filesystem_uuid === d.filesystem_uuid || (e.device !== null && e.device.major === d.device.major && e.device.minor === d.device.minor)))
    .map((e) => ({ mountpoint: e.mountpoint, uuid: e.filesystem_uuid, managed: e.managed }));
}

/** How a filesystem comes to be mounted, in plain words. */
export function mountPersistence(mounted: boolean, facts: FilesystemFacts | undefined): string | null {
  if (!facts) return null;
  const managed = facts.fstab.find((f) => f.managed);
  if (managed) return `Mounted at boot by LimeOS's managed fstab entry (${managed.mountpoint})`;
  if (facts.fstab.length) return `Mounted at boot by /etc/fstab (${facts.fstab.map((f) => f.mountpoint).join(", ")})`;
  return mounted ? "Not in /etc/fstab: mounted until the next restart" : "Not in /etc/fstab";
}

/** A disk's serial from the observer's id, else from the inventory facts of its filesystems. */
export function diskSerial(node: DiskNode, facts: Map<string, FilesystemFacts> | null): string | null {
  const own = serialOf(node.disk);
  if (own) return own;
  if (!facts) return null;
  const found = new Set(node.filesystems.map((f) => uuidOf(f.resource)).map((u) => (u ? facts.get(u)?.serial : null)).filter((s): s is string => !!s));
  return found.size === 1 ? [...found][0] : null;
}
