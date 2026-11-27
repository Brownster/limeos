import test from "node:test";
import assert from "node:assert/strict";
import {
  storageTree, storageSummary, filesystemStatus, inventoryFacts, unmatchedFstab, mountPersistence, diskSerial, serialOf, uuidOf,
} from "../src/storage/storage-model.ts";
import { decodeStorageInventory, readStorageInventory, StorageInventoryError } from "../src/storage/inventory-client.ts";

const resource = (patch) => ({
  id: patch.id ?? `x:${patch.name}`, kind: "container", name: "x", status: "running", source: "docker", parent: null, identity: null,
  filesystem: null, mountpoint: null, size_bytes: null, image: null, health: null, cpu_percent: null, memory_percent: null, ...patch,
});

// ---- Storage tree -----------------------------------------------------------------

const disk = (name, patch = {}) => resource({ id: `serial:${name.toUpperCase()}`, kind: "disk", name, status: "unmounted", source: "storage", ...patch });
const part = (name, parent, patch = {}) => resource({ id: `uuid:${name}`, kind: "partition", name, parent, status: "mounted", source: "storage", filesystem: "ext4", mountpoint: `/mnt/${name}`, ...patch });

test("storage tree nests partitions (and what is inside them) under their disk", () => {
  const tree = storageTree([
    part("sdb1", "serial:SDB"), disk("sdb", { size_bytes: 2e12 }), disk("sda", { size_bytes: 4e12 }),
    part("sda2", "serial:SDA", { status: "unmounted", mountpoint: null }), part("sda1", "serial:SDA"),
    part("crypt", "uuid:sda1", { mountpoint: "/srv" }),
    part("lost", "serial:GONE"),
    resource({ id: "pool:/mnt/media", kind: "pool", name: "/mnt/media", status: "mounted", mountpoint: "/mnt/media" }),
  ]);
  assert.deepEqual(tree.disks.map((d) => d.disk.name), ["sda", "sdb"]);
  assert.deepEqual(tree.disks[0].filesystems.map((f) => [f.resource.name, f.depth]), [["sda1", 0], ["crypt", 1], ["sda2", 0]]);
  assert.equal(tree.disks[0].mounted, 2);
  assert.deepEqual(tree.disks[0].status, { label: "In use", tone: "success" }, "a partitioned disk the observer calls unmounted is in use");
  assert.deepEqual(tree.orphans.map((f) => f.resource.name), ["lost"]);
  assert.equal(tree.pools.length, 1);
  const idle = storageTree([disk("sdc"), part("sdc1", "serial:SDC", { status: "unmounted", mountpoint: null })]).disks[0];
  assert.equal(idle.status.label, "Not mounted");
  assert.equal(storageTree([disk("sdd")]).disks[0].status.label, "No filesystem");
  assert.equal(storageTree([disk("sde", { status: "ambiguous" })]).disks[0].status.label, "Can't identify");
  const whole = storageTree([disk("sdf", { id: "uuid:whole", filesystem: "ext4", mountpoint: "/data", status: "mounted" })]).disks[0];
  assert.equal(whole.filesystems.length, 1, "a filesystem on the whole disk is listed");
  assert.deepEqual(filesystemStatus(part("x", null, { status: "swap" })), { label: "Swap", tone: "info" });
  assert.equal(filesystemStatus(part("x", null, { status: "missing" })).tone, "danger");
});

test("capacity sums known disk sizes only; root use comes from host metrics", () => {
  const tree = storageTree([disk("sda", { size_bytes: 4e12 }), disk("sdb", { size_bytes: null }), part("root", "serial:SDA", { mountpoint: "/" }), part("data", "serial:SDB", { status: "unmounted", mountpoint: null })]);
  const summary = storageSummary(tree, { disk_percent: 78.9 });
  assert.equal(summary.capacity, 4e12); assert.equal(summary.sizeUnknown, 1);
  assert.equal(summary.mounted, 1); assert.equal(summary.notMounted, 1); assert.equal(summary.filesystems, 2);
  assert.equal(summary.rootPercent, 78.9);
  assert.equal(tree.disks[0].filesystems[0].root, true);
  const none = storageSummary(storageTree([disk("sda")]), null);
  assert.equal(none.capacity, null); assert.equal(none.rootPercent, null);
  assert.equal(serialOf(disk("sda")), "SDA"); assert.equal(uuidOf(part("abc", null)), "abc"); assert.equal(serialOf(part("abc", null)), null);
});

// ---- Storage inventory --------------------------------------------------------------

const inventory = () => ({
  inventory: {
    host_root_mount_id: 1, topology_digest: "1".repeat(64), mounts_digest: "2".repeat(64), fstab_digest: "3".repeat(64),
    fstab_entries: [
      { filesystem_uuid: "sda1", device: null, mountpoint: "/mnt/sda1", managed: false },
      { filesystem_uuid: null, device: { major: 8, minor: 17 }, mountpoint: "/mnt/backup", managed: true },
      { filesystem_uuid: "gone", device: null, mountpoint: "/mnt/old", managed: false },
    ],
    devices: [
      { device: { major: 8, minor: 1 }, filesystem_uuid: "sda1", filesystem: "ext4", serial: "WD-1", boot_backing: false, in_use_as_swap: false,
        mounts: [{ mountpoint: "/mnt/sda1", mount_id: 30, filesystem_root: "/", filesystem: "ext4", writable: true }] },
      { device: { major: 8, minor: 17 }, filesystem_uuid: "sdb1", filesystem: "ext4", serial: null, boot_backing: true, mounts: [] },
      { device: { major: 8, minor: 33 }, filesystem_uuid: "sdc1", filesystem: "ext4", serial: "S-3", boot_backing: false, mounts: [] },
    ],
  },
  digest: "4".repeat(64),
});

test("storage inventory decoding is strict", () => {
  assert.equal(decodeStorageInventory(inventory()).inventory.devices.length, 3);
  for (const change of [
    (v) => { v.digest = "nope"; }, (v) => { v.inventory.host_root_mount_id = 0; }, (v) => { v.inventory.mounts_digest = "A".repeat(64); },
    (v) => { v.inventory.devices.push(structuredClone(v.inventory.devices[0])); },
    (v) => { v.inventory.devices[0].device.major = 0; }, (v) => { v.inventory.devices[0].filesystem_uuid = "bad uuid"; },
    (v) => { v.inventory.devices[0].serial = " padded "; }, (v) => { v.inventory.devices[0].serial = ""; },
    (v) => { v.inventory.devices[0].mounts[0].mountpoint = "/mnt/../etc"; }, (v) => { v.inventory.devices[0].mounts[0].mountpoint = "relative"; },
    (v) => { v.inventory.devices[0].mounts[0].mountpoint = "/mnt/x/"; }, (v) => { v.inventory.devices[0].mounts[0].writable = "yes"; },
    (v) => { v.inventory.devices = Array(129).fill(v.inventory.devices[0]); }, (v) => { v.inventory.fstab_entries[0].managed = 1; },
    (v) => { v.inventory.fstab_entries[0].mountpoint = "/mnt/\u0007"; }, (v) => { v.inventory = null; },
  ]) {
    const value = inventory(); change(value);
    assert.throws(() => decodeStorageInventory(value), StorageInventoryError, String(change));
  }
});

test("inventory facts join fstab by UUID or device number and say how a mount persists", () => {
  const facts = inventoryFacts(inventory().inventory);
  assert.deepEqual(facts.get("sda1").fstab, [{ mountpoint: "/mnt/sda1", managed: false }]);
  assert.deepEqual(facts.get("sdb1").fstab, [{ mountpoint: "/mnt/backup", managed: true }]);
  assert.equal(facts.get("sdb1").boot, true);
  assert.match(mountPersistence(true, facts.get("sda1")), /by \/etc\/fstab \(\/mnt\/sda1\)/);
  assert.match(mountPersistence(true, facts.get("sdb1")), /LimeOS's managed fstab entry/);
  assert.match(mountPersistence(true, facts.get("sdc1")), /until the next restart/);
  assert.equal(mountPersistence(false, facts.get("sdc1")), "Not in /etc/fstab");
  assert.equal(mountPersistence(true, undefined), null);
  assert.deepEqual(unmatchedFstab(inventory().inventory), [{ mountpoint: "/mnt/old", uuid: "gone", managed: false }]);
  const node = storageTree([disk("sda", { id: "device:/dev/sda" }), part("sda1", "device:/dev/sda")]).disks[0];
  assert.equal(diskSerial(node, null), null);
  assert.equal(diskSerial(node, facts), "WD-1");
});

test("inventory client: one same-origin GET, refusals told apart, bodies capped", async () => {
  const calls = [];
  const value = await readStorageInventory(new AbortController().signal, async (url, init) => { calls.push([url, init]); return Response.json(inventory()); });
  assert.equal(value.devices.length, 3);
  assert.equal(calls[0][0], "/api/v1/storage/inventory"); assert.equal(calls[0][1].method, "GET");
  assert.equal(calls[0][1].credentials, "same-origin"); assert.equal(calls[0][1].cache, "no-store");
  for (const [status, code] of [[401, "unauthenticated"], [403, "forbidden"], [503, "unavailable"]]) {
    await assert.rejects(readStorageInventory(new AbortController().signal, async () => new Response("PRIVATE", { status })), (e) => e.status === status && e.code === code && !e.message.includes("PRIVATE"));
  }
  await assert.rejects(readStorageInventory(new AbortController().signal, async () => new Response(new Uint8Array(70_000))), StorageInventoryError);
  await assert.rejects(readStorageInventory(new AbortController().signal, async () => new Response("{not json")), StorageInventoryError);
});
